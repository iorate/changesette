mod util;

use std::{fs, path::Path};

use anyhow::Result;
use changesette::{
    commands::{
        status,
        version::{self, VersionArgs},
    },
    config::Config,
    plan::{self, PlannedVersion},
    pre::PreJson,
    release_plan,
    skip::SkipSet,
    snapshot::Snapshot,
    workspace::Workspace,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use util::{
    capture_output, dir_snapshot, expected_path, manifest_version, package_dir,
    prerelease_package_dir, private_two_package_workspace_dir, read_pre_json,
    two_package_workspace_dir, workspace_dir, write_changeset, write_config, write_file,
    write_pre_changeset, write_pre_json,
};

const FILE_A: &str = "boldly-brave-otter.md";
const FILE_B: &str = "calmly-tidy-fox.md";
const ID_A: &str = "boldly-brave-otter";
const ID_B: &str = "calmly-tidy-fox";
const PRE_JSON: &str = "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n";
const EXITED_PRE_JSON: &str = "{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n";
const EMPTY_PLAN: &str = "{\n  \"changesets\": [],\n  \"releases\": []\n}\n";

type Setup = fn() -> TempDir;
type Names<'a> = &'a [&'a str];
type Releases<'a> = &'a [(&'a str, &'a str)];
type Changesets<'a> = &'a [(&'a str, Releases<'a>)];

fn pkg(name: &str, version: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\"\n}}\n")
}

fn private_pkg(name: &str, version: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"private\": true\n}}\n")
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn args() -> VersionArgs {
    VersionArgs {
        ignore: Vec::new(),
        snapshot: None,
        snapshot_prerelease_template: None,
        allow_no_changesets: false,
        output: None,
    }
}

fn ignoring(names: &[&str]) -> VersionArgs {
    VersionArgs {
        ignore: owned(names),
        ..args()
    }
}

fn snapshot_args(tag: Option<&str>, template: Option<&str>) -> VersionArgs {
    VersionArgs {
        snapshot: Some(tag.map(str::to_owned)),
        snapshot_prerelease_template: template.map(str::to_owned),
        ..args()
    }
}

fn load(dir: &Path) -> (Workspace, Config) {
    changesette::load(dir, None).unwrap()
}

fn run_with(dir: &Path, args: VersionArgs) -> Result<()> {
    let (workspace, config) = load(dir);
    version::run(workspace, &config, args)
}

fn run_ok(dir: &Path) {
    run_with(dir, args()).unwrap();
}

fn run_err_with(dir: &Path, args: VersionArgs) -> String {
    format!("{:#}", run_with(dir, args).unwrap_err())
}

fn run_err(dir: &Path) -> String {
    run_err_with(dir, args())
}

fn plan_with(dir: &Path, ignore: &[&str], snapshot: Option<&Snapshot>) -> Result<PlannedVersion> {
    let (workspace, config) = load(dir);
    plan::plan_version(workspace, &config, &owned(ignore), snapshot)
}

fn plan(dir: &Path) -> PlannedVersion {
    plan_with(dir, &[], None).unwrap()
}

fn plan_err(dir: &Path) -> String {
    match plan_with(dir, &[], None) {
        Ok(_) => panic!("the plan unexpectedly succeeded"),
        Err(err) => format!("{err:#}"),
    }
}

fn releases(planned: &PlannedVersion) -> Vec<String> {
    planned
        .releases
        .iter()
        .map(|release| {
            format!(
                "{} {} {} -> {}",
                release.name,
                release.bump.map_or("none", |bump| bump.as_str()),
                release.old_version,
                release.new_version
            )
        })
        .collect()
}

fn plan_json(planned: &PlannedVersion) -> Value {
    serde_json::to_value(release_plan::build(
        &planned.changes,
        &planned.releases,
        planned.pre.as_ref(),
    ))
    .unwrap()
}

fn status_to_file(dir: &Path, path: &Path) -> Result<()> {
    let (workspace, config) = load(dir);
    status::run(workspace, &config, false, Some(path))
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn exists(dir: &Path, rel: &str) -> bool {
    dir.join(rel).exists()
}

fn assert_datetime(text: &str) {
    assert_eq!(text.len(), 14, "unexpected datetime: {text}");
    assert!(
        text.chars().all(|c| c.is_ascii_digit()),
        "unexpected datetime: {text}"
    );
}

#[test]
fn allow_no_changesets_succeeds_and_writes_an_empty_plan() {
    let dir = package_dir();
    let before = dir_snapshot(dir.path());
    run_with(
        dir.path(),
        VersionArgs {
            allow_no_changesets: true,
            ..args()
        },
    )
    .unwrap();
    assert_eq!(dir_snapshot(dir.path()), before);
    assert!(!exists(dir.path(), ".changeset"));

    run_with(
        dir.path(),
        VersionArgs {
            allow_no_changesets: true,
            output: Some(dir.path().join("plan.json")),
            ..args()
        },
    )
    .unwrap();
    assert_eq!(read(dir.path(), "plan.json"), EMPTY_PLAN);
    assert!(!exists(dir.path(), ".changeset"));
}

#[test]
fn output_with_zero_changesets_fails_without_writing_the_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let err = run_err_with(
        dir.path(),
        VersionArgs {
            output: Some(dir.path().join("plan.json")),
            ..args()
        },
    );
    assert!(err.contains("no unreleased changesets found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn uses_the_max_bump_across_changesets() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_A,
        &[("ublacklist", "major")],
        "Rework everything",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["ublacklist major 1.2.3 -> 2.0.0"]);
    assert_eq!(planned.releases[0].changeset_ids, [ID_A, ID_B]);

    run_ok(dir.path());
    assert_eq!(manifest_version(dir.path(), "package.json"), "2.0.0");
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 2.0.0\n\n### Major Changes\n\n- Rework everything\n\n### Patch Changes\n\n- Fix bug\n"
    );
    assert!(!dir.path().join(".changeset").join(FILE_A).exists());
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn bumps_only_the_named_workspace_members() {
    let dir = two_package_workspace_dir();
    let untouched = pkg("pkg-c", "3.0.0");
    write_file(dir.path(), "packages/c/package.json", &untouched);
    write_changeset(
        dir.path(),
        FILE_B,
        &[("pkg-b", "patch"), ("pkg-a", "minor")],
        "Improve things",
    );
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["pkg-a minor 3.1.4 -> 3.2.0", "pkg-b patch 2.0.0 -> 2.0.1"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        pkg("pkg-a", "3.2.0")
    );
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 3.2.0\n\n### Minor Changes\n\n- Improve things\n"
    );
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        pkg("pkg-b", "2.0.1")
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Improve things\n"
    );
    assert_eq!(read(dir.path(), "packages/c/package.json"), untouched);
    assert!(!exists(dir.path(), "packages/c/CHANGELOG.md"));
    assert!(!exists(dir.path(), "CHANGELOG.md"));
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn consumes_a_none_only_changeset_without_bumping() {
    let dir = package_dir();
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "none")], "Note only");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["ublacklist none 1.2.3 -> 1.2.3"]);
    assert!(planned.releases[0].changelog_entry.is_none());

    run_ok(dir.path());
    assert_eq!(read(dir.path(), "package.json"), pkg("ublacklist", "1.2.3"));
    assert!(!exists(dir.path(), "CHANGELOG.md"));
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn consumes_an_empty_changeset() {
    let dir = package_dir();
    write_changeset(dir.path(), FILE_B, &[], "");
    let planned = plan(dir.path());
    assert!(planned.releases.is_empty());
    assert_eq!(planned.changes.len(), 1);

    run_ok(dir.path());
    assert!(!exists(dir.path(), "CHANGELOG.md"));
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn fails_for_a_changeset_naming_an_unknown_package_leaving_the_tree_untouched() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("other-package", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = run_err(dir.path());
    assert!(err.contains(FILE_B), "{err}");
    assert!(err.contains("`other-package` not found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);

    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{\n  \"name\": \"ublacklist\"\n}\n",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let before = dir_snapshot(dir.path());
    let err = run_err(dir.path());
    assert!(err.contains("`ublacklist` not found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn leaves_the_package_lock_untouched() {
    let dir = package_dir();
    let package_lock = "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"lockfileVersion\": 3,\n  \"packages\": {\n    \"\": {\n      \"name\": \"ublacklist\",\n      \"version\": \"1.2.3\"\n    }\n  }\n}\n";
    write_file(dir.path(), "package-lock.json", package_lock);
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_ok(dir.path());
    assert_eq!(manifest_version(dir.path(), "package.json"), "1.3.0");
    assert_eq!(read(dir.path(), "package-lock.json"), package_lock);
}

#[test]
fn skipped_packages_keep_their_changesets() {
    let cases: [(Setup, Option<&str>, Names); 3] = [
        (two_package_workspace_dir, None, &["pkg-b"]),
        (
            two_package_workspace_dir,
            Some("{ \"ignore\": [\"pkg-b\"] }\n"),
            &[],
        ),
        (private_two_package_workspace_dir, None, &[]),
    ];
    for (make_dir, config, ignore) in cases {
        let dir = make_dir();
        if let Some(config) = config {
            write_config(dir.path(), config);
        }
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
        write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
        let b_manifest = read(dir.path(), "packages/b/package.json");
        let planned = plan_with(dir.path(), ignore, None).unwrap();
        assert_eq!(releases(&planned), ["pkg-a minor 3.1.4 -> 3.2.0"]);

        run_with(dir.path(), ignoring(ignore)).unwrap();
        assert_eq!(
            manifest_version(dir.path(), "packages/a/package.json"),
            "3.2.0"
        );
        assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
        assert!(!exists(dir.path(), "packages/b/CHANGELOG.md"));
        assert!(!dir.path().join(".changeset").join(FILE_A).exists());
        assert!(dir.path().join(".changeset").join(FILE_B).exists());
    }
}

#[test]
fn release_plan_lists_skipped_changesets_without_a_release() {
    let dir = private_two_package_workspace_dir();
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let planned = plan(dir.path());
    assert_eq!(
        plan_json(&planned),
        json!({
            "changesets": [
                {
                    "id": ID_A,
                    "summary": "Improve pkg-a",
                    "releases": [{ "name": "pkg-a", "type": "minor" }]
                },
                {
                    "id": ID_B,
                    "summary": "Fix pkg-b",
                    "releases": [{ "name": "pkg-b", "type": "patch" }]
                }
            ],
            "releases": [
                {
                    "name": "pkg-a",
                    "type": "minor",
                    "oldVersion": "3.1.4",
                    "newVersion": "3.2.0",
                    "changesets": [ID_A],
                    "changelogEntry": "### Minor Changes\n\n- Improve pkg-a"
                }
            ]
        })
    );
}

#[test]
fn ignore_rejects_an_unknown_package() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = run_err_with(dir.path(), ignoring(&["other-package"]));
    assert!(err.contains("--ignore"), "{err}");
    assert!(err.contains("`other-package` not found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn succeeds_when_every_changeset_is_skipped() {
    let cases: [(Setup, Releases, Names); 2] = [
        (package_dir, &[("ublacklist", "none")], &["ublacklist"]),
        (
            private_two_package_workspace_dir,
            &[("pkg-b", "patch")],
            &[],
        ),
    ];
    for (make_dir, changeset, ignore) in cases {
        let dir = make_dir();
        write_changeset(dir.path(), FILE_B, changeset, "Note only");
        let before = dir_snapshot(dir.path());
        let planned = plan_with(dir.path(), ignore, None).unwrap();
        assert_eq!(planned.changes.len(), 1);
        assert!(planned.consumed_changes.is_empty());
        assert!(planned.releases.is_empty());

        run_with(dir.path(), ignoring(ignore)).unwrap();
        assert_eq!(dir_snapshot(dir.path()), before);
    }
}

#[test]
fn filter_changes_rejects_a_mixed_changeset() {
    let cases: [(Setup, Releases, Names); 3] = [
        (
            two_package_workspace_dir,
            &[("pkg-a", "minor"), ("pkg-b", "none")],
            &["pkg-a"],
        ),
        (
            two_package_workspace_dir,
            &[("pkg-a", "minor"), ("pkg-b", "patch")],
            &["pkg-a"],
        ),
        (
            private_two_package_workspace_dir,
            &[("pkg-a", "minor"), ("pkg-b", "patch")],
            &[],
        ),
    ];
    for (make_dir, changeset, ignore) in cases {
        let dir = make_dir();
        write_changeset(dir.path(), FILE_B, changeset, "Improve things");
        let before = dir_snapshot(dir.path());
        let err = run_err_with(dir.path(), ignoring(ignore));
        assert!(err.contains(FILE_B), "{err}");
        assert!(err.contains("cannot mix skipped packages"), "{err}");
        assert!(err.contains("`pkg-a`"), "{err}");
        assert!(err.contains("`pkg-b`"), "{err}");
        assert_eq!(dir_snapshot(dir.path()), before);
    }
}

#[test]
fn the_ignore_flag_and_a_config_ignore_are_exclusive() {
    for (config, ignore) in [
        ("{ \"ignore\": [\"pkg-b\"] }\n", "pkg-a"),
        ("{ \"ignore\": [\"missing-*\"] }\n", "pkg-b"),
    ] {
        let dir = two_package_workspace_dir();
        write_config(dir.path(), config);
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
        let before = dir_snapshot(dir.path());
        let err = run_err_with(dir.path(), ignoring(&[ignore]));
        assert!(err.contains("--ignore"), "{err}");
        assert!(err.contains("use only one of them"), "{err}");
        assert_eq!(dir_snapshot(dir.path()), before);
    }

    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    run_with(dir.path(), ignoring(&["pkg-b"])).unwrap();
    assert_eq!(
        manifest_version(dir.path(), "packages/a/package.json"),
        "3.2.0"
    );
    assert_eq!(
        manifest_version(dir.path(), "packages/b/package.json"),
        "2.0.0"
    );
    assert!(dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn private_packages_are_versioned_only_when_configured() {
    let dir = private_two_package_workspace_dir();
    let (workspace, config) = load(dir.path());
    let skip = SkipSet::load(&workspace, &config, &[]).unwrap();
    assert!(!skip.contains("pkg-a"));
    assert!(skip.contains("pkg-b"));

    write_config(
        dir.path(),
        "{ \"privatePackages\": { \"version\": true } }\n",
    );
    let (workspace, config) = load(dir.path());
    let skip = SkipSet::load(&workspace, &config, &[]).unwrap();
    assert!(!skip.contains("pkg-b"));

    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        private_pkg("pkg-b", "2.0.1")
    );
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn skip_set_reports_the_reasons_at_debug() {
    let dir = private_two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-a\"] }\n");
    let (workspace, config) = load(dir.path());
    let output = capture_output(|| {
        let skip = SkipSet::load(&workspace, &config, &[]).unwrap();
        assert!(skip.contains("pkg-a"));
        assert!(skip.contains("pkg-b"));
    });
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("is skipped"))
        .collect();
    assert_eq!(lines.len(), 2, "{output}");
    assert!(lines[0].starts_with("debug: "), "{output}");
    assert!(lines[0].contains("`pkg-a`"), "{output}");
    assert!(lines[0].contains("ignored"), "{output}");
    assert!(lines[1].starts_with("debug: "), "{output}");
    assert!(lines[1].contains("`pkg-b`"), "{output}");
    assert!(lines[1].contains("private"), "{output}");
}

#[test]
fn fixed_bumps_the_partner_with_a_heading_only_changelog() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["pkg-a minor 3.1.4 -> 3.2.0", "pkg-b minor 3.1.4 -> 3.2.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        pkg("pkg-a", "3.2.0")
    );
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 3.2.0\n\n### Minor Changes\n\n- Improve pkg-a\n"
    );
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        pkg("pkg-b", "3.2.0")
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 3.2.0\n"
    );
    assert!(!dir.path().join(".changeset").join(FILE_A).exists());
}

#[test]
fn linked_does_not_bump_a_non_releasing_member() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["pkg-b patch 3.1.4 -> 3.1.5"]);

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        pkg("pkg-a", "3.1.4")
    );
    assert!(!exists(dir.path(), "packages/a/CHANGELOG.md"));
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        pkg("pkg-b", "3.1.5")
    );
}

