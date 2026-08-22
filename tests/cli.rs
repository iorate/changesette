mod util;

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use util::{dir_snapshot, write_changeset, write_pre_changeset};

const CHANGELOG: &str = "# ublacklist\n\n## 1.1.0\n\n### Minor Changes\n\n- Add feature\n\n## 1.0.0\n\n### Patch Changes\n\n- Fix bug\n";

fn changesette(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_changesette"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn package_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n",
    )
    .unwrap();
    dir
}

fn added_path(err: &str) -> &str {
    err.lines()
        .find_map(|line| line.strip_prefix("Added "))
        .unwrap_or_else(|| panic!("unexpected output: {err:?}"))
}

fn assert_changeset_path(line: &str) {
    let name = line
        .strip_prefix(".changeset/")
        .and_then(|rest| rest.strip_suffix(".md"))
        .unwrap_or_else(|| panic!("unexpected path: {line}"));
    let words: Vec<&str> = name.split('-').collect();
    assert_eq!(words.len(), 3, "unexpected word count: {name}");
    assert!(
        words
            .iter()
            .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())),
        "unexpected name characters: {name}"
    );
}

#[test]
fn init_creates_the_changeset_directory_with_a_readme_and_a_config() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "Created .changeset/README.md\nCreated .changeset/config.json\n"
    );
    let readme = fs::read_to_string(dir.path().join(".changeset/README.md")).unwrap();
    assert!(readme.starts_with("# Changesets\n"), "{readme}");
    let config = fs::read_to_string(dir.path().join(".changeset/config.json")).unwrap();
    assert_eq!(
        config,
        "{\n  \"ignore\": [],\n  \"privatePackages\": {\n    \"version\": false\n  }\n}\n"
    );
}

#[test]
fn init_creates_the_directory_at_the_workspace_root() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "Created ../../.changeset/README.md\nCreated ../../.changeset/config.json\n"
    );
    assert!(dir.path().join(".changeset/README.md").is_file());
    assert!(!dir.path().join("packages/a/.changeset").exists());
}

#[test]
fn init_backfills_missing_files_into_an_existing_directory() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(dir.path().join(".changeset/README.md"), "custom\n").unwrap();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "Created .changeset/config.json\n");
    assert_eq!(
        fs::read_to_string(dir.path().join(".changeset/README.md")).unwrap(),
        "custom\n"
    );
    assert!(dir.path().join(".changeset/config.json").is_file());
}

#[test]
fn init_does_nothing_when_everything_exists() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn add_with_flags_creates_a_changeset() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--minor", "ublacklist", "--message", "Add feature"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    let line = added_path(&err);
    assert_changeset_path(line);
    let content = fs::read_to_string(dir.path().join(line)).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
    assert!(err.contains("Summary of changesets:"), "{err}");
    assert!(err.contains("minor:  ublacklist"), "{err}");
}

#[test]
fn add_quotes_a_scoped_package_name() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"@iorate/ublacklist\",\n  \"version\": \"1.2.3\"\n}\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &[
            "add",
            "--minor",
            "@iorate/ublacklist",
            "--message",
            "Add feature",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(
        content,
        "---\n\"@iorate/ublacklist\": minor\n---\n\nAdd feature\n"
    );
}

#[test]
fn add_is_the_default_command() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &["--minor", "ublacklist", "--message", "Add feature"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert_changeset_path(added_path(&err));
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
}

#[test]
fn add_creates_the_changeset_directory_when_missing() {
    let dir = package_dir();
    let output = changesette(
        dir.path(),
        &["add", "--minor", "ublacklist", "--message", "Add feature"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
    assert!(!dir.path().join(".changeset/README.md").exists());
    assert!(!dir.path().join(".changeset/config.json").exists());
}

#[test]
fn add_fails_on_an_invalid_config() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(dir.path().join(".changeset/config.json"), "").unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--minor", "ublacklist", "--message", "Add feature"],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("config.json"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn add_accepts_a_multi_line_message_via_the_short_flag() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--patch", "ublacklist", "-m", "line1\nline2"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\nublacklist: patch\n---\n\nline1\nline2\n");
}

#[test]
fn add_records_multiple_packages_in_flag_order() {
    let dir = workspace_dir();
    for (name, version) in [("pkg-b", "2.0.0"), ("pkg-c", "3.0.0")] {
        let member_dir = dir.path().join("packages").join(name);
        fs::create_dir_all(&member_dir).unwrap();
        fs::write(
            member_dir.join("package.json"),
            format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\"\n}}\n"),
        )
        .unwrap();
    }
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &[
            "add",
            "--minor",
            "pkg-a",
            "--patch",
            "pkg-c,pkg-b",
            "-m",
            "Improve things",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(
        content,
        "---\npkg-a: minor\npkg-c: patch\npkg-b: patch\n---\n\nImprove things\n"
    );
}

#[test]
fn add_accumulates_repeated_bump_flags() {
    let dir = workspace_dir();
    let member_dir = dir.path().join("packages/b");
    fs::create_dir_all(&member_dir).unwrap();
    fs::write(
        member_dir.join("package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &[
            "add", "--patch", "pkg-a", "--patch", "pkg-b", "-m", "Fix bugs",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(
        content,
        "---\npkg-a: patch\npkg-b: patch\n---\n\nFix bugs\n"
    );
}

#[test]
fn add_rejects_an_unknown_package_name() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--minor", "nope", "-m", "Add feature"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("`nope`"), "{err}");
    assert!(err.contains("--minor"), "{err}");
    assert_eq!(
        fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
        0
    );
}

#[test]
fn add_rejects_a_skipped_package_in_flags() {
    let dir = private_two_package_workspace_dir();
    let output = changesette(dir.path(), &["add", "--patch", "pkg-b", "-m", "Fix bug"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("`pkg-b`"), "{err}");
    assert!(err.contains("skipped"), "{err}");
    assert_eq!(
        fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
        0
    );
}

#[test]
fn add_accepts_a_private_package_when_the_config_versions_it() {
    let dir = private_two_package_workspace_dir();
    fs::create_dir_all(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/config.json"),
        "{ \"privatePackages\": { \"version\": true } }\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["add", "--patch", "pkg-b", "-m", "Fix bug"]);
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn add_fails_without_versionable_packages() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--patch", "ublacklist", "-m", "Fix bug"],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no versionable packages found"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn add_empty_fails_without_versionable_packages() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["add", "--empty", "-m", "Note"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no versionable packages found"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.path().join(".changeset").exists());
}

#[test]
fn add_rejects_a_package_passed_to_multiple_bump_flags() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &[
            "add",
            "--minor",
            "ublacklist",
            "--patch",
            "ublacklist",
            "-m",
            "Add feature",
        ],
    );
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--minor, --patch"), "{err}");
}

#[test]
fn add_without_message_fails_naming_the_missing_flag() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--minor", "ublacklist"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--message"), "{err}");
    assert!(!err.contains("--major/--minor/--patch"), "{err}");
}

