---
changesette: major
---

`version` now manages the dependencies between the packages of a workspace; `updateInternalDependencies` and `bumpVersionsWithWorkspaceProtocolOnly` in `.changeset/config.json` are honored, and `changesette.ignoreInternalDependencies: true` restores the previous behavior. The workspace now includes every package it lists, the root and packages without a `name` or `version` or with a duplicated name included; such packages are never versioned, `get-packages --all` lists them with `null` for the missing fields, and naming one in a changeset or `--ignore` is an error.
