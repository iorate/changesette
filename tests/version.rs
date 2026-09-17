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

fn pkg(name: &str, version: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\"\n}}\n")
}

fn private_pkg(name: &str, version: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"private\": true\n}}\n")
}

fn dependent_pkg(name: &str, version: &str, field: &str, dependency: &str, spec: &str) -> String {
    format!(
        "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"{field}\": {{\n    \"{dependency}\": \"{spec}\"\n  }}\n}}\n"
    )
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn args() -> VersionArgs {
    VersionArgs {
        snapshot: None,
        snapshot_prerelease_template: None,
        allow_no_changesets: false,
        output: None,
    }
}

fn snapshot_args(tag: Option<&str>, template: Option<&str>) -> VersionArgs {
    VersionArgs {
        snapshot: Some(tag.map(str::to_owned)),
        snapshot_prerelease_template: template.map(str::to_owned),
        ..args()
    }
}

fn load_with(dir: &Path, ignore: &[&str]) -> Result<(Workspace, Config)> {
    changesette::load(dir, None, &owned(ignore))
}

fn load(dir: &Path) -> (Workspace, Config) {
    load_with(dir, &[]).unwrap()
}

fn run_with(dir: &Path, ignore: &[&str], args: VersionArgs) -> Result<()> {
    let (workspace, config) = load_with(dir, ignore)?;
    version::run(workspace, &config, args)
}

fn run_ok(dir: &Path) {
    run_with(dir, &[], args()).unwrap();
}

fn run_err_with(dir: &Path, ignore: &[&str], args: VersionArgs) -> String {
    format!("{:#}", run_with(dir, ignore, args).unwrap_err())
}

fn run_err(dir: &Path) -> String {
    run_err_with(dir, &[], args())
}

fn plan_with(dir: &Path, ignore: &[&str], snapshot: Option<&Snapshot>) -> Result<PlannedVersion> {
    let (workspace, config) = load_with(dir, ignore)?;
    plan::plan_version(workspace, &config, snapshot)
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
    serde_json::to_value(release_plan::build(planned)).unwrap()
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
        &[],
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
        &[],
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
        &[],
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
}

#[test]
fn fails_for_a_changeset_naming_a_versionless_package_leaving_the_tree_untouched() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{\n  \"name\": \"ublacklist\"\n}\n",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    let before = dir_snapshot(dir.path());
    let err = run_err(dir.path());
    assert!(err.contains(FILE_B), "{err}");
    assert!(err.contains("`ublacklist` has no version"), "{err}");
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

        run_with(dir.path(), ignore, args()).unwrap();
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
                    "dir": "packages/a",
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
    let err = run_err_with(dir.path(), &["other-package"], args());
    assert!(err.contains("--ignore"), "{err}");
    assert!(err.contains("`other-package` not found"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn ignore_rejects_a_duplicated_name() {
    let dir = workspace_dir();
    write_file(
        dir.path(),
        "packages/b/package.json",
        &pkg("pkg-a", "1.0.0"),
    );
    let err = format!("{:#}", load_with(dir.path(), &["pkg-a"]).unwrap_err());
    assert!(err.contains("--ignore"), "{err}");
    assert!(
        err.contains("`pkg-a` is ambiguous: used by packages/a, packages/b"),
        "{err}"
    );
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

        run_with(dir.path(), ignore, args()).unwrap();
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
        let err = run_err_with(dir.path(), ignore, args());
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
        let err = run_err_with(dir.path(), &[ignore], args());
        assert!(err.contains("--ignore"), "{err}");
        assert!(err.contains("use only one of them"), "{err}");
        assert_eq!(dir_snapshot(dir.path()), before);
    }

    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    run_with(dir.path(), &["pkg-b"], args()).unwrap();
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
    let (workspace, _) = load(dir.path());
    assert!(workspace.package("pkg-a").unwrap().versionable().is_some());
    assert!(workspace.package("pkg-b").unwrap().versionable().is_none());

    write_config(
        dir.path(),
        "{ \"privatePackages\": { \"version\": true } }\n",
    );
    let (workspace, _) = load(dir.path());
    assert!(workspace.package("pkg-b").unwrap().versionable().is_some());

    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        private_pkg("pkg-b", "2.0.1")
    );
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
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
            "dir": "packages/b",
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
        run_with(dir.path(), ignore, args()).unwrap();
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

    run_with(dir.path(), &["pkg-b"], args()).unwrap();
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
                    "dir": ".",
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
    run_with(dir.path(), &[], snapshot_args(None, None)).unwrap();
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
    run_with(dir.path(), &[], snapshot_args(Some("canary"), None)).unwrap();
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
    run_with(dir.path(), &[], snapshot_args(Some("canary"), None)).unwrap();
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
    run_with(dir.path(), &[], snapshot_args(Some("canary"), None)).unwrap();
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
        &[],
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
    run_with(dir.path(), &[], snapshot_args(None, None)).unwrap();
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
    run_with(dir.path(), &[], snapshot_args(None, None)).unwrap();
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
    let err = run_err_with(dir.path(), &[], snapshot_args(None, None));
    assert!(err.contains("not allowed in pre mode"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn snapshot_after_exit_keeps_pre_json() {
    let dir = prerelease_package_dir("1.3.0-beta.1");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "patch")], "Fix bug");
    run_with(dir.path(), &[], snapshot_args(None, None)).unwrap();
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
    run_with(dir.path(), &[], snapshot_args(None, None)).unwrap();
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
        &[],
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

#[test]
fn dependents_are_bumped_when_the_next_version_leaves_the_range() {
    let bumps = [
        ("none", "3.1.4"),
        ("patch", "3.1.5"),
        ("minor", "3.2.0"),
        ("major", "4.0.0"),
    ];
    let specs: [(&str, &[&str]); 8] = [
        ("^3.1.4", &["major"]),
        ("~3.1.4", &["minor", "major"]),
        ("3.1.4", &["patch", "minor", "major"]),
        ("*", &[]),
        ("workspace:^", &["major"]),
        ("workspace:~", &["minor", "major"]),
        ("workspace:*", &["patch", "minor", "major"]),
        ("workspace:^3.1.4", &["major"]),
    ];
    for field in [
        "dependencies",
        "peerDependencies",
        "optionalDependencies",
        "devDependencies",
    ] {
        for (spec, bumping) in specs {
            for (bump, next) in bumps {
                let dir = workspace_dir();
                write_file(
                    dir.path(),
                    "packages/b/package.json",
                    &dependent_pkg("pkg-b", "2.0.0", field, "pkg-a", spec),
                );
                write_changeset(dir.path(), FILE_A, &[("pkg-a", bump)], "Change");
                let mut expected = vec![format!("pkg-a {bump} 3.1.4 -> {next}")];
                if field != "devDependencies" && bumping.contains(&bump) {
                    expected.push("pkg-b patch 2.0.0 -> 2.0.1".to_owned());
                }
                assert_eq!(
                    releases(&plan(dir.path())),
                    expected,
                    "{field} {spec} {bump}"
                );
            }
        }
    }
}

#[test]
fn dependents_are_bumped_transitively_listing_the_updated_dependencies() {
    let dir = workspace_dir();
    let b_manifest = |version, spec| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
    let d_manifest = |spec| dependent_pkg("pkg-d", "1.0.0", "dependencies", "pkg-c", spec);
    write_file(
        dir.path(),
        "packages/b/package.json",
        &b_manifest("2.0.0", "^3.1.4"),
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-b", "workspace:*"),
    );
    write_file(dir.path(), "packages/d/package.json", &d_manifest("^1.0.0"));
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "major")], "Break pkg-a");
    let planned = plan(dir.path());
    assert_eq!(
        releases(&planned),
        [
            "pkg-a major 3.1.4 -> 4.0.0",
            "pkg-b patch 2.0.0 -> 2.0.1",
            "pkg-c patch 1.0.0 -> 1.0.1"
        ]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("2.0.1", "^4.0.0")
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Updated dependencies\n  - pkg-a@4.0.0\n"
    );
    assert_eq!(
        read(dir.path(), "packages/c/CHANGELOG.md"),
        "# pkg-c\n\n## 1.0.1\n\n### Patch Changes\n\n- Updated dependencies\n  - pkg-b@2.0.1\n"
    );
    assert_eq!(
        read(dir.path(), "packages/d/package.json"),
        d_manifest("^1.0.1")
    );
    assert!(!exists(dir.path(), "packages/d/CHANGELOG.md"));
}

