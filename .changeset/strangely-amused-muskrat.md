---
changesette: minor
---

**Semi-breaking:** the root package is now always a workspace member candidate under npm, as it already was under Yarn and pnpm, so an npm root whose `package.json` has a `name` and a valid `version` becomes a member, and no negative pattern excludes it; a private root is still skipped unless `privatePackages.version` is set, and `ignore` excludes it.
