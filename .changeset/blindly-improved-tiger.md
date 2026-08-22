---
changesette: minor
---

The `add` command now creates the `.changeset` directory when it is missing, `init` also creates a default `config.json` and backfills missing files, and `version` and `status` treat a missing `.changeset` directory as having no changesets. When `.changeset/config.json` exists, it is now validated as the supported subset of the changesets config format.
