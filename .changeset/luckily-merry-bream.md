---
changesette: minor
---

The skip judgment for private, ignored, and versionless packages now reads the manifest data gathered during workspace discovery, so a member with an invalid version or a non-boolean private no longer fails every command; private counts only when it is boolean true, and strict package.json validation applies only to the packages being bumped.
