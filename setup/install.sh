#!/usr/bin/env bash
set -euo pipefail

# Resolve and validate version
version=$(jq -r '.version // empty' "$GITHUB_ACTION_PATH/../package.json")
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "::error::Invalid version: \"$version\""
  exit 1
fi

# Map runner platform to release artifact parameters
case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64)   target=x86_64-unknown-linux-gnu  archive_ext=.tar.xz ;;
  Linux-ARM64) target=aarch64-unknown-linux-gnu archive_ext=.tar.xz ;;
  macOS-X64)   target=x86_64-apple-darwin       archive_ext=.tar.xz ;;
  macOS-ARM64) target=aarch64-apple-darwin      archive_ext=.tar.xz ;;
  Windows-X64) target=x86_64-pc-windows-msvc    archive_ext=.zip    ;;
  *)
    echo "::error::Unsupported platform: $RUNNER_OS $RUNNER_ARCH"
    exit 1
    ;;
esac

# Check tool cache
tool_dir="$RUNNER_TOOL_CACHE/changesette/$version/$RUNNER_ARCH"
if [[ -f "$tool_dir/changesette" || -f "$tool_dir/changesette.exe" ]]; then
  echo "changesette $version is already cached"
  echo "$tool_dir" >> "$GITHUB_PATH"
  echo "version=$version" >> "$GITHUB_OUTPUT"
  exit 0
fi

# Download archive
archive="changesette-$target$archive_ext"
base_url="https://github.com/iorate/changesette/releases/download/changesette-v$version"
echo "Downloading $base_url/$archive"
curl -fsSL "$base_url/$archive" -o "$RUNNER_TEMP/$archive"

# Verify build provenance
gh attestation verify "$RUNNER_TEMP/$archive" \
  --repo iorate/changesette \
  --signer-workflow iorate/changesette/.github/workflows/release.yml
echo "Verified the build provenance of $archive"

# Extract binary into tool cache
mkdir -p "$tool_dir"
if [[ "$archive_ext" == ".tar.xz" ]]; then
  # Tarballs nest the binary under a changesette-$target/ directory.
  tar -xJf "$RUNNER_TEMP/$archive" -C "$tool_dir" --strip-components=1 "changesette-$target/changesette"
else
  # The zip places the binary at the archive root.
  unzip -q "$RUNNER_TEMP/$archive" changesette.exe -d "$tool_dir"
fi

echo "$tool_dir" >> "$GITHUB_PATH"
echo "version=$version" >> "$GITHUB_OUTPUT"
echo "Installed changesette $version"
