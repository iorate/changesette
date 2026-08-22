---
changesette: major
---

Private packages and packages without a version field are no longer versioned by default; set privatePackages.version to true in .changeset/config.json to keep versioning private packages. The add command follows suit: skipped packages are excluded from the interactive prompt, naming one in a bump flag is an error, and add (--empty included) fails when the workspace has no versionable packages.
