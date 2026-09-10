use std::path::{Path, PathBuf};

use changesette::{
    bump::Bump,
    changeset::{self, LoadedChange},
};

const FILE: &str = "calmly-tidy-fox.md";

fn fixture(case: &str) -> PathBuf {
    Path::new("tests/fixtures/changeset").join(case)
}

fn load(case: &str) -> Vec<LoadedChange> {
    changeset::load(&fixture(case)).unwrap()
}

type View<'a> = Vec<(String, Vec<(&'a str, Option<Bump>)>, &'a str)>;

fn view(changes: &[LoadedChange]) -> View<'_> {
    changes
        .iter()
        .map(|change| {
            (
                change.id(),
                change
                    .releases
                    .iter()
                    .map(|(name, bump)| (name.as_str(), *bump))
                    .collect(),
                change.summary.as_str(),
            )
        })
        .collect()
}

fn render(releases: &[(&str, Option<Bump>)], summary: &str) -> String {
    let releases: Vec<(String, Option<Bump>)> = releases
        .iter()
        .map(|(name, bump)| ((*name).to_owned(), *bump))
        .collect();
    changeset::render(&releases, summary).unwrap()
}

#[test]
fn treats_a_missing_directory_as_empty() {
    assert!(load("does-not-exist").is_empty());
}

#[test]
fn sorts_by_file_name_and_skips_ignored_files() {
    assert_eq!(
        view(&load("ordering")),
        [
            (
                "10-second".to_owned(),
                vec![("ublacklist", Some(Bump::Minor))],
                "Sorted first because file names are compared by bytes, not numerically",
            ),
            (
                "2-third".to_owned(),
                vec![("ublacklist", Some(Bump::Major))],
                "Sorted second",
            ),
            (
                "boldly-brave-otter".to_owned(),
                vec![("ublacklist", Some(Bump::Patch))],
                "Sorted last, and written without quotes the way knope writes them",
            ),
        ]
    );
}

#[test]
fn loads_pre_changesets_last_with_prefixed_ids() {
    let changes = load("with-pre");
    assert_eq!(
        view(&changes),
        [
            (
                "brave-lions-dance".to_owned(),
                vec![("ublacklist", Some(Bump::Minor))],
                "A new changeset, loaded before the pre ones",
            ),
            (
                "pre/atomic-pugs-smile".to_owned(),
                vec![("ublacklist", Some(Bump::Patch))],
                "Parked first: file names are compared within the group",
            ),
            (
                "pre/zany-moons-sing".to_owned(),
                vec![("ublacklist", Some(Bump::Major))],
                "Parked second",
            ),
        ]
    );
    assert_eq!(
        changes
            .iter()
            .map(|change| change.in_pre)
            .collect::<Vec<_>>(),
        [false, true, true]
    );
    assert_eq!(changes[0].rel_path(), Path::new("brave-lions-dance.md"));
    assert_eq!(
        changes[1].rel_path(),
        Path::new("pre").join("atomic-pugs-smile.md")
    );
}

#[test]
fn parses_files_written_by_the_upstream_cli() {
    assert_eq!(
        view(&load("upstream-generated")),
        [(
            "olive-pots-repeat".to_owned(),
            vec![("ublacklist", Some(Bump::Minor))],
            "Add SERPINFO satellites support",
        )]
    );
    assert_eq!(
        view(&load("upstream-multi-package")),
        [(
            "frank-cycles-follow".to_owned(),
            vec![
                ("ublacklist", Some(Bump::Minor)),
                ("ublacklist-docs", Some(Bump::Patch)),
            ],
            "Touches two packages.",
        )]
    );
    assert_eq!(
        view(&load("upstream-none")),
        [(
            "wild-candies-bet".to_owned(),
            vec![("ublacklist", None)],
            "Declares no bump for the package.",
        )]
    );
    assert_eq!(
        view(&load("upstream-empty")),
        [("tricky-roses-poke".to_owned(), vec![], "")]
    );
}

#[test]
fn parses_empty_frontmatter_and_empty_summary() {
    assert_eq!(
        view(&load("empty-with-summary")),
        [(
            "calmly-tidy-fox".to_owned(),
            vec![],
            "An empty changeset, as `changeset add --empty` writes upstream",
        )]
    );
    assert_eq!(
        view(&load("frontmatter-only")),
        [(
            "calmly-tidy-fox".to_owned(),
            vec![("ublacklist", Some(Bump::Minor))],
            "",
        )]
    );
}

#[test]
fn follows_symlinked_changesets() {
    assert_eq!(
        view(&load("symlink")),
        [(
            "calmly-tidy-fox".to_owned(),
            vec![("ublacklist", Some(Bump::Patch))],
            "Fix something recorded in a symlinked changeset.",
        )]
    );
}

