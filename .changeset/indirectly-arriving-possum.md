---
changesette: minor
---

Filesystem errors swallowed during workspace discovery — an unreadable directory or manifest, a directory entry that cannot be read, or an unresolvable symlink — are now reported as warnings instead of silently dropping the affected candidates, as is a directory entry whose name is not valid UTF-8; discovery still skips over them without aborting, and a plainly missing file stays silent.
