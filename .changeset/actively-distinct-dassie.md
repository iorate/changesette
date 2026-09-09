---
changesette: patch
---

Ignore an invalid "workspaces" field in package.json or an invalid pnpm-workspace.yaml with a warning instead of failing. Under Yarn, a "workspaces" array containing a non-string is now ignored as a whole instead of having the non-string skipped.