#[test]
fn add_without_bump_flags_fails_naming_them() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "-m", "Add feature"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--major/--minor/--patch"), "{err}");
    assert!(!err.contains("--message"), "{err}");
}

#[test]
fn add_open_fails_in_non_interactive_mode() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["add", "--empty", "-m", "Note", "--open"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("cannot use --open in non-interactive mode"),
        "{err}"
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn add_without_any_flags_fails_naming_all_missing_flags() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--major/--minor/--patch, --message"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn add_accepts_an_empty_message() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--minor", "ublacklist", "-m", ""]);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n");
}

#[test]
fn add_rejects_the_removed_bump_flag() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    for flag in ["--bump", "-b"] {
        let output = changesette(dir.path(), &["add", flag, "minor", "-m", "Add feature"]);
        assert!(!output.status.success(), "{flag} should be rejected");
    }
}

#[test]
fn add_empty_creates_an_empty_changeset() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--empty"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert_changeset_path(added_path(&err));
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\n---\n");
    assert!(!err.contains("Summary of changesets:"), "{err}");
}

#[test]
fn add_empty_with_a_message_appends_the_summary() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--empty", "-m", "Note only"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(content, "---\n---\n\nNote only\n");
}

#[test]
fn add_empty_conflicts_with_bump_flags() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &[
            "add",
            "--empty",
            "--minor",
            "ublacklist",
            "-m",
            "Add feature",
        ],
    );
    assert!(!output.status.success());
    assert_eq!(
        fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
        0
    );
}

#[test]
fn add_with_a_major_bump_lists_it_in_the_summary() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--major", "ublacklist", "-m", "Rework everything"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("major:  ublacklist"), "{err}");
}

#[test]
fn add_from_a_subdirectory_targets_the_workspace_root() {
    let dir = workspace_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        &dir.path().join("packages/a"),
        &["add", "--minor", "pkg-a", "-m", "Add feature"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let line = added_path(&err);
    let rest = line
        .strip_prefix("../../.changeset/")
        .unwrap_or_else(|| panic!("unexpected path: {line}"));
    assert!(rest.ends_with(".md"), "{line}");
    let content = fs::read_to_string(dir.path().join("packages/a").join(line)).unwrap();
    assert_eq!(content, "---\npkg-a: minor\n---\n\nAdd feature\n");
}

#[test]
fn add_fails_in_a_memberless_workspace() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"workspaces\": []\n}\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--empty"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no packages found"),
        "{}",
        stderr(&output)
    );
}

fn workspace_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"workspaces\": [\"packages/*\"]\n}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("packages/a")).unwrap();
    fs::write(
        dir.path().join("packages/a/package.json"),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.1.4\"\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn get_changelog_entry_without_a_version_fails() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("<VERSION>"), "{}", stderr(&output));
}

#[test]
fn get_changelog_entry_prints_the_requested_version() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.0.0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "### Patch Changes\n\n- Fix bug\n");
}

#[test]
fn get_changelog_entry_reads_a_workspace_member_changelog() {
    let dir = workspace_dir();
    fs::write(dir.path().join("packages/a/CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "pkg-a", "1.0.0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "### Patch Changes\n\n- Fix bug\n");
}

#[test]
fn get_changelog_entry_fails_for_a_missing_version() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "9.9.9"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CHANGELOG.md: version 9.9.9 not found"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn get_changelog_entry_rejects_an_invalid_version() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.0"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("invalid value '1.0'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn get_changelog_entry_fails_without_a_changelog_file() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.0.0"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("CHANGELOG.md not found"));
}

