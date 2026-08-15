---
changesette: patch
---

Match upstream changesets in empty-summary handling: changesets whose summary is empty are accepted and render an empty changelog bullet, add accepts an empty --message, and the summary editor opens with the upstream template, drops lines starting with #, and prompts again when the result is empty.