#[test]
fn linked_aligns_the_releasing_members() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "minor")], "Improve pkg-b");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["pkg-a minor 3.1.4 -> 3.2.0", "pkg-b minor 3.1.4 -> 3.2.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        manifest_version(dir.path(), "packages/a/package.json"),
        "3.2.0"
    );
    assert_eq!(
        manifest_version(dir.path(), "packages/b/package.json"),
        "3.2.0"
    );
}

#[test]
fn groups_report_the_raised_bump_at_debug() {
    let cases: [(&str, Changesets, &str); 2] = [
        ("fixed", &[(FILE_A, &[("pkg-a", "minor")])], "pkg-b"),
        (
            "linked",
            &[
                (FILE_A, &[("pkg-a", "patch")]),
                (FILE_B, &[("pkg-b", "minor")]),
            ],
            "pkg-a",
        ),
    ];
    for (kind, changesets, raised) in cases {
        let dir = two_package_workspace_dir();
        write_config(
            dir.path(),
            &format!("{{ \"{kind}\": [[\"pkg-a\", \"pkg-b\"]] }}\n"),
        );
        for (file_name, releases) in changesets {
            write_changeset(dir.path(), file_name, releases, "Change");
        }
        let output = capture_output(|| {
            let planned = plan(dir.path());
            assert_eq!(
                releases(&planned),
                ["pkg-a minor 3.1.4 -> 3.2.0", "pkg-b minor 3.1.4 -> 3.2.0"]
            );
        });
        let lines: Vec<&str> = output
            .lines()
            .filter(|line| line.contains("raises the bump"))
            .collect();
        assert_eq!(lines.len(), 1, "{output}");
        assert!(lines[0].starts_with("debug: "), "{output}");
        assert!(lines[0].contains(&format!("`{raised}`")), "{output}");
        assert!(lines[0].contains(&format!("\"{kind}\"")), "{output}");
        assert!(lines[0].contains("minor"), "{output}");
        assert!(lines[0].contains("3.1.4"), "{output}");
    }
}