#[test]
fn get_packages_prints_the_single_package_with_a_dot_dir() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"ublacklist\",\"version\":\"1.2.3\",\"private\":false,\"dir\":\".\"}]\n"
    );
}

#[test]
fn get_packages_lists_workspace_members_in_name_order() {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"},{\"name\":\"pkg-b\",\"version\":\"1.0.0\",\"private\":false,\"dir\":\"packages/b\"}]\n"
    );
}

#[test]
fn get_packages_keeps_dirs_relative_to_the_root_from_a_subdirectory() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"}]\n"
    );
}

#[test]
fn get_packages_renders_a_parent_directory_member_dir() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("ws")).unwrap();
    fs::write(
        dir.path().join("ws/pnpm-workspace.yaml"),
        "packages:\n  - \"../sibling/*\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("sibling/a")).unwrap();
    fs::write(
        dir.path().join("sibling/a/package.json"),
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
    )
    .unwrap();
    let output = changesette(&dir.path().join("ws"), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"1.0.0\",\"private\":false,\"dir\":\"../sibling/a\"}]\n"
    );
}

#[test]
fn version_skips_a_versionless_package_and_keeps_its_changeset() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\"\n}\n",
    )
    .unwrap();
    write_changeset(dir.path(), "a.md", &[("ublacklist", "patch")], "Fix bug");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "No packages to bump.\n");
    assert_eq!(dir_snapshot(dir.path()), before);
}

fn mixed_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("packages/c")).unwrap();
    fs::write(
        dir.path().join("packages/c/package.json"),
        "{\n  \"name\": \"pkg-c\",\n  \"version\": \"2.0.0\",\n  \"private\": false\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn get_packages_excludes_skipped_packages_by_default() {
    let dir = mixed_workspace_dir();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"},{\"name\":\"pkg-c\",\"version\":\"2.0.0\",\"private\":false,\"dir\":\"packages/c\"}]\n"
    );
}

#[test]
fn get_packages_all_lists_every_member_and_omits_a_missing_version() {
    let dir = mixed_workspace_dir();
    let output = changesette(dir.path(), &["get-packages", "--all"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"},{\"name\":\"pkg-b\",\"private\":true,\"dir\":\"packages/b\"},{\"name\":\"pkg-c\",\"version\":\"2.0.0\",\"private\":false,\"dir\":\"packages/c\"}]\n"
    );
}

#[test]
fn get_packages_includes_private_packages_when_the_config_versions_them() {
    let dir = private_two_package_workspace_dir();
    fs::create_dir_all(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/config.json"),
        "{ \"privatePackages\": { \"version\": true } }\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("pkg-b"), "{}", stdout(&output));
}

#[test]
fn get_packages_prints_an_empty_array_when_every_member_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "[]\n");
}

#[test]
fn get_packages_fails_on_an_invalid_config() {
    let dir = package_dir();
    fs::create_dir_all(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/config.json"),
        "{ \"privatePackages\": \"all\" }\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("privatePackages"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn get_packages_prints_an_empty_array_for_a_memberless_workspace() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"workspaces\": []\n}\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "[]\n");
}

#[test]
fn get_packages_fails_without_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(!output.status.success());
}

#[test]
fn the_old_current_subcommand_is_rejected() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["current"]);
    assert!(!output.status.success());
}

#[test]
fn the_old_changelog_subcommand_is_rejected() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["changelog", "1.0.0"]);
    assert!(!output.status.success());
}

const ULID_A: &str = "changesette-01H455VB4PEX5VSKNK084SN02Q.md";
const ULID_B: &str = "changesette-01H455WZ0H1X9PE0QB0MV1P1KG.md";
const ID_A: &str = "changesette-01H455VB4PEX5VSKNK084SN02Q";
const ID_B: &str = "changesette-01H455WZ0H1X9PE0QB0MV1P1KG";

#[test]
fn version_with_zero_changesets_fails_and_touches_nothing() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("no unreleased changesets found"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_treats_a_missing_changeset_directory_as_empty() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no unreleased changesets found"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.path().join(".changeset").exists());
}

#[test]
fn version_fails_on_an_invalid_config() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/config.json"),
        "{ \"ignore\": \"pkg\" }\n",
    )
    .unwrap();
    write_changeset(dir.path(), "a.md", &[("ublacklist", "patch")], "Fix bug");
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("config.json: \"ignore\" must be an array of strings"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn status_treats_a_missing_changeset_directory_as_empty() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Packages to be bumped:\n");
    assert!(!dir.path().join(".changeset").exists());
}

#[test]
fn version_allow_no_changesets_prints_a_notice_and_touches_nothing() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--allow-no-changesets"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "No unreleased changesets found.\n");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_output_with_zero_changesets_fails_without_writing_the_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["version", "--output", "plan.json"]);
    assert!(!output.status.success());
    assert!(!dir.path().join("plan.json").exists());
}

#[test]
fn version_allow_no_changesets_output_writes_an_empty_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["version", "-a", "--output", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("plan.json")).unwrap(),
        "{\n  \"changesets\": [],\n  \"releases\": []\n}"
    );
}

