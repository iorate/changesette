---
changesette: major
---

A `pnpm-workspace.yaml` now marks a pnpm workspace root by its presence alone, a settings-only or empty file included, and it wins over a `workspaces` field in the same directory. The pnpm root package is always a workspace member when its `package.json` qualifies, whether or not a pattern matches it, and no negation excludes it.
