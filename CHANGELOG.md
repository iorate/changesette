# changesette

## 4.0.0

### Major Changes

- `version` now fails when there are no unreleased changesets, matching `changeset version`; the new `--allow-no-changesets` (`-a`) flag restores the previous success, and exiting pre-release mode is exempt.

- `init`, `add`, `pre`, and `version` now write their status messages to stderr, leaving stdout for machine-readable output.

### Minor Changes

- Add pre-release mode: `pre enter <tag>` and `pre exit` manage `.changeset/pre.json`, and `version` bumps to `-<tag>.<n>` prerelease versions while in pre mode.

- Accept `-` as the `--output` value of `status` and `version` to write the release plan to stdout.

### Patch Changes

- The status command no longer reads CHANGELOG.md files, so an unreadable changelog no longer makes it fail.

- Name new changeset files with three-word petnames instead of ULIDs.

## 3.0.0

### Major Changes

- Add npm and pnpm workspace support.

  - The workspace root is discovered by walking up from the working directory: the first ancestor with a `pnpm-workspace.yaml` declaring a `packages` list or a `package.json` with a `workspaces` array wins, and without one the nearest `package.json` acts as a single-package workspace. `init` creates the `.changeset` directory at the root.
  - Changesets may name multiple packages, and the `none` bump type, empty changesets, and changesets with an empty summary are accepted, as in upstream changesets.
  - Dependencies are never bumped automatically; every bump must be named explicitly in a changeset.
  - Pre-release versions now bump as node-semver's `inc` does: 2.0.0-beta.1 graduates to 2.0.0 instead of being incremented past it.
  - `version --ignore <pkgs>` skips the named packages: changesets naming them are excluded from the release plan and left in place for a later run. Names must match workspace members exactly, and a changeset naming both an ignored and a not-ignored package is an error.
  - Breaking: `add` takes `--major`, `--minor`, and `--patch` package list flags, plus `--empty`, instead of `-b, --bump`.
  - Breaking: `add` no longer prints the bare path of the created changeset. Human-readable command output is informational and may change between releases; scripts should rely on `--output`, `get-packages`, and `get-changelog-entry` instead.
  - Breaking: `version` no longer prints the bare new version; `--output <file>` suppresses stdout and writes the release plan to a file as pretty-printed JSON.
  - Breaking: `version --dry-run` is replaced by a new `status` command: it prints the packages to be bumped, `--verbose` adds the new versions and the changeset files, and `--output <file>` writes the release plan to a file as pretty-printed JSON.
  - Breaking: `changelog` is renamed to `get-changelog-entry <package> <version>`.
  - Breaking: `current` is replaced by `get-packages`, which prints the workspace packages as JSON.
  - Commands no longer panic when a consumer closes stdout early; a broken pipe ends the output quietly.

## 2.0.1

### Patch Changes

- Update the setup action references in the README from @v1 to @v2, and remove the incorrect claim that lockfile handling differs from changesets.

## 2.0.0

### Major Changes

- The version command no longer updates package-lock.json; it now writes only package.json and CHANGELOG.md. If you use npm, run `npm install --package-lock-only` after `changesette version` to sync the lockfile.

## 1.0.1

### Patch Changes

- Add crates.io keywords and categories to the package metadata.

## 1.0.0

### Major Changes

- Declare changesette stable. The CLI contract (commands, flags, stdout/stderr, and exit codes) and the changeset file handling are now covered by semver.

### Minor Changes

- Add `-b` as the short form of `--bump`.

### Patch Changes

- Match the upstream changeset file discovery: symlinked changesets are now read, and a directory with a changeset-like name is now an error.

## 0.1.1

### Patch Changes

- Document all installation methods in the README.

## 0.1.0

### Minor Changes

- Initial release