#[test]
fn fixed_counts_a_skipped_member_without_adding_it() {
    let dir = workspace_dir();
    let b_manifest = private_pkg("pkg-b", "9.9.9");
    write_file(dir.path(), "packages/b/package.json", &b_manifest);
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["pkg-a minor 9.9.9 -> 9.10.0"]);

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        pkg("pkg-a", "9.10.0")
    );
    assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
    assert!(!exists(dir.path(), "packages/b/CHANGELOG.md"));
}

#[test]
fn groups_align_the_pre_counter() {
    let cases = [
        (
            "fixed",
            "1.0.1-beta.7",
            pkg("pkg-b", "2.0.0-beta.2"),
            "pkg-b",
            "2.0.0-beta.8",
            "2.0.0-beta.8",
        ),
        (
            "linked",
            "1.0.1-beta.7",
            pkg("pkg-b", "2.0.0-beta.2"),
            "pkg-b",
            "1.0.1-beta.7",
            "2.0.0-beta.8",
        ),
        (
            "fixed",
            "2.0.0-beta.2",
            private_pkg("pkg-b", "1.0.1-beta.7"),
            "pkg-a",
            "2.0.0-beta.8",
            "1.0.1-beta.7",
        ),
    ];
    for (kind, a_version, b_manifest, changed, expected_a, expected_b) in cases {
        let dir = workspace_dir();
        write_file(
            dir.path(),
            "packages/a/package.json",
            &pkg("pkg-a", a_version),
        );
        write_file(dir.path(), "packages/b/package.json", &b_manifest);
        write_config(
            dir.path(),
            &format!("{{ \"{kind}\": [[\"pkg-a\", \"pkg-b\"]] }}\n"),
        );
        write_pre_json(dir.path(), PRE_JSON);
        write_changeset(dir.path(), FILE_B, &[(changed, "patch")], "Fix");
        run_ok(dir.path());
        assert_eq!(
            manifest_version(dir.path(), "packages/a/package.json"),
            expected_a,
            "{kind}: {changed}"
        );
        assert_eq!(
            manifest_version(dir.path(), "packages/b/package.json"),
            expected_b,
            "{kind}: {changed}"
        );
        assert!(dir.path().join(".changeset/pre").join(FILE_B).exists());
    }
}

