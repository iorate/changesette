---
changesette: minor
---

A directory without a `package.json` is now a workspace root with no members instead of an error, so `init` can run before the `package.json` exists. Such a directory becomes the root when `--root` names it, or when it is the working directory and no `package.json`, `pnpm-workspace.yaml`, or `yarn.lock` is found in it or any parent.
