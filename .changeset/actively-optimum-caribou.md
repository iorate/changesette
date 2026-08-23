---
changesette: major
---

A `package.json` with an array or object `workspaces` field is now a workspace root on its own: the lockfile requirement is gone, and the Yarn 1 object form is read instead of rejected. A `package.json` that cannot be parsed during the upward root search is now always an error, instead of letting discovery silently pick another root.