#[test]
fn version_bumps_and_writes_the_changelog() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    fs::write(dir.path().join(".changeset/README.md"), "# Changesets\n").unwrap();
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "Bumped ublacklist 1.2.3 -> 1.3.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.3.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
    assert!(dir.path().join(".changeset/README.md").exists());
}

#[test]
fn version_writes_an_empty_release_line_for_an_empty_summary() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "minor")], "");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- \n"
    );
}

#[test]
fn version_leaves_the_package_lock_untouched() {
    let dir = package_dir();
    let package_lock = "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"lockfileVersion\": 3,\n  \"packages\": {\n    \"\": {\n      \"name\": \"ublacklist\",\n      \"version\": \"1.2.3\"\n    }\n  }\n}\n";
    fs::write(dir.path().join("package-lock.json"), package_lock).unwrap();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.3.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("package-lock.json")).unwrap(),
        package_lock
    );
}

#[test]
fn version_uses_the_max_bump_across_changesets() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "major")],
        "Rework everything",
    );
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "patch")], "Fix bug");
    let output = changesette(dir.path(), &["version", "--output", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(plan.contains(ID_A), "{plan}");
    assert!(plan.contains(ID_B), "{plan}");
    assert!(plan.contains("\"newVersion\": \"2.0.0\""), "{plan}");
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## 2.0.0"), "{changelog}");
    assert!(changelog.contains("### Major Changes"), "{changelog}");
    assert!(changelog.contains("### Patch Changes"), "{changelog}");
}

#[test]
fn version_bumps_only_the_named_workspace_members() {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("packages/c")).unwrap();
    let untouched = "{\n  \"name\": \"pkg-c\",\n  \"version\": \"3.0.0\"\n}\n";
    fs::write(dir.path().join("packages/c/package.json"), untouched).unwrap();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("pkg-b", "patch"), ("pkg-a", "minor")],
        "Improve things",
    );
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "Bumped pkg-a 3.1.4 -> 3.2.0\nBumped pkg-b 2.0.0 -> 2.0.1\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/a/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.2.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/a/CHANGELOG.md")).unwrap(),
        "# pkg-a\n\n## 3.2.0\n\n### Minor Changes\n\n- Improve things\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.1\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/CHANGELOG.md")).unwrap(),
        "# pkg-b\n\n## 2.0.1\n\n### Patch Changes\n\n- Improve things\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/c/package.json")).unwrap(),
        untouched
    );
    assert!(!dir.path().join("packages/c/CHANGELOG.md").exists());
    assert!(!dir.path().join("CHANGELOG.md").exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_consumes_a_none_only_changeset_without_bumping() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "-o", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(plan.contains("\"type\": \"none\""), "{plan}");
    assert!(plan.contains("\"newVersion\": \"1.2.3\""), "{plan}");
    assert!(!plan.contains("changelogEntry"), "{plan}");
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        String::from_utf8(before["package.json"].clone()).unwrap()
    );
    assert!(!dir.path().join("CHANGELOG.md").exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_consumes_an_empty_changeset() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, &[], "");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert!(!dir.path().join("CHANGELOG.md").exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_fails_on_a_validation_error_leaving_the_tree_untouched() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset").join(ULID_B),
        "---\n\"other-package\": minor\n---\n\nAdd feature\n",
    )
    .unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_rerun_replaces_the_same_section() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let package_json = fs::read_to_string(dir.path().join("package.json")).unwrap();
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();

    fs::write(dir.path().join("package.json"), package_json).unwrap();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        changelog
    );
}

fn pretty_plan(id: &str) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"changesets\": [\n",
            "    {{\n",
            "      \"id\": \"{0}\",\n",
            "      \"summary\": \"Add feature\",\n",
            "      \"releases\": [\n",
            "        {{\n",
            "          \"name\": \"ublacklist\",\n",
            "          \"type\": \"minor\"\n",
            "        }}\n",
            "      ]\n",
            "    }}\n",
            "  ],\n",
            "  \"releases\": [\n",
            "    {{\n",
            "      \"name\": \"ublacklist\",\n",
            "      \"type\": \"minor\",\n",
            "      \"oldVersion\": \"1.2.3\",\n",
            "      \"newVersion\": \"1.3.0\",\n",
            "      \"changesets\": [\n",
            "        \"{0}\"\n",
            "      ],\n",
            "      \"changelogEntry\": \"### Minor Changes\\n\\n- Add feature\"\n",
            "    }}\n",
            "  ]\n",
            "}}"
        ),
        id
    )
}

#[test]
fn version_output_writes_the_pretty_plan_and_applies_the_changesets() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version", "--output", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("plan.json")).unwrap(),
        pretty_plan(ID_B)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.3.0\"\n}\n"
    );
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_output_dash_writes_the_pretty_plan_to_stdout() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version", "--output", "-"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), pretty_plan(ID_B) + "\n");
    assert_eq!(stderr(&output), "");
    assert!(!dir.path().join("-").exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

fn two_package_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();
    dir
}

fn write_config(dir: &Path, text: &str) {
    fs::create_dir_all(dir.join(".changeset")).unwrap();
    fs::write(dir.join(".changeset/config.json"), text).unwrap();
}

