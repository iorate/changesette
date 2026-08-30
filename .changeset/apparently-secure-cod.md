---
changesette: minor
---

Workspace discovery now follows each package manager more closely.

- A `yarn.lock` marks a Yarn workspace root. The nearest marker wins, and within one directory a `pnpm-workspace.yaml` beats a `yarn.lock`, which beats a `workspaces` field.
- The Yarn root package is always a member candidate, and the `workspaces` field of every Yarn member is expanded in turn, as Yarn does for its worktrees.
- The directory names never entered follow the package manager: `node_modules` everywhere, plus `.git` and `.yarn` under Yarn and `bower_components` under pnpm.
- A symlinked directory matched by a wildcard segment is now entered one level and listed, as one matched by a literal segment already was; `**` still never enters a symlinked directory.
- On a case-insensitive filesystem, a literal segment now matches loosely wherever it appears, so `A/*` matches `a/lib`.
- An empty pattern matches nothing instead of being an error, and a `/` inside a character class is accepted as a class member. A `\` remains an escape in every dialect; npm's rewriting of `\` to `/` is not reproduced.
- In a pnpm workspace, a matched directory holding a `package.yaml` or `package.json5` but no `package.json` is an error.
- A falsy `workspaces` field (`false`, `0`, `""`) is now passed over like `null`.
