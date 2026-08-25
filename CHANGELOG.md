# changesette

## 6.0.0

### Major Changes

- A `package.json` with an array or object `workspaces` field is now a workspace root on its own: the lockfile requirement is gone, and the Yarn 1 object form is read instead of rejected. During the upward root search, a non-object `package.json` and a directory named `package.json` are now passed over instead of being errors. The `package.json` at a pnpm workspace root is now read even when no pattern matches it, and a parse failure there is an error.

- A `pnpm-workspace.yaml` now marks a pnpm workspace root by its presence alone, a settings-only or empty file included, and it wins over a `workspaces` field in the same directory. The pnpm root package is always a workspace member when its `package.json` qualifies, whether or not a pattern matches it, and no negation excludes it.

- Workspace member candidates are now excluded when their `package.json` lacks a nonempty string `name` or a valid semver `version`, or when their name is shared by another candidate; a non-string name or a duplicated name is no longer an error. The single-package fallback takes the same qualification: a nearest `package.json` failing it now yields a workspace with no members instead of an error. Candidates that are the same physical directory reached under different paths (through a symlink, for example) collapse into a single member instead of counting as a duplicated name. A key holding an invalid value and a duplicated name are reported with a warning on stderr, while a missing `name` or `version` key is reported only at the debug log level. Every member therefore has a unique name and a valid version: `get-packages` always reports `version`, and a changeset naming an excluded package makes `version` fail with a not-found error instead of being silently kept.

- Workspace glob patterns are now matched against manifest file paths with a single built-in dialect: `x/**` includes `x` itself and `!x/**` excludes it, negations are order-independent, every leading `!` toggles the polarity, braces such as `{a,b}` are expanded, explicitly dotted segments such as `.github/*` match, a wildcard-matched symlinked directory is no longer a member (a symlink behind a literal pattern segment still is), and manifests and pnpm-workspace.yaml files starting with a UTF-8 BOM are accepted. Empty patterns (`""`, `"!"`), a `/` inside braces or a character class, and absolute, Windows drive-prefixed (`C:/x`), or `../` patterns are errors instead of being ignored, skipped, or followed.

- A null `packages` in `pnpm-workspace.yaml` and a null `workspaces` in `package.json` are now read as absent, matching npm and pnpm; any other type mismatch — a non-mapping `pnpm-workspace.yaml`, a non-list `packages`, a `workspaces` that is neither an array nor an object carrying a `packages` list, or a non-string pattern entry — is an error, and the `workspaces` type is checked in every `package.json` the upward root search reads.

### Minor Changes

- New info messages report when `init` finds everything already initialized and when `--output` writes the release plan to a file, and `--log-level debug` now reports the discovered workspace members, the reason each package is skipped, candidates excluded by negative patterns or symlinks, and bumps raised by `fixed` / `linked` groups.

- The new `--log-level <LEVEL>` option (`error`, `warn`, `info`, or `debug`; the default is `info`) sets the lowest level of the messages printed to stderr; write the option after the subcommand. Debug messages carry a `debug:` prefix in the style of the existing `warning:` and `error:` ones.

- Filesystem errors swallowed during workspace discovery — an unreadable directory or manifest, a directory entry that cannot be read, or a symlink loop — are now reported as warnings instead of silently dropping the affected candidates, as is a directory entry whose name is not valid UTF-8; discovery still skips over them without aborting, and a plainly missing file or a dangling symlink stays silent.

### Patch Changes

- Error and log messages now render paths with `/` as the separator on Windows instead of `\`.

- The notice that `version` runs in pre mode is now printed at the info level instead of as a warning.

## 5.0.0

### Major Changes

- Private packages and packages without a `version` field are no longer versioned by default; set `privatePackages.version` to `true` in `.changeset/config.json` to keep versioning private packages. The `add` command follows suit: skipped packages are excluded from the interactive prompt, naming one in a bump flag is an error, and `add` (`--empty` included) fails when the workspace has no versionable packages.

- Packages whose `package.json` has no `version` field are now included as workspace members, and `get-packages` reports a `private` field for every package and omits `version` when the `package.json` has none.

- A `package.json` with `workspaces` is now a workspace root only when a `package-lock.json`, `yarn.lock`, `bun.lockb`, or `bun.lock` sits next to it, as changesets requires; without one, discovery keeps climbing and the manifest can only become a single package.

- The `get-packages` command now lists only the packages managed by `version` by default; pass `--all` to list every workspace member.

### Minor Changes

- The `add` command now creates the `.changeset` directory when it is missing, `init` also creates a default `config.json` and backfills missing files, and `version` and `status` treat a missing `.changeset` directory as having no changesets. When `.changeset/config.json` exists, it is now validated as the supported subset of the changesets config format.

- Add the `--open` flag to `add`: after the changeset is created, it opens the file in your editor (`VISUAL`, `EDITOR`, or `vi`) and waits for the editor to exit.

- Add snapshot releases: `version --snapshot [<tag>]` bumps every released package to a throwaway `0.0.0-<suffix>` version, with the suffix rendered from `--snapshot-prerelease-template` or the new `snapshot` config setting (`useCalculatedVersion`, `prereleaseTemplate`) using the `{tag}`, `{timestamp}`, and `{datetime}` placeholders.

- The skip judgment for private, ignored, and versionless packages uses the manifest data gathered during workspace discovery, counting `private` only when it is boolean `true`; `add` and `get-packages` no longer read every member's `package.json`, so a member with an invalid version breaks neither of them, and strict `package.json` validation applies only to the packages being bumped.

- Support the `fixed` and `linked` config settings.

- The release plan JSON written by `version --output` and `status --output` now includes changesets that name only skipped packages, matching the changesets `ReleasePlan`; their files are still left on disk and they produce no releases.

- The `ignore` option in `.changeset/config.json` is now supported with glob patterns including ordered negation, and cannot be combined with the `--ignore` CLI option.

- Add `fixed` and `linked` to the default config written by `init`.

### Patch Changes

- Recognize the title heading of a `CHANGELOG.md` that starts with a UTF-8 BOM, instead of prepending a duplicate title and leaving the BOM in the middle of the file.

- The `init` command now mentions workspaces in the generated `README.md`.

- The JSON output of `get-packages`, and of `version` and `status` with `--output -`, is pretty-printed when stdout is a terminal and single-line otherwise.

- The `--output` help for `version` and `status` no longer claims the JSON is pretty-printed, since it is single-line when stdout is not a terminal.

- Workspace patterns starting with a slash, such as `/packages/*` or `!/packages/a`, are now ignored as in changesets and pnpm instead of being matched as relative paths.

- Fail loudly when bumping a pathologically large version overflows, instead of silently wrapping around in release builds.

- The release plan written to a file by `version` and `status` with `--output` now ends with a newline.

## 4.0.1

### Patch Changes

- Update the setup action references in the README to v4.

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