#[test]
fn mutually_dependent_packages_bump_each_other_once() {
    let dir = workspace_dir();
    let a_manifest = |version, spec| dependent_pkg("pkg-a", version, "dependencies", "pkg-b", spec);
    let b_manifest = |version, spec| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
    write_file(
        dir.path(),
        "packages/a/package.json",
        &a_manifest("3.1.4", "^2.0.0"),
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        &b_manifest("2.0.0", "^3.1.4"),
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "major")], "Break pkg-a");
    assert_eq!(
        releases(&plan(dir.path())),
        ["pkg-a major 3.1.4 -> 4.0.0", "pkg-b patch 2.0.0 -> 2.0.1"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/a/package.json"),
        a_manifest("4.0.0", "^2.0.1")
    );
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("2.0.1", "^4.0.0")
    );
    assert_eq!(
        read(dir.path(), "packages/a/CHANGELOG.md"),
        "# pkg-a\n\n## 4.0.0\n\n### Major Changes\n\n- Break pkg-a\n\n### Patch Changes\n\n- Updated dependencies\n  - pkg-b@2.0.1\n"
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Updated dependencies\n  - pkg-a@4.0.0\n"
    );
}

#[test]
fn a_dependent_keeps_its_own_higher_bump() {
    let dir = workspace_dir();
    let b_manifest = |version, spec| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
    write_file(
        dir.path(),
        "packages/b/package.json",
        &b_manifest("2.0.0", "^3.1.4"),
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "major")], "Break pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "minor")], "Improve pkg-b");
    assert_eq!(
        releases(&plan(dir.path())),
        ["pkg-a major 3.1.4 -> 4.0.0", "pkg-b minor 2.0.0 -> 2.1.0"]
    );

    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("2.1.0", "^4.0.0")
    );
    assert_eq!(
        read(dir.path(), "packages/b/CHANGELOG.md"),
        "# pkg-b\n\n## 2.1.0\n\n### Minor Changes\n\n- Improve pkg-b\n\n### Patch Changes\n\n- Updated dependencies\n  - pkg-a@4.0.0\n"
    );
}

