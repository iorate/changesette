---
changesette: minor
---

Fail `version` and `status` when a released package depends, directly or through unreleased packages, on a skipped package that has unreleased changes, unless `--allow-unreleased-dependencies` is given.