fn private_two_package_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn version_ignore_skips_the_package_and_keeps_its_changeset() {
    let dir = two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-b"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/a/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.2.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n"
    );
    assert!(!dir.path().join("packages/b/CHANGELOG.md").exists());
    assert!(!dir.path().join(".changeset").join(ULID_A).exists());
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_ignore_includes_the_skipped_changeset_in_the_plan_without_a_release() {
    let dir = two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(
        dir.path(),
        &["version", "--ignore", "pkg-b", "-o", "plan.json"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(plan.contains(ID_A), "{plan}");
    assert!(plan.contains(ID_B), "{plan}");
    assert!(plan.contains("pkg-b"), "{plan}");
    assert!(!plan.contains("2.0.0"), "{plan}");
}

#[test]
fn version_ignore_accepts_comma_separated_packages() {
    let dir = two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-a,pkg-b"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_rejects_an_unknown_package() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "other-package"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--ignore"), "{err}");
    assert!(err.contains("other-package"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_may_be_repeated() {
    let dir = two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let before = dir_snapshot(dir.path());
    let output = changesette(
        dir.path(),
        &["version", "--ignore", "pkg-a", "--ignore", "pkg-b"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_skips_a_none_only_changeset() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "ublacklist"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_rejects_a_mixed_changeset_with_a_none_release() {
    let dir = two_package_workspace_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("pkg-a", "minor"), ("pkg-b", "none")],
        "Improve things",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-a"]);
    assert!(!output.status.success());
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_rejects_a_mixed_changeset() {
    let dir = two_package_workspace_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("pkg-a", "minor"), ("pkg-b", "patch")],
        "Improve things",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-a"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains(ULID_B), "{err}");
    assert!(err.contains("pkg-a"), "{err}");
    assert!(err.contains("pkg-b"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_skips_a_config_ignored_package_and_keeps_its_changeset() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-b\"] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
    assert!(!dir.path().join(".changeset").join(ULID_A).exists());
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_resolves_config_ignore_globs_with_negation() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-*\", \"!pkg-a\"] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_rejects_the_ignore_flag_with_a_config_ignore() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-b\"] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-a"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("use only one of them"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_rejects_the_ignore_flag_when_the_config_ignore_matches_nothing() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"missing-*\"] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-b"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("use only one of them"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_ignore_flag_works_with_an_empty_config_ignore() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-b"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn status_omits_a_config_ignored_package() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-b\"] }\n");
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Packages to be bumped:\n- minor\n  - pkg-a\n"
    );
}

#[test]
fn add_rejects_a_config_ignored_package_in_flags() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-b\"] }\n");
    let output = changesette(dir.path(), &["add", "--patch", "pkg-b", "-m", "Fix bug"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("`pkg-b`"), "{err}");
    assert!(err.contains("skipped"), "{err}");
}

#[test]
fn get_packages_excludes_a_config_ignored_package() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"ignore\": [\"pkg-b\"] }\n");
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"}]\n"
    );
}

#[test]
fn version_skips_a_private_package_and_keeps_its_changeset() {
    let dir = private_two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\",\n  \"private\": true\n}\n"
    );
    assert!(!dir.path().join(".changeset").join(ULID_A).exists());
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_bumps_a_private_package_when_the_config_versions_it() {
    let dir = private_two_package_workspace_dir();
    fs::create_dir_all(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/config.json"),
        "{ \"privatePackages\": true }\n",
    )
    .unwrap();
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-b 2.0.0 -> 2.0.1\n");
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_rejects_a_changeset_mixing_skipped_and_not_skipped_packages() {
    let dir = private_two_package_workspace_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("pkg-a", "minor"), ("pkg-b", "patch")],
        "Improve things",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("cannot mix skipped packages"), "{err}");
    assert!(err.contains("`pkg-a`"), "{err}");
    assert!(err.contains("`pkg-b`"), "{err}");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_succeeds_when_every_changeset_is_skipped() {
    let dir = private_two_package_workspace_dir();
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "No packages to bump.\n");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_output_includes_a_skipped_changeset_without_a_release() {
    let dir = private_two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version", "--output", "-"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            concat!(
                "{{\n",
                "  \"changesets\": [\n",
                "    {{\n",
                "      \"id\": \"{0}\",\n",
                "      \"summary\": \"Improve pkg-a\",\n",
                "      \"releases\": [\n",
                "        {{\n",
                "          \"name\": \"pkg-a\",\n",
                "          \"type\": \"minor\"\n",
                "        }}\n",
                "      ]\n",
                "    }},\n",
                "    {{\n",
                "      \"id\": \"{1}\",\n",
                "      \"summary\": \"Fix pkg-b\",\n",
                "      \"releases\": [\n",
                "        {{\n",
                "          \"name\": \"pkg-b\",\n",
                "          \"type\": \"patch\"\n",
                "        }}\n",
                "      ]\n",
                "    }}\n",
                "  ],\n",
                "  \"releases\": [\n",
                "    {{\n",
                "      \"name\": \"pkg-a\",\n",
                "      \"type\": \"minor\",\n",
                "      \"oldVersion\": \"3.1.4\",\n",
                "      \"newVersion\": \"3.2.0\",\n",
                "      \"changesets\": [\n",
                "        \"{0}\"\n",
                "      ],\n",
                "      \"changelogEntry\": \"### Minor Changes\\n\\n- Improve pkg-a\"\n",
                "    }}\n",
                "  ]\n",
                "}}\n"
            ),
            ID_A, ID_B
        )
    );
    assert!(!dir.path().join(".changeset").join(ULID_A).exists());
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_tolerates_a_broken_manifest_in_an_unreleased_member() {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"next\"\n}\n",
    )
    .unwrap();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.1.4 -> 3.2.0\n");
}

