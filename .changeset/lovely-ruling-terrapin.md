---
changesette: major
---

`version` now manages the dependencies between the packages of a workspace, as changesets does:

- A package whose `dependencies`, `peerDependencies`, or `optionalDependencies` range no longer includes the new version of another package is bumped as a patch, transitively.
- Every internal dependency range on a released package, in released and unreleased packages alike, is raised to the new version: `^1.0.0` becomes `^1.0.1`, `>=1.0.0 <2.0.0` becomes `>=1.0.1 <2.0.0`, and a range the new version leaves keeps its shape, as in `^2.0.0`. `workspace:*`, `workspace:^`, and `workspace:~` are left as they are, and a snapshot release pins the snapshot version. A package rewritten this way without being released appears in the release plan as a release of type `none`.
- A released package lists the new versions of the released packages in its `dependencies` and `peerDependencies` under `- Updated dependencies` in `### Patch Changes` of its changelog (a `workspace:` range counts even when it is left as it is; a range `updateInternalDependencies` leaves alone does not), and `status --verbose` lists them as well.
- `updateInternalDependencies` and `bumpVersionsWithWorkspaceProtocolOnly` in `.changeset/config.json` are honored, and `init` writes them with their defaults. `changesette.manageInternalDependencies: false` restores the previous behavior.
- Every release in the release plan carries `dir`, the package directory relative to the workspace root, and `name`, `oldVersion`, and `newVersion` are omitted for a package without them.
- The workspace now includes every package it lists, the root and packages without a `name` or `version` or with a duplicated name included. Such packages are never versioned, `get-packages --all` lists them with `null` for the missing fields, and naming one in a changeset or `--ignore` is an error.
- Versions are now read as npm reads them: `v1.0.0` and `01.0.0` are accepted as `1.0.0`, and so is `1.0.0 garbage`, whereas a component larger than JavaScript's `Number.MAX_SAFE_INTEGER` is rejected.
