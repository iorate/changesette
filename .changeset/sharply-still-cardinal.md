---
changesette: minor
---

Type mismatches around the workspace patterns are now tolerated: a non-list `packages` in `pnpm-workspace.yaml` or in the `workspaces` object and non-string pattern entries are skipped with a warning instead of raising an error, and only unparsable YAML or JSON remains fatal.
