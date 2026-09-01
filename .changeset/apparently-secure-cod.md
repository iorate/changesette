---
changesette: minor
---

**Semi-breaking:** workspace discovery now follows each package manager more closely, so the workspace root and the members found for an existing repository can differ from those of the previous release.

- A `yarn.lock` marks a Yarn workspace root. The nearest `pnpm-workspace.yaml` or `yarn.lock` above the working directory is the root (within one directory the former wins). Without either, npm's own rule applies: the nearest `package.json` is the root unless the `workspaces` of an ancestor lists it as a member, in which case that ancestor is. So a `workspaces` field below one of those markers, such as in a Yarn worktree child or a leftover `{"nohoist": [...]}` in a pnpm member, no longer becomes a root of its own; a leftover `workspaces` in an npm member no longer does either, as npm re-roots to the workspace root; and a package the npm root above does not list, such as an example, is a single package, as it is for npm.
- While looking for an npm workspace root, a `package.json` above the nearest one that fails to parse is passed over with a warning, as npm reads it as `{}`; the nearest one still has to parse.
- A falsy `workspaces` field (`false`, `0`, `""`) is now passed over like `null`.
- Under Yarn, a `workspaces` field that is neither an array nor `{"packages": [...]}` is ignored with a warning, and a non-string pattern in it is skipped with a warning, as Yarn ignores both; under npm they remain errors, as npm rejects them.
- The Yarn root package is always a member candidate, and the `workspaces` field of every Yarn member is expanded in turn, as Yarn does for its worktrees.
- An empty pattern matches nothing instead of being an error, and a `/` inside a character class is accepted as a class member. A leading Windows drive prefix is now an error however it is spelled — `C:\x` and `C:*` are rejected like `C:/x`, where they used to compile and silently match nothing. A `\` remains an escape in every dialect; npm's rewriting of `\` to `/` is not reproduced.
- The directory names never entered follow the package manager: `node_modules` everywhere, plus `.git` and `.yarn` under Yarn and `bower_components` under pnpm.
- A literal segment is now compared with the directory entries like a wildcard, so on a case-insensitive filesystem an all-literal pattern such as `A/lib` no longer matches `a/lib` and a matched path is always spelled as on disk; only the `package.json` name itself is still found however it is spelled, as it is under npm and Yarn.
- A symlinked directory matched by a wildcard segment is now entered one level and listed, as one matched by a literal segment already was; `**` still never enters a symlinked directory.
- In a pnpm workspace, a matched directory holding a `package.yaml` or `package.json5` but no `package.json` is an error.
