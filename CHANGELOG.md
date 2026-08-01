# changesette

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
