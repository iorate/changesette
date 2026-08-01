# changesette

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