#[test]
fn get_packages_treats_a_non_boolean_private_as_not_private() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\",\n  \"private\": \"true\"\n}\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"ublacklist\",\"version\":\"1.2.3\",\"private\":false,\"dir\":\".\"}]\n"
    );
}

#[test]
fn version_in_pre_mode_leaves_a_skipped_changeset_in_place() {
    let dir = private_two_package_workspace_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(dir.path().join(".changeset/pre").join(ULID_A).exists());
    assert!(dir.path().join(".changeset").join(ULID_B).exists());
    assert!(!dir.path().join(".changeset/pre").join(ULID_B).exists());
}

#[test]
fn version_after_exit_does_not_rescue_a_skipped_prerelease() {
    let dir = private_two_package_workspace_dir();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.1.0-beta.0\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.1.0-beta.0\",\n  \"private\": true\n}\n"
    );
    assert!(!dir.path().join(".changeset/pre.json").exists());
}

#[test]
fn status_omits_skipped_packages() {
    let dir = private_two_package_workspace_dir();
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Packages to be bumped:\n- minor\n  - pkg-a\n"
    );
}

#[test]
fn status_rejects_a_changeset_mixing_skipped_and_not_skipped_packages() {
    let dir = private_two_package_workspace_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("pkg-a", "minor"), ("pkg-b", "patch")],
        "Improve things",
    );
    let output = changesette(dir.path(), &["status"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot mix skipped packages"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn version_rejects_the_removed_dry_run_flag() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    for flag in ["--dry-run", "-n"] {
        let output = changesette(dir.path(), &["version", flag]);
        assert!(!output.status.success(), "{flag} should be rejected");
    }
}

#[test]
fn status_lists_packages_grouped_by_bump_without_modifying_files() {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();
    write_changeset(dir.path(), ULID_A, &[("pkg-b", "major")], "Rework");
    write_changeset(dir.path(), ULID_B, &[("pkg-a", "minor")], "Add feature");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Packages to be bumped:\n- major\n  - pkg-b\n- minor\n  - pkg-a\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn status_verbose_adds_versions_and_changeset_files() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    for flag in ["--verbose", "-v"] {
        let output = changesette(dir.path(), &["status", flag]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            stdout(&output),
            format!(
                "Packages to be bumped:\n- minor\n  - ublacklist -> 1.3.0\n    - .changeset/{ULID_A}\n    - .changeset/{ULID_B}\n"
            )
        );
    }
}

#[test]
fn status_omits_none_only_packages_from_the_listing() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Packages to be bumped:\n");
}

#[test]
fn status_with_zero_changesets_prints_only_the_heading() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Packages to be bumped:\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn status_output_writes_the_pretty_plan_without_modifying_files() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["status", "--output", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("plan.json")).unwrap(),
        pretty_plan(ID_B)
    );
    fs::remove_file(dir.path().join("plan.json")).unwrap();
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn status_output_dash_writes_the_pretty_plan_to_stdout() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["status", "-o", "-"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), pretty_plan(ID_B) + "\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn status_output_with_zero_changesets_writes_an_empty_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["status", "-o", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("plan.json")).unwrap(),
        "{\n  \"changesets\": [],\n  \"releases\": []\n}"
    );
}

const PRE_JSON: &str = "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n";
const EXITED_PRE_JSON: &str = "{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n";

fn write_pre_json(dir: &Path, text: &str) {
    let changeset_dir = dir.join(".changeset");
    fs::create_dir_all(&changeset_dir).unwrap();
    fs::write(changeset_dir.join("pre.json"), text).unwrap();
}

fn read_pre_json(dir: &Path) -> String {
    fs::read_to_string(dir.join(".changeset/pre.json")).unwrap()
}

fn prerelease_package_dir(version: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        format!("{{\n  \"name\": \"ublacklist\",\n  \"version\": \"{version}\"\n}}\n"),
    )
    .unwrap();
    dir
}

#[test]
fn pre_enter_creates_pre_json() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["pre", "enter", "beta"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "Entered pre mode with tag `beta`\nRun `changesette version` to bump to prerelease versions\n"
    );
    assert_eq!(read_pre_json(dir.path()), PRE_JSON);
}

#[test]
fn pre_enter_creates_pre_json_at_the_workspace_root() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["pre", "enter", "beta"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(read_pre_json(dir.path()), PRE_JSON);
    assert!(!dir.path().join("packages/a/.changeset").exists());
}