#[test]
fn release_plan_reports_the_group_old_version() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let planned = plan(dir.path());
    assert_eq!(
        plan_json(&planned)["releases"][1],
        json!({
            "name": "pkg-b",
            "type": "minor",
            "oldVersion": "3.1.4",
            "newVersion": "3.2.0",
            "changesets": [],
            "changelogEntry": ""
        })
    );
}

#[test]
fn plan_version_names_the_config_on_a_group_error() {
    let dir = two_package_workspace_dir();
    write_config(
        dir.path(),
        "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]], \"linked\": [[\"pkg-b\"]] }\n",
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let before = dir_snapshot(dir.path());
    let err = run_err(dir.path());
    assert!(
        err.contains(&expected_path(dir.path(), ".changeset/config.json")),
        "{err}"
    );
    assert!(err.contains("`pkg-b`"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn after_exit_rescues_a_linked_partner_without_a_prerelease() {
    let dir = two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/b/package.json",
        &pkg("pkg-b", "2.0.1-beta.4"),
    );
    write_config(dir.path(), "{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let planned = plan(dir.path());
    assert!(planned.exiting_pre());
    assert_eq!(
        releases(&planned),
        [
            "pkg-a patch 3.1.4 -> 3.1.5",
            "pkg-b patch 2.0.1-beta.4 -> 2.0.1"
        ]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 3.1.5\n"
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 2.0.1\n"
    );
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_rescues_the_fixed_group_of_a_skipped_prerelease() {
    let dir = workspace_dir();
    let b_manifest = private_pkg("pkg-b", "2.0.0-beta.1");
    write_file(dir.path(), "packages/b/package.json", &b_manifest);
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["pkg-a patch 3.1.4 -> 3.1.5"]);

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 3.1.5\n"
    );
    assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn linked_leaves_a_none_only_member_unchanged() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "none")], "Note only");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["pkg-a patch 3.1.4 -> 3.1.5", "pkg-b none 2.0.0 -> 2.0.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        pkg("pkg-b", "2.0.0")
    );
    assert!(!exists(dir.path(), "packages/b/CHANGELOG.md"));
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn fixed_upgrades_a_none_only_member() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "none")], "Note only");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["pkg-a minor 3.1.4 -> 3.2.0", "pkg-b minor 3.1.4 -> 3.2.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 3.2.0\n"
    );
    assert!(!dir.path().join(".changeset").join(FILE_A).exists());
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn fixed_is_not_triggered_by_a_none_only_member() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "none")], "Note only");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["pkg-a none 3.1.4 -> 3.1.4"]);

    run_ok(dir.path());
    assert_eq!(
        manifest_version(dir.path(), "packages/a/package.json"),
        "3.1.4"
    );
    assert_eq!(
        manifest_version(dir.path(), "packages/b/package.json"),
        "2.0.0"
    );
    assert!(!dir.path().join(".changeset").join(FILE_A).exists());
}

