---
changesette: patch
---

Duplicate keys in a changeset's frontmatter, `pnpm-workspace.yaml`, or `.yarnrc.yml` are now reported as invalid YAML instead of silently keeping the last value, as changesets and pnpm do.
