# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`changesette` is a version and changelog manager for single packages and npm / yarn / pnpm workspaces, using the same changeset file format as [changesets](https://github.com/changesets/changesets).

## Changesets

When a change affects the published binary or the setup action, add a changeset with `cargo run -- add --<bump> changesette -m <message>`. Write the message as full sentences ending with a period; it becomes a changelog entry as is.

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
