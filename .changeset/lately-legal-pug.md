---
changesette: minor
---

Add snapshot releases: `version --snapshot [<tag>]` bumps every released package to a throwaway `0.0.0-<suffix>` version, with the suffix rendered from `--snapshot-prerelease-template` or the new `snapshot` config setting (`useCalculatedVersion`, `prereleaseTemplate`) using the `{tag}`, `{timestamp}`, and `{datetime}` placeholders.
