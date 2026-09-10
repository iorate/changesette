# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`changesette` is a CLI implementing a reduced, data-format-compatible subset of [changesets](https://github.com/changesets/changesets) for single packages and npm / pnpm workspaces.

The crate has a library target (`src/lib.rs`) and a thin binary (`src/main.rs`) holding the clap definitions and the dispatch. The library API is internal: it exists for `main.rs` and the tests under `tests/`, and it is not covered by semver (only the CLI contract is). Use bare `pub` only for items that `main.rs` or `tests/` need; everything else crossing module boundaries is `pub(crate)`.

## Changesets

When a change affects the published binary or the setup action, add a changeset with `cargo run -- add --<bump> changesette -m <message>`. Write the message as full sentences ending with a period; it becomes a changelog entry as is.

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
