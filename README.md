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
uses: iorate/changesette/setup@v4
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

      - uses: iorate/changesette/setup@v4

      - id: version
        run: |
          plan="$(changesette version --allow-no-changesets --output -)"
          if release="$(jq -e '[.releases[] | select(.type != "none")][0]' <<< "$plan")"; then
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

      - uses: iorate/changesette/setup@v4

      - id: version
        run: |
          plan="$(changesette version --allow-no-changesets --output -)"
          if jq -e 'any(.releases[]; .type != "none")' <<< "$plan" > /dev/null; then
            echo "title=Version packages" >> "$GITHUB_OUTPUT"
            delim="$(openssl rand -hex 16)"
            {
              echo "body<<$delim"
              jq -r '[.releases[] | select(.type != "none") | "## \(.name)@\(.newVersion)\n\n\(.changelogEntry)"] | join("\n\n")' <<< "$plan"
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

Creates the `.changeset/` directory with a README.md and a config.json holding the default configuration. Creates whichever of them are missing; does nothing if all of them already exist. Running `init` is optional: every command works without the directory, and `add` creates it on demand.

### `changesette [add] [--empty] [--message <text>] [--major <pkgs>] [--minor <pkgs>] [--patch <pkgs>]`

Creates a changeset file in `.changeset/`, creating the directory if needed. `--empty` creates a changeset that names no packages and conflicts with the bump flags; `--message` (short form `-m`) sets the summary; `--major`, `--minor`, and `--patch` each take a comma-separated list of package names and may be repeated. Only the packages `version` manages can be named: a [skipped](#configuration) package is rejected in the bump flags and not offered by the prompts. When run in a terminal, missing inputs are prompted for interactively: the affected packages and their bump types when no bump flag is given, and the summary when `--message` is not given (submitting an empty summary opens your editor for a multi-line one).

### `changesette version [--ignore <pkgs>] [--allow-no-changesets] [--output <file>]`

Applies all pending changesets: bumps each named package's `package.json`, inserts the new section into its `CHANGELOG.md`, and deletes the consumed changesets. Each package receives the widest bump across the changesets naming it. Packages named only with the `none` type keep their version and changelog, but their changesets are still deleted, as are empty changesets. With zero changesets, nothing changes and the command fails; `--allow-no-changesets` (short form `-a`) makes it succeed instead, and exiting pre-release mode always succeeds. Lockfiles are not updated; if you use npm, run `npm install --package-lock-only` afterwards.

In pre-release mode, `version` bumps to `-<tag>.<n>` prereleases and moves the consumed changesets to `.changeset/pre/` instead of deleting them; see [Pre-release mode](#pre-release-mode).

A package is skipped when the [configuration](#configuration)'s `ignore` matches it or `--ignore` names it, when it is a private package not versioned by the configuration, or when its package.json has no `version` field. A changeset naming only skipped packages is excluded from the release plan and left in place for a later run; a changeset mixing skipped and not skipped packages is an error. `--ignore` takes a comma-separated list of package names — exact names, not glob patterns — and may be repeated, and each name must be a workspace member's package name; it cannot be used while the configuration's `ignore` matches any package.

`--output` (short form `-o`) suppresses the report and writes the release plan to the given file (`-` for stdout) as pretty-printed JSON, extending the changesets `ReleasePlan` type with `changelogEntry` (with `--allow-no-changesets`, an empty plan when there are zero changesets):

```json
{
  "changesets": [
    {
      "id": "lovely-notable-rooster",
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
        "lovely-notable-rooster"
      ],
      "changelogEntry": "### Minor Changes\n\n- Add feature"
    }
  ]
}
```

`releases` lists every named package. `changelogEntry` is the body of the package's new changelog section, without the `## <version>` heading; a `"none"`-type release has an unchanged version and no `changelogEntry`. In pre-release mode, a top-level `"preState"` object (`{ "mode", "tag" }`) is included, and the ids of the changesets in `.changeset/pre/` carry a `pre/` prefix.

### `changesette status [--verbose] [--output <file>]`

Prints the packages that `version` would bump, without changing any file. `--verbose` (short form `-v`) adds each package's new version and the changeset files naming it. `--output` (short form `-o`) writes the release plan to the given file (`-` for stdout) instead of printing the list — the same JSON `version --output` writes. Packages named only with the `none` type appear in the JSON but not in the list.

### `changesette pre enter <tag>`

Enters pre-release mode by writing `.changeset/pre.json` with the given tag (the `beta` of `1.1.0-beta.0`), creating `.changeset/` if needed. The tag must be a valid semver pre-release identifier sequence, such as `beta`, `rc-1`, or `beta.2`. It is an error to already be in pre mode; a `pre.json` left in the exited state is rewritten in place.

### `changesette pre exit`

Leaves pre-release mode by flipping `.changeset/pre.json` to the exited state, so that the next `version` bumps to final versions and deletes the file. It is an error to have no `pre.json`; running it twice is harmless.

### `changesette get-packages [--all]`

Prints the packages managed by `version` — the workspace members it does not [skip](#configuration) — to stdout as a single-line JSON array in package name order. Each entry has the package's `name`, its `version`, a boolean `private`, and its `dir` relative to the workspace root (`"."` when the package is the workspace root itself):

```json
[{"name":"pkg-a","version":"3.1.4","private":false,"dir":"packages/a"},{"name":"pkg-b","version":"1.0.0","private":true,"dir":"packages/b"}]
```

With `--all`, every workspace member is printed instead; only then may `version` be omitted, when the package.json has no version field.

### `changesette get-changelog-entry <package> <version>`

Prints the body of the `## <version>` section of the named package's `CHANGELOG.md` — the text below that heading, without the heading itself.

## Pre-release mode

Pre-release mode publishes `1.3.0-beta.0`, `1.3.0-beta.1`, … from the pending changesets before releasing `1.3.0`. `pre enter` and `pre exit` maintain `.changeset/pre.json`, in the same format changesets uses:

```sh
changesette pre enter beta                    # write .changeset/pre.json
changesette version                           # 1.2.3 -> 1.3.0-beta.0
npm publish --tag beta

changesette add --patch my-package -m "Fix bug"
changesette version                           # 1.3.0-beta.0 -> 1.3.0-beta.1
npm publish --tag beta

changesette pre exit                          # flip pre.json to the exited state
changesette version                           # 1.3.0-beta.1 -> 1.3.0, deletes pre.json
npm publish
```

While in pre mode, `version` moves the changesets it consumes to `.changeset/pre/` instead of deleting them. Once pre mode is exited, the next `version` plans the parked changesets together with the new ones into the final version and deletes both the consumed changesets and `pre.json`. A package left on a prerelease version that no changeset names is given a patch bump too, which amounts to dropping its `-<tag>.<n>` suffix; a [skipped](#configuration) package is exempt.

Choosing the npm dist-tag is up to you, as `changesette` never publishes: pass `--tag <tag>` while pre-releasing so that `latest` keeps pointing at the stable version.

## Configuration

`.changeset/config.json` is read when present and is format-compatible with the changesets config; a missing file means the defaults, and unknown keys are ignored. The supported subset:

- `ignore` (default `[]`): glob patterns for package names to skip. The patterns are evaluated in order against the workspace members' names: a package is ignored once a pattern matches it, and un-ignored when a later `!`-prefixed pattern matches it, so `["pkg-*", "!pkg-b"]` ignores every `pkg-*` package except `pkg-b`. A pattern matching no package is not an error. An ignored package is skipped like a private one: `add` does not offer it, `version` leaves its changesets in place, and `get-packages` omits it. While `ignore` matches any package, the `version --ignore` flag cannot be used.
- `privatePackages` (default `{"version": false}`): whether private packages (`"private": true` in package.json) are versioned. By default they are skipped: `add` does not offer them, `version` leaves their changesets in place, and `get-packages` omits them. Set `{"privatePackages": {"version": true}}` (or the shorthand `"privatePackages": true`) to version private packages. In particular, a single-package repository whose package.json is private — which changesette v4 versioned without any configuration — needs this setting since v5.

Packages whose package.json has no `version` field are always skipped, independent of the configuration.

## Workspaces

`changesette` works on npm / yarn / pnpm workspaces, and its changeset files are format-compatible with changesets — but `version` deliberately does not behave like `changeset version` in a workspace. The dependency management changesets performs is two separate jobs, and `changesette` does neither: **internal dependency ranges are never rewritten, and dependents of a bumped package are never bumped** (so no "Updated dependencies" changelog entries either). Only the packages explicitly named in changesets are bumped.

Workspace members are discovered from the `workspaces` globs in the root package.json (npm / yarn) or the `packages` list in pnpm-workspace.yaml, using the [wax](https://github.com/olson-sean-k/wax) glob engine, the one pnpm 12 uses: wildcards like `*` and `**` also match dot directories (`.tools`, `.github`, …), brace patterns like `packages/{a,b}` are supported, symbolic links are not followed, and `node_modules` and `bower_components` trees are always excluded. I/O errors during the scan, such as a permission denial inside a matched directory, are reported instead of silently ignored.

Ranges are a mechanical job, and the `workspace:` protocol of yarn and pnpm makes it the package manager's: in development a `workspace:` dependency always resolves to the local copy, and at publish the range is derived from the dependency's current version (`workspace:^` becomes a caret range, `workspace:*` an exact pin, and so on), so published ranges always reflect the versions the dependent was actually built against. Plain npm workspaces work too, but literal ranges like `^1.2.0` are then yours to maintain: rewrite them when the dependency moves to a new major (otherwise npm stops linking the local copy), and raise them when the dependent starts relying on newer behavior.

Dependent releases are a judgment, not bookkeeping. Already-published dependents keep working after an internal dependency's major release: their published ranges still resolve to the old, compatible versions. So release a dependent when it has changes of its own; the one other reason is a consumer who cannot run two copies of the dependency side by side (a peer dependency conflict, a shared singleton) and so needs a published range that accepts the new major. Either way, name the dependent in a changeset like any other change.

## Differences from changesets

`changesette` shares the changeset file format with changesets, but is deliberately much smaller. Coming from changesets, expect the following:

- No dependency management: dependents of a bumped package are never bumped, and dependency ranges are never rewritten (see [Workspaces](#workspaces)).
- Minimal configuration: `.changeset/config.json` is read, but only `ignore` and `privatePackages` are supported (no `fixed` / `linked`). See [Configuration](#configuration).
- Glob patterns — the workspace member globs and the config `ignore` — use the [wax](https://github.com/olson-sean-k/wax) syntax rather than fast-glob / picomatch: common patterns such as exact names, `@scope/*`, `packages/*`, braces, and character classes behave the same, but extglobs like `!(...)` are not supported, and `{` `}` `<` `>` are metacharacters that need escaping to be literal. Wildcards also match dot directories, and symbolic links are not followed when scanning for members (see [Workspaces](#workspaces)).
- No changed-package detection: `add` does not inspect git to suggest packages, and `status` has no `--since`.
- No changelog decoration: entries are the plain changeset summaries, without auto-generated PR / commit / author links, and there are no changelog plugins.
- No drop-in command compatibility: the implemented commands follow `changeset`'s flags and exit codes, but coverage is partial and terminal output differs; the changeset files are fully interchangeable, and the release plan JSON written by `--output` extends the changesets `ReleasePlan` type with `changelogEntry`.

## License

[MIT](LICENSE)