#[test]
fn a_fixed_partner_bumps_its_dependents() {
    let dir = two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-b", "^2.0.0"),
    );
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    assert_eq!(
        releases(&plan(dir.path())),
        [
            "pkg-a patch 3.1.4 -> 3.1.5",
            "pkg-b patch 3.1.4 -> 3.1.5",
            "pkg-c patch 1.0.0 -> 1.0.1"
        ]
    );
}

#[test]
fn a_dependent_raised_by_its_linked_group_bumps_its_own_dependents() {
    let dir = two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/b/package.json",
        &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-x", "1.0.0"),
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-b", "~2.0.0"),
    );
    write_file(
        dir.path(),
        "packages/x/package.json",
        &pkg("pkg-x", "1.0.0"),
    );
    write_config(dir.path(), "{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-x", "patch")], "Fix pkg-x");
    assert_eq!(
        releases(&plan(dir.path())),
        [
            "pkg-a minor 3.1.4 -> 3.2.0",
            "pkg-b minor 3.1.4 -> 3.2.0",
            "pkg-c patch 1.0.0 -> 1.0.1",
            "pkg-x patch 1.0.0 -> 1.0.1"
        ]
    );
}

#[test]
fn pre_mode_bumps_the_dependents_whose_range_excludes_the_prerelease() {
    for spec in ["^3.1.4", "*"] {
        let dir = workspace_dir();
        write_file(
            dir.path(),
            "packages/b/package.json",
            &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", spec),
        );
        write_pre_json(dir.path(), PRE_JSON);
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
        assert_eq!(
            releases(&plan(dir.path())),
            [
                "pkg-a patch 3.1.4 -> 3.1.5-beta.0",
                "pkg-b patch 2.0.0 -> 2.0.1-beta.0"
            ],
            "{spec}"
        );
    }
}

