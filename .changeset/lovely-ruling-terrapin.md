---
changesette: major
---

`version` now manages the dependencies between the packages of a workspace, as changesets does:

- A package whose `dependencies`, `peerDependencies`, or `optionalDependencies` range no longer includes the new version of another package is bumped as a patch, transitively.
- Every internal dependency range on a released package, in released and unreleased packages alike, is raised to the new version: `^1.0.0` becomes `^1.0.1`, `>=1.0.0 <2.0.0` becomes `>=1.0.1 <2.0.0`, and a range the new version leaves keeps its shape, as in `^2.0.0`. `workspace:*`, `workspace:^`, and `workspace:~` are left as they are, and a snapshot release pins the snapshot version. A package rewritten this way without being released appears in the release plan as a release of type `none`.
- A released package lists the new versions of the released packages in its `dependencies` and `peerDependencies` under `- Updated dependencies` in `### Patch Changes` of its changelog (a `workspace:` range counts even when it is left as it is; a range `updateInternalDependencies` leaves alone does not), and `status --verbose` lists them as well.
- `updateInternalDependencies` and `bumpVersionsWithWorkspaceProtocolOnly` in `.changeset/config.json` are honored, and `init` writes them with their defaults. `changesette.manageInternalDependencies: false` restores the previous behavior.
- Every release in the release plan carries `dir`, the package directory relative to the workspace root, and `name`, `oldVersion`, and `newVersion` are omitted for a package without them.
- The workspace now includes every package it lists, the root and packages without a `name` or `version` included. A package without a `name` or `version` is skipped like an ignored or private one: it is never versioned, `get-packages --all` lists it without the missing fields, and a changeset naming only skipped packages is left in place. When several packages share a name, the name resolves to the last of them in directory order, with a warning, and the others are skipped as well.
- Versions are now read as npm reads them: `v1.0.0` and `01.0.0` are accepted as `1.0.0`, and so is `1.0.0 garbage`, whereas a component larger than JavaScript's `Number.MAX_SAFE_INTEGER` is rejected.
