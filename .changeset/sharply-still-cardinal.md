---
changesette: major
---

A null `packages` in `pnpm-workspace.yaml` and a null `workspaces` in `package.json` are now read as absent, matching npm and pnpm; any other type mismatch — a non-mapping `pnpm-workspace.yaml`, a non-list `packages`, a `workspaces` that is neither an array nor an object carrying a `packages` list, or a non-string pattern entry — is an error, and the `workspaces` type is checked in every `package.json` the upward root search reads.
