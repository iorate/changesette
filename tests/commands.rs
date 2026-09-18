mod util;

use std::{fs, path::Path};

use changesette::{
    commands::{add::releases_from_flags, pre, set_summary},
    config::Config,
    workspace::{Versioned, Workspace},
};
use util::{
    dir_snapshot, package_dir, private_two_package_workspace_dir, read_pre_json,
    two_package_workspace_dir, write_config, write_file, write_pre_changeset, write_pre_json,
};

const PRE_JSON: &str = "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n";
const EXITED_PRE_JSON: &str = "{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n";

type Names<'a> = &'a [&'a str];

fn load(dir: &Path) -> (Workspace, Config) {
    changesette::load(dir, None, &[]).unwrap()
}

fn workspace(dir: &Path) -> Workspace {
    load(dir).0
}

fn error_text(result: anyhow::Result<()>) -> String {
    format!("{:#}", result.unwrap_err())
}

#[test]
fn pre_enter_accepts_a_dotted_tag() {
    let dir = package_dir();
    pre::enter(&workspace(dir.path()), "beta.2").unwrap();
    assert_eq!(
        read_pre_json(dir.path()),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta.2\"\n}\n"
    );
}

#[test]
fn pre_enter_fails_when_already_in_pre_mode() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    let before = dir_snapshot(dir.path());
    let err = error_text(pre::enter(&workspace(dir.path()), "alpha"));
    assert!(err.contains("already in pre mode"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn pre_exit_fails_without_pre_json() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let err = error_text(pre::exit(&workspace(dir.path())));
    assert!(err.contains("not in pre mode"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn pre_exit_is_idempotent() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    let workspace = workspace(dir.path());
    for _ in 0..2 {
        pre::exit(&workspace).unwrap();
        assert_eq!(read_pre_json(dir.path()), EXITED_PRE_JSON);
    }
}

#[test]
fn pre_exit_ignores_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    pre::exit(&workspace(dir.path())).unwrap();
    assert_eq!(
        read_pre_json(dir.path()),
        "{\n  \"mode\": \"exit\",\n  \"tag\": \"not a tag\"\n}\n"
    );
}

#[test]
fn set_summary_rewrites_a_pre_changeset() {
    let dir = package_dir();
    write_pre_changeset(
        dir.path(),
        "early.md",
        &[("ublacklist", "minor")],
        "Old summary",
    );
    set_summary::run(&workspace(dir.path()), "pre/early", "New summary").unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join(".changeset/pre/early.md")).unwrap(),
        "---\nublacklist: minor\n---\n\nNew summary\n"
    );
}

fn versioned(workspace: &Workspace) -> Vec<Versioned<'_>> {
    workspace.versioned().collect()
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn releases_from_flags_keeps_the_flag_order_and_dedupes_a_repeated_name() {
    let dir = two_package_workspace_dir();
    let workspace = workspace(dir.path());
    let releases = releases_from_flags(
        &workspace,
        &owned(&["pkg-b"]),
        &owned(&["pkg-a", "pkg-a"]),
        &[],
    )
    .unwrap();
    let releases: Vec<String> = releases
        .iter()
        .map(|(name, bump)| format!("{name} {}", bump.map_or("none", |bump| bump.as_str())))
        .collect();
    assert_eq!(releases, ["pkg-b major", "pkg-a minor"]);
}

#[test]
fn releases_from_flags_rejects_unknown_skipped_and_doubly_flagged_packages() {
    let dir = private_two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/c/package.json",
        "{\n  \"name\": \"pkg-c\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    write_config(dir.path(), "{ \"ignore\": [\"pkg-c\"] }\n");
    let workspace = workspace(dir.path());
    let versioned = versioned(&workspace);
    let names: Vec<&str> = versioned.iter().map(Versioned::name).collect();
    assert_eq!(names, ["pkg-a"]);

    let cases: [(Names, Names, Names, Names); 5] = [
        (
            &[],
            &["nope"],
            &[],
            &["`--minor`: package `nope` not found; known packages: pkg-a, pkg-b, pkg-c"],
        ),
        (
            &[],
            &[],
            &["pkg-b"],
            &["`--patch`: package `pkg-b` is skipped: private"],
        ),
        (
            &["pkg-c"],
            &[],
            &[],
            &["`--major`: package `pkg-c` is skipped: ignored"],
        ),
        (
            &[],
            &["pkg-a"],
            &["pkg-a"],
            &["`pkg-a`", "multiple bump type flags", "--minor, --patch"],
        ),
        (
            &["nope"],
            &["nope"],
            &[],
            &["`--major`: package `nope` not found"],
        ),
    ];
    for (major, minor, patch, needles) in cases {
        let err = releases_from_flags(&workspace, &owned(major), &owned(minor), &owned(patch))
            .err()
            .unwrap();
        let err = format!("{err:#}");
        assert!(!err.contains('\n'), "{err}");
        for needle in needles {
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }
}