#[test]
fn pre_enter_accepts_a_dotted_tag() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["pre", "enter", "beta.2"]);
    assert!(output.status.success(), "{}", stderr(&output));
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
    let output = changesette(dir.path(), &["pre", "enter", "alpha"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("already in pre mode"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn pre_enter_after_exit_rewrites_in_place() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{ // pre state\n\t\"tag\":\t\"alpha\",\n\t\"mode\": \"exit\",\n\t\"someday\": [1, 2, 3]\n}",
    );
    let output = changesette(dir.path(), &["pre", "enter", "beta"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        read_pre_json(dir.path()),
        "{ // pre state\n\t\"tag\":\t\"beta\",\n\t\"mode\": \"pre\",\n\t\"someday\": [1, 2, 3]\n}"
    );
}

#[test]
fn pre_enter_rejects_an_invalid_tag() {
    for tag in ["", " ", "01", "beta 2"] {
        let dir = package_dir();
        let before = dir_snapshot(dir.path());
        let output = changesette(dir.path(), &["pre", "enter", tag]);
        assert!(!output.status.success(), "{tag:?} should be rejected");
        assert!(
            stderr(&output).contains("invalid pre tag"),
            "{}",
            stderr(&output)
        );
        assert_eq!(dir_snapshot(dir.path()), before);
    }
}

#[test]
fn pre_enter_fails_on_a_v2_pre_json() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"initialVersions\": {},\n  \"changesets\": []\n}\n",
    );
    let output = changesette(dir.path(), &["pre", "enter", "beta"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("changesets v2 format"), "{err}");
    assert!(err.contains("@changesets/cli@3"), "{err}");
}

#[test]
fn pre_exit_fails_without_pre_json() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["pre", "exit"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("not in pre mode"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn pre_exit_flips_the_mode() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    let output = changesette(dir.path(), &["pre", "exit"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "Exited pre mode\nRun `changesette version` to bump to final versions\n"
    );
    assert_eq!(read_pre_json(dir.path()), EXITED_PRE_JSON);
}

#[test]
fn pre_exit_is_idempotent() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    for _ in 0..2 {
        let output = changesette(dir.path(), &["pre", "exit"]);
        assert!(output.status.success(), "{}", stderr(&output));
    }
    assert_eq!(read_pre_json(dir.path()), EXITED_PRE_JSON);
}

#[test]
fn pre_exit_ignores_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    let output = changesette(dir.path(), &["pre", "exit"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        read_pre_json(dir.path()),
        "{\n  \"mode\": \"exit\",\n  \"tag\": \"not a tag\"\n}\n"
    );
}

#[test]
fn version_in_pre_mode_bumps_to_a_prerelease() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(err.contains("in pre mode with tag `beta`"), "{err}");
    assert!(
        err.contains("Bumped ublacklist 1.2.3 -> 1.3.0-beta.0\n"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.3.0-beta.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
    assert!(dir.path().join(".changeset/pre").join(ULID_B).is_file());
    assert_eq!(read_pre_json(dir.path()), PRE_JSON);
}

#[test]
fn version_in_pre_mode_increments_the_counter() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));

    write_changeset(dir.path(), ULID_B, &[("ublacklist", "patch")], "Fix bug");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Bumped ublacklist 1.3.0-beta.0 -> 1.3.0-beta.1\n"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0-beta.1\n\n### Patch Changes\n\n- Fix bug\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(dir.path().join(".changeset/pre").join(ULID_A).is_file());
    assert!(dir.path().join(".changeset/pre").join(ULID_B).is_file());
}

#[test]
fn version_in_pre_mode_fails_on_a_move_collision() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_pre_changeset(dir.path(), ULID_B, &[("ublacklist", "patch")], "Fix bug");
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("refusing to overwrite"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_in_pre_mode_rejects_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("invalid pre tag"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_in_pre_mode_leaves_ignored_changesets_in_place() {
    let dir = two_package_workspace_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(dir.path(), ULID_A, &[("pkg-a", "minor")], "Improve pkg-a");
    write_changeset(dir.path(), ULID_B, &[("pkg-b", "patch")], "Fix pkg-b");
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-b"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Bumped pkg-a 3.1.4 -> 3.2.0-beta.0\n"),
        "{}",
        stderr(&output)
    );
    assert!(dir.path().join(".changeset/pre").join(ULID_A).is_file());
    assert!(dir.path().join(".changeset").join(ULID_B).is_file());
    assert!(!dir.path().join(".changeset/pre").join(ULID_B).exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n"
    );
}

#[test]
fn version_in_pre_mode_keeps_a_none_only_package_unchanged() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n"
    );
    assert!(!dir.path().join("CHANGELOG.md").exists());
    assert!(dir.path().join(".changeset/pre").join(ULID_B).is_file());
}

#[test]
fn version_in_pre_mode_with_no_new_changesets_fails() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_pre_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no unreleased changesets found"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_after_exit_finalizes() {
    let dir = prerelease_package_dir("1.3.0-beta.0");
    fs::write(
        dir.path().join("CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
    )
    .unwrap();
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "patch")], "Fix bug");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "Bumped ublacklist 1.3.0-beta.0 -> 1.3.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.3.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- Add feature\n\n### Patch Changes\n\n- Fix bug\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(ULID_A).exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
    assert!(!dir.path().join(".changeset/pre.json").exists());
}