#[test]
fn after_exit_dependents_join_the_rescued_packages() {
    let dir = workspace_dir();
    write_file(
        dir.path(),
        "packages/a/package.json",
        &pkg("pkg-a", "3.1.5-beta.1"),
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-c", "^1.0.0"),
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        &pkg("pkg-c", "1.0.0"),
    );
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_changeset(dir.path(), FILE_A, &[("pkg-c", "major")], "Break pkg-c");
    assert_eq!(
        releases(&plan(dir.path())),
        [
            "pkg-a patch 3.1.5-beta.1 -> 3.1.5",
            "pkg-b patch 2.0.0 -> 2.0.1",
            "pkg-c major 1.0.0 -> 2.0.0"
        ]
    );
}

#[test]
fn snapshot_judges_dependents_by_the_plain_next_version() {
    for (spec, bumped) in [("~3.1.4", true), ("^3.1.4", false)] {
        let dir = workspace_dir();
        write_file(
            dir.path(),
            "packages/b/package.json",
            &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", spec),
        );
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
        let snapshot = Snapshot {
            tag: Some("canary".to_owned()),
            template: None,
        };
        let planned = plan_with(dir.path(), &[], Some(&snapshot)).unwrap();
        let names: Vec<&str> = planned
            .releases
            .iter()
            .map(|release| release.name.as_str())
            .collect();
        let expected: &[&str] = if bumped {
            &["pkg-a", "pkg-b"]
        } else {
            &["pkg-a"]
        };
        assert_eq!(names, expected, "{spec}");
        for release in &planned.releases {
            let new_version = release.new_version.to_string();
            assert!(
                new_version.starts_with("0.0.0-canary-"),
                "{spec}: {new_version}"
            );
        }
    }
}

