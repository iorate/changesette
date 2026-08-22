---
changesette: minor
---

The skip judgment for private, ignored, and versionless packages uses the manifest data gathered during workspace discovery, counting `private` only when it is boolean `true`; `add` and `get-packages` no longer read every member's `package.json`, so a member with an invalid version breaks neither of them, and strict `package.json` validation applies only to the packages being bumped.
