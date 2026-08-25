---
changesette: major
---

Workspace glob patterns are now matched against manifest file paths with a single built-in dialect: `x/**` includes `x` itself and `!x/**` excludes it, negations are order-independent, braces such as `{a,b}` are expanded, explicitly dotted segments such as `.github/*` match, a wildcard-matched symlinked directory is no longer a member (a symlink behind a literal pattern segment still is), manifests and pnpm-workspace.yaml files starting with a UTF-8 BOM are accepted, and absolute or `../` patterns are errors instead of being ignored or followed.
