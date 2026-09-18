---
changesette: patch
---

A changeset with CRLF line endings is read as if it used LF, so its summary no longer carries `\r` into the release plan and the status output. `version` now writes a new changelog section with the line ending of the existing CHANGELOG.md, instead of LF with an extra blank line after the title when the file uses CRLF.
