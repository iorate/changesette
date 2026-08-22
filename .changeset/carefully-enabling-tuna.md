---
changesette: major
---

A `pnpm-workspace.yaml` without a `packages` key now makes its directory a workspace root whose only member is the root package, as pnpm does. Such a file was previously ignored, so discovery kept climbing and could pick an outer workspace instead.
