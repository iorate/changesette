use std::{collections::BTreeMap, fs, io, path::Path, sync::LazyLock};

use anyhow::{Context, Result, bail};
use regex::Regex;
use saphyr::{LoadableYamlNode, Yaml};

use crate::bump::Bump;

const IGNORED_FILE_NAMES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

// A port of the upstream `@changesets/parse` regex; capture 1 is the
// frontmatter YAML and capture 2 is the summary.
static FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\s*---(.*?)\r?\n\s*---(\s*(?:\n|$).*)").unwrap());

#[derive(Debug)]
pub(crate) struct LoadedChange {
    pub(crate) file_name: String,
    /// The packages named in the frontmatter, in frontmatter order, each with
    /// its requested bump; `None` stands for the `none` type. Empty for an
    /// empty changeset.
    pub(crate) releases: Vec<(String, Option<Bump>)>,
    /// The summary text below the frontmatter, trimmed. May be empty, as in
    /// the upstream parser, which does not validate the summary.
    pub(crate) summary: String,
}

impl LoadedChange {
    pub(crate) fn id(&self) -> &str {
        self.file_name
            .strip_suffix(".md")
            .unwrap_or(&self.file_name)
    }
}

/// Loads every changeset in `changeset_dir`, in file-name order. Package
/// names are not validated here; callers match them against the workspace
/// members.
pub(crate) fn load(changeset_dir: &Path) -> Result<Vec<LoadedChange>> {
    let entries = match fs::read_dir(changeset_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => bail!(
            "{}: changeset directory not found; run `changesette init` to create it",
            changeset_dir.display()
        ),
        Err(err) => return Err(err).context(changeset_dir.display().to_string()),
    };

    let mut file_names = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| changeset_dir.display().to_string())?;
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        // Entries are selected by name alone, as in the upstream
        // `@changesets/read`: dotfiles, non-`.md` names, README.md, and agent
        // instruction files are skipped, symlinks are followed, and a
        // directory with an adopted name is a read error.
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
        .map(|file_name| load_one(changeset_dir, file_name))
        .collect()
}

/// Groups `changes` by package name: each named package maps to the widest
/// bump requested for it, or `None` when it is only ever named with the
/// `none` type.
pub(crate) fn max_bumps(changes: &[LoadedChange]) -> BTreeMap<&str, Option<Bump>> {
    let mut bumps = BTreeMap::new();
    for change in changes {
        for (name, bump) in &change.releases {
            let entry = bumps.entry(name.as_str()).or_insert(None);
            *entry = (*entry).max(*bump);
        }
    }
    bumps
}

