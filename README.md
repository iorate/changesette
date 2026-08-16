# changesette

[![Crates.io](https://img.shields.io/crates/v/changesette.svg)](https://crates.io/crates/changesette)
[![CI](https://github.com/iorate/changesette/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/changesette/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/changesette.svg)](#license)

A version and changelog manager using the same changeset file format as [changesets](https://github.com/changesets/changesets) and shipped as a single dependency-free Rust binary. The name is changeset + the diminutive suffix -ette (as in diskette).

`changesette` reads changeset files, bumps the version in each named package's `package.json`, and generates its `CHANGELOG.md`. It works on single-package repositories and on npm / yarn / pnpm workspaces. It bumps only the packages named in changesets, does no dependency management ([Workspaces](#workspaces) covers what happens instead), and never touches lockfiles; regenerating lockfiles such as `package-lock.json` belongs to the package-manager layer.

`changesette` performs no git operations and no network access; commits, pull requests, tags, and releases belong to your workflows. The CLI feeds those workflows structured data: a machine-readable release plan (`version --output`), the workspace package list (`get-packages`), and per-version changelog sections (`get-changelog-entry`). The [example workflows](#example-workflows) build the whole release loop from these outputs — no changesets-specific action or bot required.

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

## Example workflows

### Single package (npm)

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it publishes the package to the npm registry and creates a GitHub Release (and its tag) with the changelog section as the notes. A version whose section is missing from the changelog (for example one released before adopting `changesette`) gets no GitHub Release. Replace `my-package` with the `name` declared in your `package.json`.

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
      id-token: write # npm trusted publishing (OIDC)

    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false

      - uses: iorate/changesette/setup@v3

      - id: version
        run: |
          changesette version --output "$RUNNER_TEMP/plan.json"
          if release="$(jq -e '[.releases[] | select(.type != "none")][0]' "$RUNNER_TEMP/plan.json")"; then
            version="$(jq -re '.newVersion' <<< "$release")"
            echo "title=Release v$version" >> "$GITHUB_OUTPUT"
            delim="$(openssl rand -hex 16)"
            {
              echo "body<<$delim"
              jq -re '.changelogEntry' <<< "$release"
              echo "$delim"
            } >> "$GITHUB_OUTPUT"
            npm install --package-lock-only
          else
            echo "title=Consume changesets" >> "$GITHUB_OUTPUT"
          fi

      - id: pr
        uses: peter-evans/create-pull-request@v8
        with:
          branch: changesette/release
          commit-message: ${{ steps.version.outputs.title }}
          title: ${{ steps.version.outputs.title }}
          body: ${{ steps.version.outputs.body }}
          delete-branch: true

      - if: steps.pr.outputs.pull-request-number == ''
        run: |
          version="$(jq -re .version package.json)"
          if ! npm view "my-package@$version" version > /dev/null 2>&1; then
            npm publish
          fi
          if ! gh release view "v$version" > /dev/null 2>&1; then
            if notes="$(changesette get-changelog-entry my-package "$version")"; then
              gh release create "v$version" \
                --target "$GITHUB_SHA" \
                --notes "$notes"
            fi
          fi
        env:
          GH_TOKEN: ${{ github.token }}
```

### Workspace (pnpm)

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it publishes the bumped packages to the npm registry with pnpm and creates a GitHub Release (and its tag, `<name>@<version>`) per package with the changelog section as the notes. `pnpm publish -r` publishes every workspace package whose version is not on the registry yet and skips the rest, so no per-package bookkeeping is needed; insert a build step before it if your packages need one. A package whose changelog has no section for its current version (for example a private package never named in a changeset) gets no GitHub Release. With npm instead of pnpm, there is no equivalent of `pnpm publish -r`; iterate over `changesette get-packages` and publish each package whose version is not on the registry yet.

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
      id-token: write # npm trusted publishing (OIDC)

    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false

      - uses: pnpm/action-setup@v6

      - uses: iorate/changesette/setup@v3

      - id: version
        run: |
          changesette version --output "$RUNNER_TEMP/plan.json"
          if jq -e 'any(.releases[]; .type != "none")' "$RUNNER_TEMP/plan.json" > /dev/null; then
            echo "title=Version packages" >> "$GITHUB_OUTPUT"
            delim="$(openssl rand -hex 16)"
            {
              echo "body<<$delim"
              jq -r '[.releases[] | select(.type != "none") | "## \(.name)@\(.newVersion)\n\n\(.changelogEntry)"] | join("\n\n")' "$RUNNER_TEMP/plan.json"
              echo "$delim"
            } >> "$GITHUB_OUTPUT"
            pnpm install --lockfile-only
          else
            echo "title=Consume changesets" >> "$GITHUB_OUTPUT"
          fi

      - id: pr
        uses: peter-evans/create-pull-request@v8
        with:
          branch: changesette/release
          commit-message: ${{ steps.version.outputs.title }}
          title: ${{ steps.version.outputs.title }}
          body: ${{ steps.version.outputs.body }}
          delete-branch: true

      - if: steps.pr.outputs.pull-request-number == ''
        run: |
          pnpm install --frozen-lockfile
          pnpm publish -r
          packages="$(changesette get-packages)"
          jq -c '.[]' <<< "$packages" | while read -r package; do
            name="$(jq -re .name <<< "$package")"
            version="$(jq -re .version <<< "$package")"
            if ! gh release view "$name@$version" > /dev/null 2>&1; then
              if notes="$(changesette get-changelog-entry "$name" "$version")"; then
                gh release create "$name@$version" \
                  --target "$GITHUB_SHA" \
                  --notes "$notes"
              fi
            fi
          done
        env:
          GH_TOKEN: ${{ github.token }}
```

## CLI

### `changesette init`

Creates the `.changeset/` directory with a README.md. Does nothing if the directory already exists.

### `changesette [add] [--empty] [--message <text>] [--major <pkgs>] [--minor <pkgs>] [--patch <pkgs>]`

Creates a changeset file in `.changeset/`. `--empty` creates a changeset that names no packages and conflicts with the bump flags; `--message` (short form `-m`) sets the summary; `--major`, `--minor`, and `--patch` each take a comma-separated list of package names and may be repeated. When run in a terminal, missing inputs are prompted for interactively: the affected packages and their bump types when no bump flag is given, and the summary when `--message` is not given (submitting an empty summary opens your editor for a multi-line one).

### `changesette version [--ignore <pkgs>] [--output <file>]`

Applies all pending changesets: bumps each named package's `package.json`, inserts the new section into its `CHANGELOG.md`, and deletes the consumed changesets. Each package receives the widest bump across the changesets naming it. Packages named only with the `none` type keep their version and changelog, but their changesets are still deleted, as are empty changesets. With zero changesets, nothing changes. Lockfiles are not updated; if you use npm, run `npm install --package-lock-only` afterwards.

`--ignore` takes a comma-separated list of package names and may be repeated. Each name must be a workspace member's package name. Changesets naming an ignored package are skipped: they are excluded from the release plan and left in place for a later run. A changeset naming both an ignored and a not-ignored package is an error.

`--output` (short form `-o`) suppresses stdout and writes the release plan to the given file as pretty-printed JSON, mirroring the changesets `ReleasePlan` type (an empty plan when there are zero changesets):

```json
{
  "changesets": [
    {
      "id": "changesette-01M02G4ZT0Q3D9WVK6XJ5R8YBN",
      "summary": "Add feature",
      "releases": [
        {
          "name": "my-package",
          "type": "minor"
        }
      ]
    }
  ],
  "releases": [
    {
      "name": "my-package",
      "type": "minor",
      "oldVersion": "1.2.3",
      "newVersion": "1.3.0",
      "changesets": [
        "changesette-01M02G4ZT0Q3D9WVK6XJ5R8YBN"
      ],
      "changelogEntry": "### Minor Changes\n\n- Add feature"
    }
  ]
}
```

`releases` lists every named package. `changelogEntry` is the body of the package's new changelog section, without the `## <version>` heading; a `"none"`-type release has an unchanged version and no `changelogEntry`.

### `changesette status [--verbose] [--output <file>]`

Prints the packages that `version` would bump, without changing any file. `--verbose` (short form `-v`) adds each package's new version and the changeset files naming it. `--output` (short form `-o`) writes the release plan to the given file instead of printing the list — the same file `version --output` writes. Packages named only with the `none` type appear in the JSON but not in the list.

### `changesette get-packages`

Prints the workspace packages to stdout as a single-line JSON array in package name order. Each entry has the package's `name`, its `version`, and its `dir` relative to the workspace root (`"."` when the package is the workspace root itself):

```json
[{"name":"pkg-a","version":"3.1.4","dir":"packages/a"},{"name":"pkg-b","version":"1.0.0","dir":"packages/b"}]
```

### `changesette get-changelog-entry <package> <version>`

Prints the body of the `## <version>` section of the named package's `CHANGELOG.md` — the text below that heading, without the heading itself.

## Workspaces

`changesette` works on npm / yarn / pnpm workspaces, and its changeset files are format-compatible with changesets — but `version` deliberately does not behave like `changeset version` in a workspace. The dependency management changesets performs is two separate jobs, and `changesette` does neither: **internal dependency ranges are never rewritten, and dependents of a bumped package are never bumped** (so no "Updated dependencies" changelog entries either). Only the packages explicitly named in changesets are bumped.

Ranges are a mechanical job, and the `workspace:` protocol of yarn and pnpm makes it the package manager's: in development a `workspace:` dependency always resolves to the local copy, and at publish the range is derived from the dependency's current version (`workspace:^` becomes a caret range, `workspace:*` an exact pin, and so on), so published ranges always reflect the versions the dependent was actually built against. Plain npm workspaces work too, but literal ranges like `^1.2.0` are then yours to maintain: rewrite them when the dependency moves to a new major (otherwise npm stops linking the local copy), and raise them when the dependent starts relying on newer behavior.

Dependent releases are a judgment, not bookkeeping. Already-published dependents keep working after an internal dependency's major release: their published ranges still resolve to the old, compatible versions. So release a dependent when it has changes of its own; the one other reason is a consumer who cannot run two copies of the dependency side by side (a peer dependency conflict, a shared singleton) and so needs a published range that accepts the new major. Either way, name the dependent in a changeset like any other change.

## Differences from changesets

`changesette` shares the changeset file format with changesets, but is deliberately much smaller. Coming from changesets, expect the following:

- No dependency management: dependents of a bumped package are never bumped, and dependency ranges are never rewritten (see [Workspaces](#workspaces)).
- No configuration: `.changeset/config.json` is not read, and there is nothing to configure (no `fixed` / `linked`; `ignore` exists only as the `version --ignore` flag).
- No pre-release mode (`pre.json`).
- No changed-package detection: `add` does not inspect git to suggest packages, and `status` has no `--since`.
- No changelog decoration: entries are the plain changeset summaries, without auto-generated PR / commit / author links, and there are no changelog plugins.
- No command compatibility: the CLI is not a drop-in replacement for `changeset`; only the changeset files and the release plan JSON written by `--output` are interchangeable.

## License

[MIT](LICENSE)