#[test]
fn version_after_exit_rescues_prerelease_packages() {
    let dir = two_package_workspace_dir();
    // The upstream rescues neither of these: `-beta.0` has the counter it
    // starts at, and `-alpha.2` is left over from an earlier tag.
    fs::write(
        dir.path().join("packages/a/package.json"),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.2.0-beta.0\"\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.1-alpha.2\"\n}\n",
    )
    .unwrap();
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let output = changesette(dir.path(), &["version", "--ignore", "pkg-b"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped pkg-a 3.2.0-beta.0 -> 3.2.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/a/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.2.0\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/a/CHANGELOG.md")).unwrap(),
        "# pkg-a\n\n## 3.2.0\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("packages/b/package.json")).unwrap(),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.1-alpha.2\"\n}\n"
    );
    assert!(!dir.path().join("packages/b/CHANGELOG.md").exists());
    assert!(!dir.path().join(".changeset/pre.json").exists());
}

#[test]
fn version_after_exit_rescues_a_none_only_package() {
    let dir = prerelease_package_dir("1.2.3-beta.1");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(dir.path(), ULID_B, &[("ublacklist", "none")], "Note only");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped ublacklist 1.2.3-beta.1 -> 1.2.3\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.2.3\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(ULID_B).exists());
    assert!(!dir.path().join(".changeset/pre.json").exists());
}

#[test]
fn version_after_exit_succeeds_with_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["pre", "exit"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped ublacklist 1.2.3 -> 1.3.0\n");
    assert!(!dir.path().join(".changeset/pre.json").exists());
}

#[test]
fn version_after_exit_with_no_changesets_still_deletes_pre_json() {
    let dir = package_dir();
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert!(!dir.path().join(".changeset/pre.json").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n"
    );
}

#[test]
fn version_consumes_pre_changesets_without_pre_json() {
    let dir = package_dir();
    write_pre_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), ULID_B, &[("ublacklist", "patch")], "Fix bug");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "Bumped ublacklist 1.2.3 -> 1.3.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- Add feature\n\n### Patch Changes\n\n- Fix bug\n"
    );
    assert!(!dir.path().join(".changeset/pre").join(ULID_A).exists());
    assert!(!dir.path().join(".changeset").join(ULID_B).exists());
}

#[test]
fn version_output_omits_pre_state_without_pre_json() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version", "-o", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(!plan.contains("preState"), "{plan}");
}

#[test]
fn status_in_pre_mode_shows_prerelease_versions() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_pre_changeset(dir.path(), ULID_A, &[("ublacklist", "major")], "Rework");
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["status", "--verbose"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Packages to be bumped:\n- minor\n  - ublacklist -> 1.3.0-beta.0\n    - .changeset/{ULID_B}\n"
        )
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn status_in_pre_mode_rejects_an_invalid_tag() {
    let dir = package_dir();
    write_pre_json(
        dir.path(),
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"not a tag\"\n}\n",
    );
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["status"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("invalid pre tag"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn status_output_in_pre_mode_includes_pre_state() {
    let dir = package_dir();
    write_pre_json(dir.path(), PRE_JSON);
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["status", "-o", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(
        plan.contains("\"preState\": {\n    \"mode\": \"pre\",\n    \"tag\": \"beta\"\n  }"),
        "{plan}"
    );
    assert!(plan.contains("\"newVersion\": \"1.3.0-beta.0\""), "{plan}");
}

#[test]
fn status_output_includes_pre_state_and_prefixed_ids() {
    let dir = prerelease_package_dir("1.3.0-beta.0");
    write_pre_json(dir.path(), EXITED_PRE_JSON);
    write_pre_changeset(
        dir.path(),
        ULID_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["status", "-o", "plan.json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let plan = fs::read_to_string(dir.path().join("plan.json")).unwrap();
    assert!(
        plan.contains("\"preState\": {\n    \"mode\": \"exit\",\n    \"tag\": \"beta\"\n  }"),
        "{plan}"
    );
    assert!(plan.contains(&format!("\"id\": \"pre/{ID_A}\"")), "{plan}");
    assert!(plan.contains(&format!("\"pre/{ID_A}\"")), "{plan}");
    assert!(plan.contains("\"newVersion\": \"1.3.0\""), "{plan}");
}

#[test]
fn get_changelog_entry_reads_a_prerelease_section() {
    let dir = package_dir();
    fs::write(
        dir.path().join("CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
    )
    .unwrap();
    let output = changesette(
        dir.path(),
        &["get-changelog-entry", "ublacklist", "1.3.0-beta.0"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "### Minor Changes\n\n- Add feature\n");
}

#[test]
fn get_changelog_entry_returns_an_empty_rescued_section() {
    let dir = package_dir();
    fs::write(
        dir.path().join("CHANGELOG.md"),
        "# ublacklist\n\n## 1.3.0\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.3.0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "\n");
}

#[test]
fn prints_the_crate_version() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["--version"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        format!("changesette {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn prints_help() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["--help"]);
    assert!(output.status.success());
    let out = stdout(&output);
    for subcommand in [
        "init",
        "add",
        "version",
        "pre",
        "status",
        "get-packages",
        "get-changelog-entry",
    ] {
        assert!(out.contains(subcommand), "{out}");
    }
}

#[test]
fn rejects_an_unknown_subcommand() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["publish"]);
    assert!(!output.status.success());
}
