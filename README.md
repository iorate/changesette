# changesette

[![Crates.io](https://img.shields.io/crates/v/changesette.svg)](https://crates.io/crates/changesette)
[![CI](https://github.com/iorate/changesette/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/changesette/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/changesette.svg)](#license)

A version and changelog manager for single-package applications, using the same changeset file format as [changesets](https://github.com/changesets/changesets) and shipped as a single dependency-free Rust binary. The name is changeset + the diminutive suffix -ette (as in diskette).

`changesette` reads changeset files, bumps the version in `package.json` (and `package-lock.json` when present), and generates `CHANGELOG.md`. `changesette` performs no git operations and no network access; commits, pull requests, tags, and releases belong to your workflows.

## Install

GitHub Actions (verifies the build provenance of the downloaded archive; GitHub-hosted runners are assumed):

```yaml
uses: iorate/changesette/setup@v1
```

Shell script (macOS / Linux):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/iorate/changesette/releases/latest/download/changesette-installer.sh | sh
```

PowerShell (Windows):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/iorate/changesette/releases/latest/download/changesette-installer.ps1 | iex"
```

Homebrew:

```sh
brew install iorate/tap/changesette
```

npm:

```sh
npm install -g @iorate/changesette
```

Cargo (requires Rust 1.85+):

```sh
cargo install changesette
```

## Example workflow

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it creates a GitHub Release (and its tag) with the changelog section as the notes.

```yaml
name: Version

on:
  push:
    branches:
      - main

concurrency: version

jobs:
  version:
    runs-on: ubuntu-latest

    permissions:
      contents: write
      pull-requests: write

    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false

      - uses: iorate/changesette/setup@v1

      - id: version
        run: |
          next="$(changesette version)"
          echo "next=$next" >> "$GITHUB_OUTPUT"
          if [[ -n "$next" ]]; then
            delim="$(openssl rand -hex 16)"
            {
              echo "changelog<<$delim"
              changesette changelog "$next"
              echo "$delim"
            } >> "$GITHUB_OUTPUT"
          fi

      - uses: peter-evans/create-pull-request@v8
        with:
          branch: changesette/release
          commit-message: Release v${{ steps.version.outputs.next }}
          title: Release v${{ steps.version.outputs.next }}
          body: ${{ steps.version.outputs.changelog }}
          delete-branch: true

      - if: steps.version.outputs.next == ''
        run: |
          version="$(changesette current)"
          if ! gh release view "v$version" > /dev/null 2>&1; then
            gh release create "v$version" \
              --target "$GITHUB_SHA" \
              --notes "$(changesette changelog "$version")"
          fi
        env:
          GH_TOKEN: ${{ github.token }}
```

## CLI

### `changesette init`

Creates the `.changeset/` directory with a README.md. Does nothing if the directory already exists.

### `changesette [add] [--bump <major|minor|patch>] [--message <text>]`

Creates a changeset file in `.changeset/` and prints its path. `--bump` and `--message` have the short forms `-b` and `-m`. When run in a terminal, missing flags are prompted for interactively; submitting an empty summary opens your editor for a multi-line one.

### `changesette version [--dry-run]`

Applies all pending changesets: bumps `package.json` (and `package-lock.json`), inserts the new section into `CHANGELOG.md`, and deletes the consumed changesets. Prints the next version, or nothing when there were no changesets and nothing was changed. `--dry-run` (short form `-n`) prints the plan to stderr without changing any files.

### `changesette current`

Prints the current version from `package.json`.

### `changesette changelog <version>`

Prints the `## <version>` section of `CHANGELOG.md`.

## Differences from changesets

`changesette` shares the changeset file format with changesets, but is deliberately much smaller. Coming from changesets, expect the following:

- Single package only: no monorepo / workspace support.
- No configuration: `.changeset/config.json` is not read, and there is nothing to configure.
- No pre-release mode (`pre.json`).
- No `none` bump type and no empty changesets; both are errors.
- Changelog entries are plain summaries: no auto-generated PR / commit / author links and no changelog plugins.
- The only lockfile synced is npm's `package-lock.json`; yarn and pnpm lockfiles do not record the package's own version, so they need no syncing.
- The CLI is not command-compatible with `changeset`; only the changeset files are interchangeable.

## License

[MIT](LICENSE)
