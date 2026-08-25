---
changesette: major
---

A `package.json` with an array or object `workspaces` field is now a workspace root on its own: the lockfile requirement is gone, and the Yarn 1 object form is read instead of rejected. During the upward root search, a non-object `package.json` and a directory named `package.json` are now passed over instead of being errors. The `package.json` at a pnpm workspace root is now read even when no pattern matches it, and a parse failure there is an error.
