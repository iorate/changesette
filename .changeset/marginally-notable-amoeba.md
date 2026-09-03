---
changesette: minor
---

A brace alternative in a workspace pattern may now contain a `/`, so `packages/{a,b/c}` lists `packages/a` and `packages/b/c`: braces are expanded before the pattern is split into segments, as npm, Yarn, and pnpm do. As a consequence each alternative is read on its own: `{.a,b}` now matches the dot directory `.a`, which the dot rule used to skip, and a `**` alternative such as `{**,a}` now reaches any depth, where it used to match a single directory name. A pattern whose braces expand into more than 100,000 alternatives is an error.