#[test]
fn in_pre_mode_bumps_to_a_prerelease() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let planned = plan(dir.path());
    assert_eq!(planned.in_pre().map(PreJson::tag), Some("beta"));
    assert_eq!(
        releases(&planned),
        ["ublacklist minor 1.2.3 -> 1.3.0-beta.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "package.json"),
        pkg("ublacklist", "1.3.0-beta.0")
    );
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
    assert!(dir.path().join(".changeset/pre").join(FILE_B).is_file());
    assert_eq!(read_pre_json(dir.path()), PRE_JSON);
}

#[test]
fn in_pre_mode_increments_the_counter() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        FILE_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_ok(dir.path());

    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["ublacklist patch 1.3.0-beta.0 -> 1.3.0-beta.1"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0-beta.1\n\n### Patch Changes\n\n- Fix bug\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(dir.path().join(".changeset/pre").join(FILE_A).is_file());
    assert!(dir.path().join(".changeset/pre").join(FILE_B).is_file());
}

#[test]
fn in_pre_mode_fails_on_a_move_collision() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_pre_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = run_err(dir.path());
    assert!(err.contains(FILE_B), "{err}");
    assert!(err.contains("refusing to overwrite"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn plan_version_rejects_an_invalid_pre_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = plan_err(dir.path());
    assert!(err.contains("invalid pre tag"), "{err}");
    assert!(err.contains("not a tag"), "{err}");
    let err = run_err(dir.path());
    assert!(err.contains("invalid pre tag"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn in_pre_mode_leaves_skipped_changesets_in_place() {
    let cases: [(Setup, Names); 2] = [
        (two_package_workspace_dir, &["pkg-b"]),
        (private_two_package_workspace_dir, &[]),
    ];
    for (make_dir, ignore) in cases {
        let dir = make_dir();
        write_pre_json(dir.path(), PRE_JSON);
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
        write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
        let b_manifest = read(dir.path(), "packages/b/package.json");
        run_with(dir.path(), ignoring(ignore)).unwrap();
        assert_eq!(
            manifest_version(dir.path(), "packages/a/package.json"),
            "3.2.0-beta.0"
        );
        assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
        assert!(dir.path().join(".changeset/pre").join(FILE_A).is_file());
        assert!(dir.path().join(".changeset").join(FILE_B).is_file());
        assert!(!dir.path().join(".changeset/pre").join(FILE_B).exists());
    }
}

#[test]
fn in_pre_mode_keeps_a_none_only_package_unchanged() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "none")], "Note only");
    run_ok(dir.path());
    assert_eq!(read(dir.path(), "package.json"), pkg("ublacklist", "1.2.3"));
    assert!(!exists(dir.path(), "CHANGELOG.md"));
    assert!(dir.path().join(".changeset/pre").join(FILE_B).is_file());
}