#[test]
fn rejects_a_directory_with_an_adopted_name() {
    let err = format!(
        "{:#}",
        changeset::load(&fixture("md-directory")).unwrap_err()
    );
    let path = fixture("md-directory").join("nested.md");
    assert!(err.contains(&format!("{}: ", path.display())), "{err}");
}

#[test]
fn keeps_multi_line_summaries() {
    assert_eq!(
        load("multi-line-body")[0].summary,
        "First line of body\nsecond line of body\n\n- a bullet\n- another bullet"
    );
}

#[test]
fn parses_crlf_files() {
    assert_eq!(
        view(&load("crlf")),
        [(
            "calmly-tidy-fox".to_owned(),
            vec![("ublacklist", Some(Bump::Patch))],
            "A body written with CRLF line endings\r\nand a second line",
        )]
    );
}

#[test]
fn parses_a_quoted_bump_type() {
    assert_eq!(
        load("quoted-value")[0].releases,
        [("ublacklist".to_owned(), Some(Bump::Minor))]
    );
}

#[test]
fn parses_frontmatter_with_comments_and_blank_lines() {
    assert_eq!(
        view(&load("comments-and-blank-lines")),
        [(
            "calmly-tidy-fox".to_owned(),
            vec![("ublacklist", Some(Bump::Patch))],
            "Frontmatter with a comment and a blank line",
        )]
    );
}

#[test]
fn max_bumps_picks_the_highest_per_package() {
    assert!(changeset::max_bumps(&[]).is_empty());
    let change = |file_name: &str, releases: &[(&str, Option<Bump>)]| LoadedChange {
        file_name: file_name.to_owned(),
        in_pre: false,
        releases: releases
            .iter()
            .map(|(name, bump)| ((*name).to_owned(), *bump))
            .collect(),
        summary: String::new(),
    };
    let changes = [
        change(
            "a.md",
            &[("one", Some(Bump::Patch)), ("two", Some(Bump::Major))],
        ),
        change("b.md", &[("one", Some(Bump::Minor)), ("three", None)]),
        change("c.md", &[("four", None)]),
        change("d.md", &[("four", Some(Bump::Patch))]),
    ];
    assert_eq!(
        changeset::max_bumps(&changes)
            .into_iter()
            .collect::<Vec<_>>(),
        [
            ("four", Some(Bump::Patch)),
            ("one", Some(Bump::Minor)),
            ("three", None),
            ("two", Some(Bump::Major)),
        ]
    );
}

#[test]
fn rejects_malformed_changesets() {
    for case in [
        "custom-type",
        "no-frontmatter",
        "invalid-yaml",
        "non-mapping",
        "null-bump",
    ] {
        let err = format!("{:#}", changeset::load(&fixture(case)).unwrap_err());
        let path = fixture(case).join(FILE);
        assert!(err.contains(&path.display().to_string()), "{case}: {err}");
    }
}

#[test]
fn renders_a_scoped_name_quoted() {
    assert_eq!(
        render(&[("@iorate/ublacklist", Some(Bump::Minor))], "Add feature"),
        "---\n\"@iorate/ublacklist\": minor\n---\n\nAdd feature\n"
    );
}

#[test]
fn renders_none_and_empty_releases() {
    assert_eq!(
        render(&[("ublacklist", None)], "New summary"),
        "---\nublacklist: none\n---\n\nNew summary\n"
    );
    assert_eq!(render(&[], "Note only"), "---\n---\n\nNote only\n");
    assert_eq!(render(&[], ""), "---\n---\n");
}

#[test]
fn renders_summaries_trimmed_and_multi_line() {
    let releases = [("ublacklist", Some(Bump::Minor))];
    assert_eq!(render(&releases, ""), "---\nublacklist: minor\n---\n");
    assert_eq!(render(&releases, " \n"), "---\nublacklist: minor\n---\n");
    assert_eq!(
        render(&releases, "\nAdd feature\n\n"),
        "---\nublacklist: minor\n---\n\nAdd feature\n"
    );
    assert_eq!(
        render(&releases, "line1\nline2"),
        "---\nublacklist: minor\n---\n\nline1\nline2\n"
    );
    assert_eq!(
        render(&releases, "First line.\n\nSecond line."),
        "---\nublacklist: minor\n---\n\nFirst line.\n\nSecond line.\n"
    );
}

#[test]
fn renders_releases_in_the_given_order() {
    assert_eq!(
        render(
            &[
                ("ublacklist", Some(Bump::Minor)),
                ("@iorate/ublacklist", Some(Bump::Patch)),
            ],
            "New summary",
        ),
        "---\nublacklist: minor\n\"@iorate/ublacklist\": patch\n---\n\nNew summary\n"
    );
    assert_eq!(
        render(
            &[("pkg-b", Some(Bump::Patch)), ("pkg-a", Some(Bump::Major))],
            "Summary",
        ),
        "---\npkg-b: patch\npkg-a: major\n---\n\nSummary\n"
    );
}
