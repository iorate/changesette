---
changesette: minor
---

Read the `_` key in a changeset's front matter as metadata instead of a package name, as changesets 3.1 does. `status --output` and `version --output` report it as `metadata` on each changeset, and `set-summary` keeps it. The value must be a mapping that has a JSON form.