fn load_one(changeset_dir: &Path, file_name: &str) -> Result<LoadedChange> {
    let file_path = changeset_dir.join(file_name);

    let content =
        fs::read_to_string(&file_path).with_context(|| file_path.display().to_string())?;
    let Some(captures) = FRONTMATTER.captures(&content) else {
        bail!(
            "{}: missing frontmatter (expected `---`-delimited YAML)",
            file_path.display()
        )
    };
    let frontmatter = &captures[1];
    let summary = captures[2].trim();

    let docs = match Yaml::load_from_str(frontmatter) {
        Ok(docs) => docs,
        Err(err) => bail!(
            "{}: invalid YAML in frontmatter: {err}",
            file_path.display()
        ),
    };

    let mut releases = Vec::new();
    match docs.into_iter().next() {
        None => {}
        Some(doc) if doc.is_null() => {}
        Some(Yaml::Mapping(mapping)) => {
            for (key, value) in &mapping {
                let Some(name) = key.as_str() else {
                    bail!(
                        "{}: invalid package name in frontmatter",
                        file_path.display()
                    )
                };
                let bump = match value.as_str() {
                    Some("major") => Some(Bump::Major),
                    Some("minor") => Some(Bump::Minor),
                    Some("patch") => Some(Bump::Patch),
                    Some("none") => None,
                    Some(other) => bail!(
                        "{}: unknown bump type {other:?}; expected major, minor, patch, or none",
                        file_path.display()
                    ),
                    None => bail!(
                        "{}: invalid bump type; expected major, minor, patch, or none",
                        file_path.display()
                    ),
                };
                releases.push((name.to_owned(), bump));
            }
        }
        Some(_) => bail!(
            "{}: frontmatter must be a mapping of package names to bump types",
            file_path.display()
        ),
    }

    Ok(LoadedChange {
        file_name: file_name.to_owned(),
        releases,
        summary: summary.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> std::path::PathBuf {
        Path::new("tests/fixtures/changeset").join(case)
    }

    fn load_ok(case: &str) -> Vec<LoadedChange> {
        load(&fixture(case)).unwrap()
    }

    fn load_err(case: &str) -> String {
        format!("{:#}", load(&fixture(case)).unwrap_err())
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
    fn parses_a_multi_package_file_written_by_the_upstream_cli() {
        insta::assert_debug_snapshot!(load_ok("upstream-multi-package"));
    }

    #[test]
    fn parses_a_none_file_written_by_the_upstream_cli() {
        insta::assert_debug_snapshot!(load_ok("upstream-none"));
    }

    #[test]
    fn parses_an_empty_file_written_by_the_upstream_cli() {
        insta::assert_debug_snapshot!(load_ok("upstream-empty"));
    }

    #[test]
    fn parses_a_two_package_changeset() {
        insta::assert_debug_snapshot!(load_ok("two-packages"));
    }

    #[test]
    fn parses_an_empty_frontmatter_with_a_summary() {
        insta::assert_debug_snapshot!(load_ok("empty-with-summary"));
    }

    #[test]
    fn follows_symlinked_changesets() {
        insta::assert_debug_snapshot!(load_ok("symlink"));
    }

    #[test]
    fn rejects_a_directory_with_an_adopted_name() {
        insta::assert_snapshot!(load_err("md-directory"));
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
    fn parses_a_quoted_bump_type() {
        insta::assert_debug_snapshot!(load_ok("quoted-value"));
    }

    #[test]
    fn parses_frontmatter_with_comments_and_blank_lines() {
        insta::assert_debug_snapshot!(load_ok("comments-and-blank-lines"));
    }

    #[test]
    fn max_bumps_is_empty_without_changesets() {
        assert!(max_bumps(&[]).is_empty());
    }

    #[test]
    fn max_bumps_picks_the_highest_per_package() {
        let changes = [
            LoadedChange {
                file_name: "a.md".into(),
                releases: vec![
                    ("one".into(), Some(Bump::Patch)),
                    ("two".into(), Some(Bump::Major)),
                ],
                summary: "a".into(),
            },
            LoadedChange {
                file_name: "b.md".into(),
                releases: vec![("one".into(), Some(Bump::Minor)), ("three".into(), None)],
                summary: "b".into(),
            },
        ];
        assert_eq!(
            max_bumps(&changes).into_iter().collect::<Vec<_>>(),
            [
                ("one", Some(Bump::Minor)),
                ("three", None),
                ("two", Some(Bump::Major)),
            ]
        );
    }

    #[test]
    fn max_bumps_keeps_none_below_any_bump() {
        let changes = [
            LoadedChange {
                file_name: "a.md".into(),
                releases: vec![("one".into(), None)],
                summary: "a".into(),
            },
            LoadedChange {
                file_name: "b.md".into(),
                releases: vec![("one".into(), Some(Bump::Patch))],
                summary: "b".into(),
            },
        ];
        assert_eq!(
            max_bumps(&changes).into_iter().collect::<Vec<_>>(),
            [("one", Some(Bump::Patch))]
        );
    }

    #[test]
    fn parses_a_frontmatter_only_file_with_an_empty_summary() {
        insta::assert_debug_snapshot!(load_ok("frontmatter-only"));
    }

    #[test]
    fn rejects_a_custom_bump_type() {
        insta::assert_snapshot!(load_err("custom-type"));
    }

    #[test]
    fn rejects_a_file_without_frontmatter() {
        insta::assert_snapshot!(load_err("no-frontmatter"));
    }

    #[test]
    fn rejects_invalid_yaml_in_frontmatter() {
        insta::assert_snapshot!(load_err("invalid-yaml"));
    }

    #[test]
    fn rejects_a_non_mapping_frontmatter() {
        insta::assert_snapshot!(load_err("non-mapping"));
    }

    #[test]
    fn rejects_a_missing_bump_value() {
        insta::assert_snapshot!(load_err("null-bump"));
    }
}
