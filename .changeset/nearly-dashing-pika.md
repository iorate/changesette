---
changesette: minor
---

A workspace pattern may now start with `..` to reach outside the root, as it does under npm, Yarn, and pnpm, including in the `workspaces` field of a Yarn member; a `..` after the first segment remains an error. `get-packages` reports such a member with a `dir` starting with `..`. The `dir` is always the path relative to the root: a member found through the `workspaces` field of a Yarn member is reported from the root rather than from the declaring member, and a pattern that climbs back into the root, such as `../root/packages/*`, lists its members with the direct spelling, `packages/a`.
