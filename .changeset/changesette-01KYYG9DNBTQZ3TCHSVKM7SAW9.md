---
changesette: major
---

The version command no longer updates package-lock.json; it now writes only package.json and CHANGELOG.md. If you use npm, run `npm install --package-lock-only` after `changesette version` to sync the lockfile.
