---
changesette: patch
---

The setup action no longer compares the `--version` output of the downloaded binary with the expected version. It only checks that the binary runs, since the version is already fixed by the release tag and the build provenance.