#[test]
fn in_pre_mode_with_no_new_changesets_fails() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_pre_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let planned = plan(dir.path());
    assert!(planned.changes.is_empty());
    let err = run_err(dir.path());
    assert!(err.contains("no unreleased changesets found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn after_exit_finalizes() {
    let dir = prerelease_package_dir("1.3.0-beta.0");
    write_file(
        dir.path(),
        "CHANGELOG.md",
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
    );
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(
        dir.path(),
        FILE_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let planned = plan(dir.path());
    assert!(planned.exiting_pre());
    assert_eq!(
        releases(&planned),
        ["ublacklist minor 1.3.0-beta.0 -> 1.3.0"]
    );

    run_ok(dir.path());
    assert_eq!(read(dir.path(), "package.json"), pkg("ublacklist", "1.3.0"));
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- Add feature\n\n### Patch Changes\n\n- Fix bug\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(FILE_A).exists());
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_rescues_prerelease_packages() {
    let dir = two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/a/package.json",
        &pkg("pkg-a", "3.2.0-beta.0"),
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        &pkg("pkg-b", "2.0.1-alpha.2"),
    );
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let planned = plan_with(dir.path(), &["pkg-b"], None).unwrap();
    assert_eq!(releases(&planned), ["pkg-a patch 3.2.0-beta.0 -> 3.2.0"]);

    run_with(dir.path(), ignoring(&["pkg-b"])).unwrap();
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        pkg("pkg-a", "3.2.0")
    );
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 3.2.0\n"
    );
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        pkg("pkg-b", "2.0.1-alpha.2")
    );
    assert!(!exists(dir.path(), "packages/b/CHANGELOG.md"));
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_rescues_a_none_only_package() {
    let dir = prerelease_package_dir("1.2.3-beta.1");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(dir.path(), FILE_B, &[("ublacklist", "none")], "Note only");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        ["ublacklist patch 1.2.3-beta.1 -> 1.2.3"]
    );

    run_ok(dir.path());
    assert_eq!(read(dir.path(), "package.json"), pkg("ublacklist", "1.2.3"));
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 1.2.3\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(FILE_B).exists());
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_does_not_rescue_a_skipped_prerelease() {
    let dir = private_two_package_workspace_dir();
    let b_manifest = private_pkg("pkg-b", "2.1.0-beta.0");
    write_file(dir.path(), "packages/b/package.json", &b_manifest);
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let planned = plan(dir.path());
    assert!(planned.releases.is_empty());

    run_ok(dir.path());
    assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_succeeds_with_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"exit\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_ok(dir.path());
    assert_eq!(manifest_version(dir.path(), "package.json"), "1.3.0");
    assert!(!exists(dir.path(), ".changeset/pre.json"));
}

