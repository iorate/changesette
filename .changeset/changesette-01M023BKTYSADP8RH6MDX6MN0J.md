---
changesette: major
---

Add npm and pnpm workspace support.

- The workspace root is discovered by walking up from the working directory: the first ancestor with a `pnpm-workspace.yaml` declaring a `packages` list or a `package.json` with a `workspaces` array wins, and without one the nearest `package.json` acts as a single-package workspace. `init` creates the `.changeset` directory at the root.
- Changesets may name multiple packages, and the `none` bump type, empty changesets, and changesets with an empty summary are accepted, as in upstream changesets.
- Dependencies are never bumped automatically; every bump must be named explicitly in a changeset.
- Pre-release versions now bump as node-semver's `inc` does: 2.0.0-beta.1 graduates to 2.0.0 instead of being incremented past it.
- Breaking: `add` takes `--major`, `--minor`, and `--patch` package list flags, plus `--empty`, instead of `-b, --bump`.
- Breaking: `version` prints a JSON release plan to stdout.
- Breaking: `changelog` is renamed to `get-changelog-entry <package> <version>`.
- Breaking: `current` is replaced by `get-packages`, which prints the workspace packages as JSON.
- Commands no longer panic when a consumer closes stdout early; a broken pipe ends the output quietly.
