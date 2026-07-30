use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail, ensure};
use changesets::{Change, ChangeType, LoadingError, ParsingError};

use crate::bump::Bump;

const IGNORED_FILE_NAMES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

/// A changeset parsed from one `.changeset/*.md` file.
#[derive(Debug)]
pub(crate) struct LoadedChange {
    /// The file name within the changeset directory, e.g. `brave-lions-jump.md`.
    pub(crate) file_name: String,
    /// The widest bump type declared in the frontmatter.
    pub(crate) bump: Bump,
    /// The summary text below the frontmatter, trimmed.
    pub(crate) summary: String,
}

/// Loads every changeset in `changeset_dir` in file-name order, verifying
/// that each targets `package_name`. Dotfiles, non-`.md` files, README.md,
/// and agent instruction files are skipped.
pub(crate) fn load(changeset_dir: &Path, package_name: &str) -> Result<Vec<LoadedChange>> {
    let entries = match fs::read_dir(changeset_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => bail!(
            "{}: changeset directory not found; run `changesette add` to create it",
            changeset_dir.display()
        ),
        Err(err) => return Err(err).context(changeset_dir.display().to_string()),
    };

    let mut file_names = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| changeset_dir.display().to_string())?;
        if !entry
            .file_type()
            .with_context(|| entry.path().display().to_string())?
            .is_file()
        {
            continue;
        }
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if file_name.starts_with('.')
            || !file_name.ends_with(".md")
            || file_name.eq_ignore_ascii_case("README.md")
            || IGNORED_FILE_NAMES.contains(&file_name.as_str())
        {
            continue;
        }
        file_names.push(file_name);
    }
    file_names.sort();

    file_names
        .iter()
        .map(|file_name| load_one(changeset_dir, file_name, package_name))
        .collect()
}

/// Returns the widest bump among `changes`, or `None` if there are none.
pub(crate) fn max_bump(changes: &[LoadedChange]) -> Option<Bump> {
    changes.iter().map(|change| change.bump).max()
}

fn load_one(changeset_dir: &Path, file_name: &str, package_name: &str) -> Result<LoadedChange> {
    let file_path = changeset_dir.join(file_name);

    let content =
        fs::read_to_string(&file_path).with_context(|| file_path.display().to_string())?;
    let change = match Change::from_file_name_and_content(file_name, &content) {
        Ok(change) => change,
        Err(LoadingError::Parsing(ParsingError::InvalidVersioning(_))) => bail!(
            "{}: empty changeset (no bump specified)",
            file_path.display()
        ),
        Err(err) => return Err(err).context(file_path.display().to_string()),
    };

    let mut bump = None;
    for (name, change_type) in change.versioning.iter() {
        let name = strip_quotes(name);
        ensure!(
            name == package_name,
            "{}: changeset targets package `{name}`, but the manifest declares `{package_name}`",
            file_path.display()
        );
        let entry_bump = match change_type {
            ChangeType::Major => Bump::Major,
            ChangeType::Minor => Bump::Minor,
            ChangeType::Patch => Bump::Patch,
            ChangeType::Custom(change_type) => bail!(
                "{}: unknown bump type `{change_type}`; expected major, minor, or patch",
                file_path.display()
            ),
        };
        bump = bump.max(Some(entry_bump));
    }
    let Some(bump) = bump else {
        bail!(
            "{}: empty changeset (no bump specified)",
            file_path.display()
        )
    };

    let summary = change.summary.trim();
    ensure!(
        !summary.is_empty(),
        "{}: empty changeset (the summary is empty)",
        file_path.display()
    );

    Ok(LoadedChange {
        file_name: file_name.to_owned(),
        bump,
        summary: summary.to_owned(),
    })
}

fn strip_quotes(name: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(name) = name
            .strip_prefix(quote)
            .and_then(|name| name.strip_suffix(quote))
        {
            return name;
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> std::path::PathBuf {
        Path::new("tests/fixtures/changeset").join(case)
    }

    fn load_ok(case: &str) -> Vec<LoadedChange> {
        load(&fixture(case), "ublacklist").unwrap()
    }

    fn load_err(case: &str) -> String {
        format!("{:#}", load(&fixture(case), "ublacklist").unwrap_err())
    }

    #[test]
    fn rejects_a_missing_directory() {
        insta::assert_snapshot!(load_err("does-not-exist"));
    }

    #[test]
    fn sorts_by_file_name_and_skips_ignored_files() {
        insta::assert_debug_snapshot!(load_ok("ordering"));
    }

    #[test]
    fn parses_a_file_written_by_the_upstream_cli() {
        insta::assert_debug_snapshot!(load_ok("upstream-generated"));
    }

    #[test]
    fn keeps_multi_line_summaries() {
        insta::assert_debug_snapshot!(load_ok("multi-line-body"));
    }

    #[test]
    fn parses_crlf_files() {
        insta::assert_debug_snapshot!(load_ok("crlf"));
    }

    #[test]
    fn max_bump_is_none_without_changesets() {
        assert_eq!(max_bump(&[]), None);
    }

    #[test]
    fn max_bump_picks_the_highest() {
        assert_eq!(max_bump(&load_ok("ordering")), Some(Bump::Major));
    }

    #[test]
    fn rejects_a_changeset_that_also_targets_another_package() {
        insta::assert_snapshot!(load_err("two-packages"));
    }

    #[test]
    fn rejects_a_mismatched_package_name() {
        insta::assert_snapshot!(load_err("name-mismatch"));
    }

    #[test]
    fn rejects_a_frontmatter_only_file() {
        insta::assert_snapshot!(load_err("frontmatter-only"));
    }

    #[test]
    fn rejects_an_empty_frontmatter() {
        insta::assert_snapshot!(load_err("no-bump"));
    }

    #[test]
    fn rejects_a_custom_bump_type() {
        insta::assert_snapshot!(load_err("custom-type"));
    }
}