#[test]
fn skipped_dependents_are_not_released() {
    let dir = workspace_dir();
    write_file(
        dir.path(),
        "packages/b/package.json",
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\",\n  \"private\": true,\n  \"dependencies\": {\n    \"pkg-a\": \"3.1.4\"\n  }\n}\n",
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-a", "3.1.4"),
    );
    write_file(
        dir.path(),
        "packages/d/package.json",
        "{\n  \"name\": \"pkg-d\",\n  \"dependencies\": {\n    \"pkg-a\": \"3.1.4\"\n  }\n}\n",
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let planned = plan_with(dir.path(), &["pkg-c"], None).unwrap();
    assert_eq!(releases(&planned), ["pkg-a patch 3.1.4 -> 3.1.5"]);
}

#[test]
fn manage_internal_dependencies_false_leaves_the_dependents_alone() {
    let dir = workspace_dir();
    let b_manifest = dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", "3.1.4");
    write_file(dir.path(), "packages/b/package.json", &b_manifest);
    write_config(
        dir.path(),
        "{ \"changesette\": { \"manageInternalDependencies\": false } }\n",
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    assert_eq!(releases(&plan(dir.path())), ["pkg-a patch 3.1.4 -> 3.1.5"]);
    run_ok(dir.path());
    assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
}

#[test]
fn workspace_protocol_only_bumps_only_the_workspace_dependents() {
    let dir = workspace_dir();
    let b_manifest = dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", "3.1.4");
    let d_manifest = |spec| dependent_pkg("pkg-d", "1.0.0", "dependencies", "pkg-a", spec);
    write_file(dir.path(), "packages/b/package.json", &b_manifest);
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-a", "workspace:*"),
    );
    write_file(
        dir.path(),
        "packages/d/package.json",
        &d_manifest("workspace:^3.1.4"),
    );
    write_config(
        dir.path(),
        "{ \"bumpVersionsWithWorkspaceProtocolOnly\": true }\n",
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    assert_eq!(
        releases(&plan(dir.path())),
        ["pkg-a patch 3.1.4 -> 3.1.5", "pkg-c patch 1.0.0 -> 1.0.1"]
    );
    run_ok(dir.path());
    assert_eq!(read(dir.path(), "packages/b/package.json"), b_manifest);
    assert_eq!(
        read(dir.path(), "packages/d/package.json"),
        d_manifest("workspace:^3.1.5")
    );
}

#[test]
fn release_plan_reports_a_dependent_release() {
    let dir = workspace_dir();
    write_file(
        dir.path(),
        "packages/b/package.json",
        &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", "3.1.4"),
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    assert_eq!(
        plan_json(&plan(dir.path()))["releases"][1],
        json!({
            "name": "pkg-b",
            "type": "patch",
            "oldVersion": "2.0.0",
            "newVersion": "2.0.1",
            "changesets": [],
            "dir": "packages/b",
            "changelogEntry": "### Patch Changes\n\n- Updated dependencies\n  - pkg-a@3.1.5"
        })
    );

    write_changeset(dir.path(), FILE_B, &[("pkg-b", "none")], "Note pkg-b");
    let release = &plan_json(&plan(dir.path()))["releases"][1];
    assert_eq!(release["type"], "patch");
    assert_eq!(release["changesets"], json!([ID_B]));
    assert_eq!(
        release["changelogEntry"],
        "### Patch Changes\n\n- Updated dependencies\n  - pkg-a@3.1.5"
    );
}

#[test]
fn only_dependencies_and_peer_dependencies_are_listed_as_updated() {
    for (field, listed) in [
        ("dependencies", true),
        ("peerDependencies", true),
        ("optionalDependencies", false),
        ("devDependencies", false),
    ] {
        let dir = workspace_dir();
        let b_manifest = |version, spec| dependent_pkg("pkg-b", version, field, "pkg-a", spec);
        write_file(
            dir.path(),
            "packages/b/package.json",
            &b_manifest("2.0.0", "3.1.4"),
        );
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
        write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
        let planned = plan(dir.path());
        let release = &planned.releases[1];
        let updated: Vec<String> = release
            .updated_dependencies
            .iter()
            .map(|(name, version)| format!("{name}@{version}"))
            .collect();
        let expected_entry = if listed {
            "### Patch Changes\n\n- Fix pkg-b\n\n- Updated dependencies\n  - pkg-a@3.1.5"
        } else {
            "### Patch Changes\n\n- Fix pkg-b"
        };
        assert_eq!(
            updated,
            if listed { vec!["pkg-a@3.1.5"] } else { vec![] },
            "{field}"
        );
        assert_eq!(
            release.changelog_entry.as_deref(),
            Some(expected_entry),
            "{field}"
        );
        run_ok(dir.path());
        assert_eq!(
            read(dir.path(), "packages/b/package.json"),
            b_manifest("2.0.1", "3.1.5"),
            "{field}"
        );
    }
}

#[test]
fn kept_ranges_are_listed_as_updated_dependencies() {
    for (spec, min, listed) in [
        ("workspace:*", "patch", true),
        ("*", "patch", true),
        ("*", "minor", false),
        ("workspace:^3.1.4", "minor", true),
        ("^3.1.4", "minor", false),
    ] {
        let dir = workspace_dir();
        let b_manifest = |version| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
        write_file(dir.path(), "packages/b/package.json", &b_manifest("2.0.0"));
        write_config(
            dir.path(),
            &format!("{{ \"updateInternalDependencies\": \"{min}\" }}\n"),
        );
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
        write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
        let output = capture_output(|| run_ok(dir.path()));
        assert!(!output.contains("Updated pkg-b"), "{spec} {min}: {output}");
        assert_eq!(
            read(dir.path(), "packages/b/package.json"),
            b_manifest("2.0.1"),
            "{spec} {min}"
        );
        let expected = if listed {
            "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Fix pkg-b\n\n- Updated dependencies\n  - pkg-a@3.1.5\n"
        } else {
            "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Fix pkg-b\n"
        };
        assert_eq!(
            read(dir.path(), "packages/b/CHANGELOG.md"),
            expected,
            "{spec} {min}"
        );
    }
}

#[test]
fn ranges_are_raised_in_unreleased_dependents_and_the_unnamed_root() {
    let dir = workspace_dir();
    let root_manifest = |spec| {
        format!(
            "{{\n  \"workspaces\": [\"packages/*\"],\n  \"dependencies\": {{\n    \"pkg-a\": \"{spec}\"\n  }}\n}}\n"
        )
    };
    let b_manifest = |spec| dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", spec);
    write_file(dir.path(), "package.json", &root_manifest("^3.1.4"));
    write_file(dir.path(), "packages/b/package.json", &b_manifest("~3.1.4"));
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let output = capture_output(|| run_ok(dir.path()));
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.starts_with("debug: "))
        .collect();
    assert_eq!(
        lines,
        [
            "Bumped pkg-a 3.1.4 -> 3.1.5",
            "Updated <unnamed> (.): pkg-a ^3.1.4 -> ^3.1.5",
            "Updated pkg-b (packages/b): pkg-a ~3.1.4 -> ~3.1.5",
        ],
        "{output}"
    );
    assert_eq!(read(dir.path(), "package.json"), root_manifest("^3.1.5"));
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("~3.1.5")
    );
    assert!(!exists(dir.path(), "packages/b/CHANGELOG.md"));
}

#[test]
fn packages_rewritten_without_a_bump_are_none_releases() {
    let dir = workspace_dir();
    write_file(
        dir.path(),
        "package.json",
        "{\n  \"workspaces\": [\"packages/*\"],\n  \"dependencies\": {\n    \"pkg-a\": \"^3.1.4\"\n  }\n}\n",
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        &dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", "~3.1.4"),
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-c", "1.0.0", "dependencies", "pkg-a", "workspace:^"),
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let root_release = json!({
        "type": "none",
        "changesets": [],
        "dir": "."
    });
    let plan_value = plan_json(&plan(dir.path()));
    assert_eq!(plan_value["releases"][0], root_release);
    assert_eq!(plan_value["releases"][1]["name"], "pkg-a");
    assert_eq!(
        plan_value["releases"][2],
        json!({
            "name": "pkg-b",
            "type": "none",
            "oldVersion": "2.0.0",
            "newVersion": "2.0.0",
            "changesets": [],
            "dir": "packages/b"
        })
    );
    assert_eq!(plan_value["releases"].as_array().unwrap().len(), 3);

    write_changeset(dir.path(), FILE_B, &[("pkg-b", "none")], "Note pkg-b");
    let plan_value = plan_json(&plan(dir.path()));
    assert_eq!(plan_value["releases"][2]["type"], "none");
    assert_eq!(plan_value["releases"][2]["changesets"], json!([ID_B]));
    assert_eq!(plan_value["releases"].as_array().unwrap().len(), 3);
}

#[test]
fn dependency_updates_are_ordered_by_dependent_dir_then_dependency_dir() {
    let dir = two_package_workspace_dir();
    write_file(
        dir.path(),
        "packages/c/package.json",
        &dependent_pkg("pkg-y", "1.0.0", "dependencies", "pkg-a", "^3.1.4"),
    );
    write_file(
        dir.path(),
        "packages/d/package.json",
        "{\n  \"name\": \"pkg-x\",\n  \"version\": \"1.0.0\",\n  \"dependencies\": {\n    \"pkg-b\": \"^2.0.0\",\n    \"pkg-a\": \"^3.1.4\"\n  }\n}\n",
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = capture_output(|| run_ok(dir.path()));
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.starts_with("debug: "))
        .collect();
    assert_eq!(
        lines,
        [
            "Bumped pkg-a 3.1.4 -> 3.1.5",
            "Bumped pkg-b 2.0.0 -> 2.0.1",
            "Updated pkg-y (packages/c): pkg-a ^3.1.4 -> ^3.1.5",
            "Updated pkg-x (packages/d): pkg-a ^3.1.4 -> ^3.1.5",
            "Updated pkg-x (packages/d): pkg-b ^2.0.0 -> ^2.0.1",
        ],
        "{output}"
    );
}

#[test]
fn a_released_dependent_gets_its_version_and_ranges_in_one_write() {
    let dir = workspace_dir();
    let b_manifest = |version, spec| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
    write_file(
        dir.path(),
        "packages/b/package.json",
        &b_manifest("2.0.0", "3.1.4"),
    );
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let planned = plan(dir.path());
    let writes = plan::stage_writes(
        &planned.workspace,
        &planned.releases,
        &planned.dependency_updates,
    )
    .unwrap();
    let mut paths: Vec<String> = writes
        .iter()
        .map(|write| write.path.display().to_string())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            expected_path(dir.path(), "packages/a/CHANGELOG.md"),
            expected_path(dir.path(), "packages/a/package.json"),
            expected_path(dir.path(), "packages/b/CHANGELOG.md"),
            expected_path(dir.path(), "packages/b/package.json"),
        ]
    );
    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("2.0.1", "3.1.5")
    );
}

