# changesette

[![Crates.io](https://img.shields.io/crates/v/changesette.svg)](https://crates.io/crates/changesette)
[![CI](https://github.com/iorate/changesette/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/changesette/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/changesette.svg)](#license)

A version and changelog manager for applications, using the same changeset file format as [changesets](https://github.com/changesets/changesets) and shipped as a single dependency-free Rust binary. The name is changeset + the diminutive suffix -ette (as in diskette).

`changesette` reads changeset files, bumps the version in each named package's `package.json`, and generates its `CHANGELOG.md`. It works on single-package repositories and on npm / pnpm workspaces. It never touches lockfiles; regenerating a lockfile such as `package-lock.json` is your package manager's job. `changesette` performs no git operations and no network access; commits, pull requests, tags, and releases belong to your workflows.

## Install

GitHub Actions (verifies the build provenance of the downloaded archive; GitHub-hosted runners are assumed):

```yaml
uses: iorate/changesette/setup@v3
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

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it creates a GitHub Release (and its tag) with the changelog section as the notes. The example is for a single-package repository; replace `my-package` with the `name` declared in your `package.json`.

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

      - uses: iorate/changesette/setup@v3

      - id: version
        run: |
          plan="$(changesette version)"
          next="$(jq -r '[.releases[] | select(.type != "none")][0].newVersion // empty' <<< "$plan")"
          if [[ -n "$next" ]]; then
            echo "title=Release v$next" >> "$GITHUB_OUTPUT"
            delim="$(openssl rand -hex 16)"
            {
              echo "changelog<<$delim"
              changesette get-changelog-entry my-package "$next"
              echo "$delim"
            } >> "$GITHUB_OUTPUT"
            npm install --package-lock-only # if you use npm
          else
            echo "title=Consume changesets" >> "$GITHUB_OUTPUT"
          fi

      - id: pr
        uses: peter-evans/create-pull-request@v8
        with:
          branch: changesette/release
          commit-message: ${{ steps.version.outputs.title }}
          title: ${{ steps.version.outputs.title }}
          body: ${{ steps.version.outputs.changelog }}
          delete-branch: true

      - if: steps.pr.outputs.pull-request-number == ''
        run: |
          version="$(changesette get-version my-package)"
          if ! gh release view "v$version" > /dev/null 2>&1; then
            gh release create "v$version" \
              --target "$GITHUB_SHA" \
              --notes "$(changesette get-changelog-entry my-package "$version")"
          fi
        env:
          GH_TOKEN: ${{ github.token }}
```

## CLI

### `changesette init`

Creates the `.changeset/` directory with a README.md. Does nothing if the directory already exists.

### `changesette [add] [--major <pkgs>] [--minor <pkgs>] [--patch <pkgs>] [--empty] [--message <text>]`

Creates a changeset file in `.changeset/` and prints its path (relative to the working directory). `--major`, `--minor`, and `--patch` each take a comma-separated list of package names and may be repeated; `--empty` creates a changeset that names no packages and conflicts with the bump flags; `--message` (short form `-m`) sets the summary. When run in a terminal, missing inputs are prompted for interactively: the affected packages and their bump types when no bump flag is given, and the summary when `--message` is not given (submitting an empty summary opens your editor for a multi-line one).

### `changesette version [--dry-run]`

Applies all pending changesets: bumps each named package's `package.json`, inserts the new section into its `CHANGELOG.md`, and deletes the consumed changesets. Prints the release plan to stdout as single-line JSON, mirroring the changesets `ReleasePlan` type:

```json
{"changesets":[{"id":"brave-lions-jump","summary":"Add feature","releases":[{"name":"my-package","type":"minor"}]}],"releases":[{"name":"my-package","type":"minor","oldVersion":"1.2.3","newVersion":"1.3.0","changesets":["brave-lions-jump"]}]}
```

- With pending bumps, `releases` lists every named package with its widest bump and new version.
- Packages named only with the `none` type appear with `"type": "none"` and an unchanged version; their files are still deleted, as are empty changesets.
- With zero changesets, prints `{"changesets":[],"releases":[]}` and changes nothing.

`--dry-run` (short form `-n`) prints exactly the same JSON without changing any files. Lockfiles are not updated; if you use npm, run `npm install --package-lock-only` afterwards.

### `changesette get-version <package>`

Prints the named package's version from its `package.json`.

### `changesette get-changelog-entry <package> <version>`

Prints the `## <version>` section of the named package's `CHANGELOG.md`.

## Workspaces

Every command resolves its workspace by walking up from the working directory. The first ancestor directory that is a workspace root wins:

- a `pnpm-workspace.yaml` with a `packages` list (pnpm), or
- with no `pnpm-workspace.yaml`, a `package.json` whose `workspaces` key is an array of globs (npm / Yarn; the Yarn 1 object form is not supported).

The workspace members are the directories matching those globs whose `package.json` has both a `name` and a `version`; the root itself is not a member. Without a workspace root, the nearest `package.json` acts as a single-package workspace. The `.changeset/` directory always lives at the resolved root.

The changeset files are format-compatible with changesets, but `version` deliberately does not behave like `changeset version` in a workspace: **dependencies are never bumped automatically**. Only the packages explicitly named in changesets are bumped; internal dependency ranges are not rewritten, and no "Updated dependencies" changelog entries are generated. This assumes internal references use the `workspace:*` protocol or ranges loose enough to keep matching.

## Differences from changesets

`changesette` shares the changeset file format with changesets, but is deliberately much smaller. Coming from changesets, expect the following:

- No dependency management: dependents of a bumped package are never bumped, and dependency ranges are never rewritten (see [Workspaces](#workspaces)).
- No configuration: `.changeset/config.json` is not read, and there is nothing to configure (no `fixed` / `linked` / `ignore`).
- No pre-release mode (`pre.json`).
- No changed-package detection: `add` does not inspect git to suggest packages.
- Changelog entries are plain summaries: no auto-generated PR / commit / author links and no changelog plugins.
- The CLI is not command-compatible with `changeset`; only the changeset files are interchangeable.

## License

[MIT](LICENSE)
