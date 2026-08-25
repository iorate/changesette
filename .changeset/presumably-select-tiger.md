---
changesette: major
---

Workspace glob patterns are now matched against manifest file paths with a single built-in dialect: `x/**` includes `x` itself and `!x/**` excludes it, negations are order-independent, every leading `!` toggles the polarity, braces such as `{a,b}` are expanded, explicitly dotted segments such as `.github/*` match, a wildcard-matched symlinked directory is no longer a member (a symlink behind a literal pattern segment still is), and manifests and pnpm-workspace.yaml files starting with a UTF-8 BOM are accepted. Empty patterns (`""`, `"!"`), a `/` inside braces or a character class, and absolute, Windows drive-prefixed (`C:/x`), or `../` patterns are errors instead of being ignored, skipped, or followed.