#[test]
fn pre_mode_pins_any_and_keeps_workspace_aliases() {
    let dir = workspace_dir();
    let b_manifest = |version, spec| dependent_pkg("pkg-b", version, "dependencies", "pkg-a", spec);
    let c_manifest =
        |version| dependent_pkg("pkg-c", version, "dependencies", "pkg-a", "workspace:*");
    write_file(
        dir.path(),
        "packages/b/package.json",
        &b_manifest("2.0.0", "*"),
    );
    write_file(dir.path(), "packages/c/package.json", &c_manifest("1.0.0"));
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    run_ok(dir.path());
    assert_eq!(
        read(dir.path(), "packages/b/package.json"),
        b_manifest("2.0.1-beta.0", "3.1.5-beta.0")
    );
    assert_eq!(
        read(dir.path(), "packages/c/package.json"),
        c_manifest("1.0.1-beta.0")
    );
}

#[test]
fn update_internal_dependencies_minor_keeps_the_range_on_a_patch_bump() {
    for (bump, expected) in [("patch", "^3.1.4"), ("minor", "^3.2.0")] {
        let dir = workspace_dir();
        let b_manifest = |spec| dependent_pkg("pkg-b", "2.0.0", "dependencies", "pkg-a", spec);
        write_file(dir.path(), "packages/b/package.json", &b_manifest("^3.1.4"));
        write_config(
            dir.path(),
            "{ \"updateInternalDependencies\": \"minor\" }\n",
        );
        write_changeset(dir.path(), FILE_A, &[("pkg-a", bump)], "Change pkg-a");
        run_ok(dir.path());
        assert_eq!(
            read(dir.path(), "packages/b/package.json"),
            b_manifest(expected),
            "{bump}"
        );
    }
}
