---
changesette: minor
---

A workspace pattern may now start with `..` to reach outside the root, as it does under npm, Yarn, and pnpm, including in the `workspaces` field of a Yarn member; a `..` after the first segment remains an error. `get-packages` reports such a member with a `dir` starting with `..`.
