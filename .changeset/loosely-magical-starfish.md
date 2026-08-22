---
changesette: patch
---

add --empty now fails when the workspace has no versionable packages, as changesets does, and add no longer creates the .changeset directory before that check passes.
