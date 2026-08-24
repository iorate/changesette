---
changesette: major
---

Workspace member candidates are now excluded, with a warning on stderr, when their `package.json` lacks a nonempty string `name` or a valid semver `version`, or when their name is shared by another candidate; a non-string name or a duplicated name is no longer an error. Every member therefore has a unique name and a valid version: `get-packages` always reports `version`, and a changeset naming an excluded package makes `version` fail with a not-found error instead of being silently kept.
