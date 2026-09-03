---
changesette: minor
---

Add two ways around the workspace detection:

- `--root <DIR>` names the workspace root directly, so no ancestor of the working directory is looked at: only the `pnpm-workspace.yaml`, `yarn.lock`, or `workspaces` field in that directory decides how its members are enumerated, and a directory with none of them is a single package. A relative `DIR` is taken relative to the working directory. The option is also read from `CHANGESETTE_ROOT`; it wins over the variable, and an empty value counts as unset.
- The `changesette.packages` setting of `.changeset/config.json` — `{"changesette": {"packages": ["packages/a", "."]}}` — replaces the member enumeration entirely with the listed directories, each of which must hold a `package.json`. Each entry is a `/`-separated path relative to the root, taken as written: a `*` is a directory named `*`, not a wildcard, and `..` may name a directory outside the root. Neither the markers nor the workspace patterns are read, and the root `package.json` is read only when listed, so a pattern the built-in dialect rejects can be worked around by listing the directories. The listed packages still take the usual member qualification and the `ignore` / `privatePackages` settings.
