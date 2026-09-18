# changesette

[![Crates.io](https://img.shields.io/crates/v/changesette.svg)](https://crates.io/crates/changesette)
[![CI](https://github.com/iorate/changesette/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/changesette/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/changesette.svg)](#license)

A version and changelog manager using the same changeset file format as [changesets](https://github.com/changesets/changesets) and shipped as a single dependency-free Rust binary. The name is changeset + the diminutive suffix -ette (as in diskette).

`changesette` reads changeset files, bumps the version in each released package's `package.json`, and generates its `CHANGELOG.md`. It works on single-package repositories and on npm / yarn / pnpm workspaces.

`changesette` performs **no git operations and no network access**; commits, pull requests, tags, and releases belong to your workflows. The CLI feeds those workflows structured data — a machine-readable release plan (`version --output`), the workspace package list (`get-packages`), and per-version changelog sections (`get-changelog-entry`) — and accepts summary rewrites (`set-summary`). The [example workflows](#example-workflows) build the whole release loop from these outputs — no changesets-specific action or bot required.

## Install

GitHub Actions (verifies the build provenance of the downloaded archive; GitHub-hosted runners are assumed):

```yaml
uses: iorate/changesette/setup@v7
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

Cargo (requires Rust 1.88+):

```sh
cargo install changesette
```

## Example workflows

### Single package (npm)

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it publishes the package to the npm registry and creates a GitHub Release (and its tag) with the changelog section as the notes. Replace `my-package` with the `name` declared in your `package.json`.

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

      - uses: iorate/changesette/setup@v7

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
                --title "v$version" \
                --notes "$notes"
            fi
          fi
        env:
          GH_TOKEN: ${{ github.token }}
```

### Workspace (pnpm)

On every push to `main`, maintains a Version PR that applies the pending changesets; merging it publishes the bumped packages to the npm registry with pnpm and creates a GitHub Release (and its tag, `<name>@<version>`) per package with the changelog section as the notes. With npm instead of pnpm, there is no equivalent of `pnpm publish -r`; iterate over `changesette get-packages` and publish each package whose version is not on the registry yet.

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

      - uses: iorate/changesette/setup@v7

      - id: version
        run: |
          plan="$(changesette version --allow-no-changesets --output -)"
          if jq -e 'any(.releases[]; .type != "none")' <<< "$plan" > /dev/null; then
            echo "title=Version packages" >> "$GITHUB_OUTPUT"
            delim="$(openssl rand -hex 16)"
            {
              echo "body<<$delim"
              jq -r '[.releases[]
                  | select(.type != "none")
                  | "## \(.name)@\(.newVersion)\n\n\(.changelogEntry)"]
                | join("\n\n")' <<< "$plan"
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
                  --title "$name@$version" \
                  --notes "$notes"
              fi
            fi
          done
        env:
          GH_TOKEN: ${{ github.token }}
```

### Adding commit, pull request, and author attributions

Prefixes each changeset summary with the short hash of the commit that added it — the same format as `@changesets/changelog-git`, the changesets default. Insert this step before the `version` step in either workflow above:

```yaml
      - run: |
          changesette status --output - | jq -c '.changesets[]' | while read -r changeset; do
            id="$(jq -re .id <<< "$changeset")"
            [[ "$id" == pre/* ]] && continue
            summary="$(jq -re .summary <<< "$changeset")"
            commit="$(gh api -X GET "repos/$GITHUB_REPOSITORY/commits" \
              -f "path=.changeset/$id.md" -F per_page=100 \
              --jq '.[-1].sha // empty')"
            if [[ -n "$commit" ]]; then
              changesette set-summary "$id" "${commit:0:7}: $summary"
            fi
          done
        env:
          GH_TOKEN: ${{ github.token }}
```

To turn the hash into a link and add the pull request and author, as `@changesets/changelog-github` does, use this step instead:

```yaml
      - run: |
          changesette status --output - | jq -c '.changesets[]' | while read -r changeset; do
            id="$(jq -re .id <<< "$changeset")"
            [[ "$id" == pre/* ]] && continue
            summary="$(jq -re .summary <<< "$changeset")"
            commit="$(gh api -X GET "repos/$GITHUB_REPOSITORY/commits" \
              -f "path=.changeset/$id.md" -F per_page=100 \
              --jq '.[-1] // empty')"
            if [[ -n "$commit" ]]; then
              commit_sha="$(jq -re .sha <<< "$commit")"
              commit_url="$(jq -re .html_url <<< "$commit")"
              prefix="[\`${commit_sha:0:7}\`]($commit_url)"
              pr="$(gh api "repos/$GITHUB_REPOSITORY/commits/$commit_sha/pulls" \
                --jq '.[0] // empty')"
              if [[ -n "$pr" ]]; then
                pr_number="$(jq -re .number <<< "$pr")"
                pr_url="$(jq -re .html_url <<< "$pr")"
                prefix="[#$pr_number]($pr_url) $prefix"
                user="$(jq -c '.user // empty' <<< "$pr")"
              else
                user="$(jq -c '.author // empty' <<< "$commit")"
              fi
              if [[ -n "$user" ]]; then
                user_login="$(jq -re .login <<< "$user")"
                user_url="$(jq -re .html_url <<< "$user")"
                prefix="$prefix Thanks [@$user_login]($user_url)!"
              fi
              changesette set-summary "$id" "$prefix - $summary"
            fi
          done
        env:
          GH_TOKEN: ${{ github.token }}
```

## CLI

Every command accepts `--log-level <error|warn|info|debug>` (default `info`) after the subcommand, setting the lowest level of the messages printed to stderr. Every command also accepts `--root <dir>`, using the given directory as the root instead of finding it from the working directory (see [Workspaces](#workspaces)); the `CHANGESETTE_ROOT` environment variable does the same.

### `changesette init`

Creates the `.changeset/` directory with a `README.md` and a `config.json` holding the default configuration. Optional: every command works without the directory, and `add` creates it on demand.

### `changesette [add] [--empty] [--open] [--message <text>] [--major <pkgs>] [--minor <pkgs>] [--patch <pkgs>]`

Creates a changeset file in `.changeset/`. `--empty` creates a changeset that names no packages; `--open` opens the created changeset in your editor; `--message` (short form `-m`) sets the summary; `--major`, `--minor`, and `--patch` each take a comma-separated list of package names. When run in a terminal, missing inputs are prompted for interactively.

### `changesette version [--ignore <pkgs>] [--snapshot [<tag>]] [--snapshot-prerelease-template <template>] [--allow-no-changesets] [--output <file>]`

Applies all pending changesets: bumps each released package's `package.json`, inserts the new section into its `CHANGELOG.md`, and deletes the consumed changesets. With zero changesets, nothing changes and the command fails; `--allow-no-changesets` (short form `-a`) makes it succeed instead. In [pre-release mode](#pre-release-mode), `version` bumps to `-<tag>.<n>` prereleases.

`--ignore` skips packages by exact name for this run.

`--snapshot` and `--snapshot-prerelease-template` create a [snapshot release](#snapshot-releases) instead, bumping to throwaway `0.0.0-<suffix>` versions.

`--output` (short form `-o`) suppresses the report and writes the release plan to the given file (`-` for stdout) as JSON, extending the changesets `ReleasePlan` type with `dir` and `changelogEntry`:

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
      "dir": ".",
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

In pre-release mode, a top-level `preState` object is included.

### `changesette status [--verbose] [--output <file>]`

Prints the packages that `version` would bump, without changing any file. `--output` (short form `-o`) writes the release plan to the given file (`-` for stdout) instead — the same JSON `version --output` writes.

### `changesette pre enter <tag>`

Enters [pre-release mode](#pre-release-mode) by writing `.changeset/pre.json` with the given tag (the `beta` of `1.1.0-beta.0`).

### `changesette pre exit`

Leaves pre-release mode by flipping `.changeset/pre.json` to the exited state, so that the next `version` bumps to final versions and deletes the file.

### `changesette get-packages [--all]`

Prints the packages managed by `version` as a JSON array:

```json
[
  {
    "dir": "packages/a",
    "name": "pkg-a",
    "version": "3.1.4",
    "private": false
  },
  {
    "dir": "packages/b",
    "name": "pkg-b",
    "version": "1.0.0",
    "private": false
  }
]
```

With `--all`, skipped packages are included too; `name` and `version` are omitted for a package without them.

### `changesette get-changelog-entry <package> <version>`

Prints the body of the `## <version>` section of the named package's `CHANGELOG.md`.

### `changesette set-summary <id> <summary>`

Rewrites the summary of the changeset `.changeset/<id>.md`, leaving its releases unchanged.

## Configuration

`.changeset/config.json` is read when present and is format-compatible with the changesets config. Wherever a setting lists package names, glob patterns like `"@scope/*"` also work.

### `fixed`

Default: `[]`.

Groups (arrays) of names of packages that are always released together at the same version.

### `linked`

Default: `[]`.

Groups (arrays) of names of packages whose versions are aligned whenever they are released together.

### `privatePackages`

Default: `{ "version": false }`.

Whether private packages (`"private": true` in `package.json`) are versioned. Set `{ "version": true }` (or the shorthand `true`) to version them; by default they are skipped.

### `updateInternalDependencies`

Default: `"patch"`.

The smallest bump of a dependency that updates the ranges on it in the other packages. With `"minor"`, a patch bump leaves a range that still includes the new version alone.

### `ignore`

Default: `[]`.

Names of packages to skip.

### `bumpVersionsWithWorkspaceProtocolOnly`

Default: `false`.

Whether only `workspace:` ranges count as internal dependencies. By default any range that resolves to the workspace package does.

### `snapshot`

Default: `{ "useCalculatedVersion": false }`.

Options for [snapshot releases](#snapshot-releases). `useCalculatedVersion` bases snapshot versions on the calculated next version instead of `0.0.0`; `prereleaseTemplate`, unset by default, sets the suffix template (`--snapshot-prerelease-template` overrides it).

### `changesette`

Default: `{ "manageInternalDependencies": true }`.

Settings specific to `changesette`. `manageInternalDependencies` set to `false` makes `version` leave the dependency ranges and the dependents of a released package alone; `packages`, unset by default, lists the directories of the workspace packages as literal `/`-separated paths relative to the root (`.` for the root itself), used instead of discovering them (see [Workspaces](#workspaces)).

## Workspaces

`changesette` works on npm / yarn / pnpm workspaces and manages the dependencies between their packages. When a workspace package is released, the packages that depend on it are updated as well: a dependent whose range on it no longer includes the new version is bumped as a patch, and the range on the released package in every dependent's `package.json` is raised to the new version (`workspace:*`, `workspace:^`, and `workspace:~` ranges are left alone). Each dependent that is released, whether by its own changesets or by that patch bump, lists the new version under "Updated dependencies" in its changelog. [`updateInternalDependencies`](#updateinternaldependencies), [`bumpVersionsWithWorkspaceProtocolOnly`](#bumpversionswithworkspaceprotocolonly), and [`changesette.manageInternalDependencies`](#changesette-1) adjust this.

`changesette` resolves the workspace by rules of its own, which can differ from the package manager's. When they do, override it: [`--root`](#cli) sets the workspace root, and [`changesette.packages`](#changesette-1) lists the package directories directly.

## Pre-release mode

Pre-release mode turns the pending changesets into `1.3.0-beta.0`, `1.3.0-beta.1`, … before the final `1.3.0`. `pre enter` and `pre exit` maintain `.changeset/pre.json`, in the same format changesets uses:

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

While in pre mode, `version` moves the changesets it consumes to `.changeset/pre/` instead of deleting them. Once pre mode is exited, the next `version` plans the parked changesets together with the new ones into the final version and deletes both the consumed changesets and `pre.json`. A package left on a prerelease version that no changeset names is bumped to its final version too.

## Snapshot releases

`version --snapshot [<tag>]` creates a snapshot release for publishing work-in-progress changes under a temporary dist-tag. Every package that would be bumped gets a throwaway `0.0.0-<suffix>` version, so that no ordinary semver range resolves to a snapshot; the [`snapshot.useCalculatedVersion`](#snapshot) setting bases snapshot versions on the calculated next version instead of `0.0.0`. Changesets are consumed and `CHANGELOG.md` sections written as usual, so run `version --snapshot` on a throwaway working tree:

```sh
changesette version --snapshot canary         # 1.2.3 -> 0.0.0-canary-20260822123456
npm publish --tag canary
```

By default the suffix is `<tag>-<datetime>`, or just `<datetime>` when no tag is given. It can be customized with `--snapshot-prerelease-template`, or the [`snapshot.prereleaseTemplate`](#snapshot) setting, using the `{tag}`, `{timestamp}`, and `{datetime}` placeholders; changesets' `{commit}` and `{commit-short}` are not supported.

## Differences from changesets

`changesette` shares the changeset file format with changesets, but is deliberately much smaller. Coming from changesets, expect the following:

- No git operations: nothing is committed or tagged, and changelog sections are built from the plain changeset summaries, without auto-generated commit / pull request / author attributions (see [Adding commit, pull request, and author attributions](#adding-commit-pull-request-and-author-attributions)).
- No npm publishing: publishing belongs to your workflows (see [Example workflows](#example-workflows)).

## License

[MIT](LICENSE)
