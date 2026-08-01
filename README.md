# changesette

A version and changelog manager for single-package applications, using the same changeset file format as [changesets](https://github.com/changesets/changesets) and shipped as a single dependency-free Rust binary. The name is changeset + the diminutive suffix -ette (as in diskette).

`changesette` reads changeset files, bumps the version in `package.json` (and `package-lock.json` when present), and generates `CHANGELOG.md`. `changesette` performs no git operations and no network access; commits, pull requests, tags, and releases belong to your workflows.

## Setup

The setup action installs the `changesette` binary and adds it to `PATH`, verifying the build provenance of the downloaded archive. GitHub-hosted runners are assumed.

```yaml
uses: iorate/changesette/setup@v1
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
              echo "body<<$delim"
              changesette changelog "$next"
              echo "$delim"
            } >> "$GITHUB_OUTPUT"
          fi

      - uses: peter-evans/create-pull-request@v8
        with:
          branch: changesette/release
          commit-message: Release v${{ steps.version.outputs.next }}
          title: Release v${{ steps.version.outputs.next }}
          body: ${{ steps.version.outputs.body }}
          delete-branch: true

      - if: steps.version.outputs.next == ''
        run: |
          v="v$(changesette current)"
          if ! gh release view "$v" > /dev/null 2>&1; then
            gh release create "$v" \
              --target "$GITHUB_SHA" \
              --notes "$(changesette changelog "${v#v}")"
          fi
        env:
          GH_TOKEN: ${{ github.token }}
```

## CLI

### `changesette init`

Creates the `.changeset/` directory with a README.md. Does nothing if the directory already exists.

### `changesette [add] [--bump <major|minor|patch>] [--message <text>]`

Creates a changeset file in `.changeset/` and prints its path.

### `changesette version [--dry-run]`

Applies all pending changesets: bumps `package.json` (and `package-lock.json`), inserts the new section into `CHANGELOG.md`, and deletes the consumed changesets. Prints the next version, or nothing when there were no changesets and nothing was changed.

### `changesette current`

Prints the current version from `package.json`.

### `changesette changelog <version>`

Prints the `## <version>` section of `CHANGELOG.md`.

## License

[MIT](LICENSE)