#[test]
fn after_exit_with_no_changesets_still_deletes_pre_json() {
    let dir = package_dir();
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    run_ok(dir.path());
    assert!(!exists(dir.path(), ".changeset/pre.json"));
    assert_eq!(read(dir.path(), "package.json"), pkg("ublacklist", "1.2.3"));
}

#[test]
fn consumes_pre_changesets_without_pre_json() {
    let dir = package_dir();
    write_pre_changeset(
        dir.path(),
        FILE_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let planned = plan(dir.path());
    assert_eq!(releases(&planned), ["ublacklist minor 1.2.3 -> 1.3.0"]);

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- Add feature\n\n### Patch Changes\n\n- Fix bug\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(FILE_A).exists());
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn release_plan_carries_pre_state_and_prefixed_ids() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let plan_value = plan_json(&plan(dir.path()));
    assert!(plan_value.get("preState").is_none(), "{plan_value}");
    assert_eq!(plan_value["releases"][0]["newVersion"], "1.3.0");

    write_pre_json(dir.path(), PRE_JSON);
    let plan_value = plan_json(&plan(dir.path()));
    assert_eq!(
        plan_value["preState"],
        json!({ "mode": "pre", "tag": "beta" })
    );
    assert_eq!(plan_value["changesets"][0]["id"], ID_B);
    assert_eq!(plan_value["releases"][0]["newVersion"], "1.3.0-beta.0");

    let dir = prerelease_package_dir("1.3.0-beta.0");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(
        dir.path(),
        FILE_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let plan_value = plan_json(&plan(dir.path()));
    assert_eq!(
        plan_value["preState"],
        json!({ "mode": "exit", "tag": "beta" })
    );
    assert_eq!(plan_value["changesets"][0]["id"], format!("pre/{ID_A}"));
    assert_eq!(
        plan_value["releases"][0]["changesets"],
        json!([format!("pre/{ID_A}")])
    );
    assert_eq!(plan_value["releases"][0]["newVersion"], "1.3.0");
}

#[test]
fn status_writes_the_plan_without_modifying_files() {
    let dir = package_dir();
    let plan_path = dir.path().join("plan.json");
    status_to_file(dir.path(), &plan_path).unwrap();
    assert_eq!(read(dir.path(), "plan.json"), EMPTY_PLAN);
    assert!(!exists(dir.path(), ".changeset"));
    fs::remove_file(&plan_path).unwrap();

    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    status_to_file(dir.path(), &plan_path).unwrap();
    let plan_value: Value = serde_json::from_str(&read(dir.path(), "plan.json")).unwrap();
    assert_eq!(
        plan_value,
        json!({
            "changesets": [
                {
                    "id": ID_B,
                    "summary": "Add feature",
                    "releases": [{ "name": "ublacklist", "type": "minor" }]
                }
            ],
            "releases": [
                {
                    "name": "ublacklist",
                    "type": "minor",
                    "oldVersion": "1.2.3",
                    "newVersion": "1.3.0",
                    "changesets": [ID_B],
                    "changelogEntry": "### Minor Changes\n\n- Add feature"
                }
            ]
        })
    );
    fs::remove_file(&plan_path).unwrap();
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn snapshot_bumps_to_a_zero_based_version() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_with(dir.path(), snapshot_args(None, None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    let suffix = version
        .strip_prefix("0.0.0-")
        .unwrap_or_else(|| panic!("unexpected version: {version}"));
    assert_datetime(suffix);
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        format!("# ublacklist\n\n## {version}\n\n### Minor Changes\n\n- Add feature\n")
    );
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn snapshot_with_a_tag_prefixes_the_suffix() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_with(dir.path(), snapshot_args(Some("canary"), None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    let suffix = version
        .strip_prefix("0.0.0-canary-")
        .unwrap_or_else(|| panic!("unexpected version: {version}"));
    assert_datetime(suffix);
}

#[test]
fn snapshot_shares_one_suffix_across_packages() {
    let dir = two_package_workspace_dir();
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    run_with(dir.path(), snapshot_args(Some("canary"), None)).unwrap();
    let version_a = manifest_version(dir.path(), "packages/a/package.json");
    let version_b = manifest_version(dir.path(), "packages/b/package.json");
    assert_eq!(version_a, version_b);
    assert!(
        version_a.starts_with("0.0.0-canary-"),
        "unexpected version: {version_a}"
    );
}

#[test]
fn snapshot_uses_the_config_template() {
    let dir = package_dir();
    write_config(
        dir.path(),
        "{ \"snapshot\": { \"prereleaseTemplate\": \"{tag}.{timestamp}\" } }\n",
    );
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_with(dir.path(), snapshot_args(Some("canary"), None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    let timestamp = version
        .strip_prefix("0.0.0-canary.")
        .unwrap_or_else(|| panic!("unexpected version: {version}"));
    assert_eq!(timestamp.len(), 13, "unexpected timestamp: {timestamp}");
    assert!(
        timestamp.chars().all(|c| c.is_ascii_digit()),
        "unexpected timestamp: {timestamp}"
    );
}

#[test]
fn snapshot_cli_template_overrides_the_config() {
    let dir = package_dir();
    write_config(
        dir.path(),
        "{ \"snapshot\": { \"prereleaseTemplate\": \"{tag}-config-{datetime}\" } }\n",
    );
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_with(
        dir.path(),
        snapshot_args(Some("canary"), Some("{tag}-cli-{datetime}")),
    )
    .unwrap();
    let version = manifest_version(dir.path(), "package.json");
    assert!(
        version.starts_with("0.0.0-canary-cli-"),
        "unexpected version: {version}"
    );
}

#[test]
fn snapshot_uses_the_calculated_version() {
    let dir = package_dir();
    write_config(
        dir.path(),
        "{ \"snapshot\": { \"useCalculatedVersion\": true } }\n",
    );
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    run_with(dir.path(), snapshot_args(None, None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    let suffix = version
        .strip_prefix("1.3.0-")
        .unwrap_or_else(|| panic!("unexpected version: {version}"));
    assert_datetime(suffix);
}

#[test]
fn snapshot_keeps_a_none_only_package_unchanged() {
    let dir = package_dir();
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "none")], "Note only");
    run_with(dir.path(), snapshot_args(None, None)).unwrap();
    assert_eq!(manifest_version(dir.path(), "package.json"), "1.2.3");
    assert!(!exists(dir.path(), "CHANGELOG.md"));
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn snapshot_fails_in_pre_mode() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = run_err_with(dir.path(), snapshot_args(None, None));
    assert!(err.contains("not allowed in pre mode"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn snapshot_after_exit_keeps_pre_json() {
    let dir = prerelease_package_dir("1.3.0-beta.1");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    run_with(dir.path(), snapshot_args(None, None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    assert!(
        version.starts_with("0.0.0-"),
        "unexpected version: {version}"
    );
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
    assert_eq!(read_pre_json(dir.path()), EXITED_PRE_JSON);
}

#[test]
fn snapshot_after_exit_rescues_to_a_snapshot_version() {
    let dir = prerelease_package_dir("1.3.0-beta.1");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    run_with(dir.path(), snapshot_args(None, None)).unwrap();
    let version = manifest_version(dir.path(), "package.json");
    let suffix = version
        .strip_prefix("0.0.0-")
        .unwrap_or_else(|| panic!("unexpected version: {version}"));
    assert_datetime(suffix);
    assert_eq!(read_pre_json(dir.path()), EXITED_PRE_JSON);
}

#[test]
fn snapshot_rejects_an_invalid_template_leaving_the_tree_untouched() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let err = run_err_with(
        dir.path(),
        snapshot_args(Some("canary"), Some("{datetime}")),
    );
    assert!(err.contains("{tag}"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn release_plan_reports_snapshot_versions() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let snapshot = Snapshot {
        tag: Some("canary".to_owned()),
        template: None,
    };
    let planned = plan_with(dir.path(), &[], Some(&snapshot)).unwrap();
    let release = &plan_json(&planned)["releases"][0];
    assert_eq!(release["oldVersion"], "1.2.3");
    let new_version = release["newVersion"].as_str().unwrap();
    assert!(
        new_version.starts_with("0.0.0-canary-"),
        "unexpected version: {new_version}"
    );
}
