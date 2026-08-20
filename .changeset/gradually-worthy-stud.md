---
changesette: major
---

`version` now fails when there are no unreleased changesets, matching `changeset version`; the new `--allow-no-changesets` (`-a`) flag restores the previous success, and exiting pre-release mode is exempt.
