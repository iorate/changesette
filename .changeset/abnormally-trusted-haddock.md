---
changesette: major
---

Workspace package detection now uses the wax glob engine, matching pnpm 12: wildcards match dot directories, symbolic links are no longer followed, brace patterns are supported, and I/O errors during the scan are reported instead of ignored.
