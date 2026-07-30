# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`changesette` is a CLI implementing a reduced, data-format-compatible subset of [changesets](https://github.com/changesets/changesets) for single-package applications.

Pure bin crate: there is no library target, and items crossing module boundaries are `pub(crate)`, never bare `pub`.

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
