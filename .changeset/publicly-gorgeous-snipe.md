---
changesette: major
---

A `package.json` with `workspaces` is now a workspace root only when a `package-lock.json`, `yarn.lock`, `bun.lockb`, or `bun.lock` sits next to it, as changesets requires; without one, discovery keeps climbing and the manifest can only become a single package.
