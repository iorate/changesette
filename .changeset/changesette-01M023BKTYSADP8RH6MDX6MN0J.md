---
changesette: major
---

Add npm and pnpm workspace support. The workspace root is discovered by walking up from the working directory, `init` creates the `.changeset` directory there, changesets may name multiple packages, and the `none` bump type and empty changesets are accepted. Dependencies are never bumped automatically; every bump must be named explicitly in a changeset. Breaking changes: `add` takes `--major`, `--minor`, and `--patch` package list flags instead of `-b, --bump`; `version` prints a JSON release plan to stdout; `changelog` is renamed to `get-changelog-entry <package> <version>`; `current` is replaced by `get-packages`, which prints the workspace packages as JSON.
