# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`changesette` is a CLI implementing a reduced, data-format-compatible subset of [changesets](https://github.com/changesets/changesets) for single-package applications.

Pure bin crate: there is no library target, and items crossing module boundaries are `pub(crate)`, never bare `pub`.

Give every `pub(crate)` item a concise doc comment describing its contract, not its implementation, and keep doc comments in sync when changing code.

## Changesets

When a change affects the published binary, add a changeset with `cargo run -- add -b <bump> -m <message>`. Write the message as full sentences ending with a period; it becomes a changelog entry as is.

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
