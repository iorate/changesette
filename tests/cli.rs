mod util;

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use util::{dir_snapshot, write_changeset};

const CROCKFORD_ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

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

fn assert_changeset_path(line: &str) {
    let ulid = line
        .strip_prefix(".changeset/changesette-")
        .and_then(|rest| rest.strip_suffix(".md"))
        .unwrap_or_else(|| panic!("unexpected path: {line}"));
    assert_eq!(ulid.len(), 26, "unexpected ULID length: {ulid}");
    assert!(
        ulid.chars().all(|c| CROCKFORD_ALPHABET.contains(c)),
        "unexpected ULID characters: {ulid}"
    );
}

#[test]
fn init_creates_the_changeset_directory_with_a_readme() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    let readme = fs::read_to_string(dir.path().join(".changeset/README.md")).unwrap();
    assert!(readme.starts_with("# Changesets\n"), "{readme}");
}

#[test]
fn init_does_nothing_when_the_directory_exists() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert_eq!(dir_snapshot(dir.path()), before);
    assert!(!dir.path().join(".changeset/README.md").exists());
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
    let out = stdout(&output);
    let line = out.strip_suffix('\n').unwrap();
    assert!(
        !line.contains('\n'),
        "stdout must be only the path: {out:?}"
    );
    assert_changeset_path(line);
    let content = fs::read_to_string(dir.path().join(line)).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
    let err = stderr(&output);
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
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
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
    let out = stdout(&output);
    assert_changeset_path(out.trim_end());
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
}

#[test]
fn add_fails_without_the_changeset_directory() {
    let dir = package_dir();
    let output = changesette(
        dir.path(),
        &["add", "--minor", "ublacklist", "--message", "Add feature"],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("changesette init"),
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
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
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
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
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
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
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
fn add_rejects_an_empty_message() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--minor", "ublacklist", "-m", ""]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
        0
    );
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
    let out = stdout(&output);
    assert_changeset_path(out.trim_end());
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
    assert_eq!(content, "---\n---\n");
    assert!(
        !stderr(&output).contains("Summary of changesets:"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn add_empty_with_a_message_appends_the_summary() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--empty", "-m", "Note only"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
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
    let out = stdout(&output);
    let line = out.trim_end();
    let rest = line
        .strip_prefix("../../.changeset/changesette-")
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
        "[{\"name\":\"ublacklist\",\"version\":\"1.2.3\",\"dir\":\".\"}]\n"
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
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"dir\":\"packages/a\"},{\"name\":\"pkg-b\",\"version\":\"1.0.0\",\"dir\":\"packages/b\"}]\n"
    );
}

#[test]
fn get_packages_keeps_dirs_relative_to_the_root_from_a_subdirectory() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"dir\":\"packages/a\"}]\n"
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
const EMPTY_PLAN: &str = "{\"changesets\":[],\"releases\":[]}\n";

#[test]
fn version_with_zero_changesets_prints_an_empty_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), EMPTY_PLAN);
    assert!(
        stderr(&output).contains("no changesets found"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn version_fails_without_the_changeset_directory() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains(".changeset"),
        "{}",
        stderr(&output)
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
    assert_eq!(
        stdout(&output),
        format!(
            "{{\"changesets\":[{{\"id\":\"{ID_B}\",\"summary\":\"Add feature\",\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"minor\"}}]}}],\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"minor\",\"oldVersion\":\"1.2.3\",\"newVersion\":\"1.3.0\",\"changesets\":[\"{ID_B}\"]}}]}}\n"
        )
    );
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
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{{\"changesets\":[{{\"id\":\"{ID_A}\",\"summary\":\"Rework everything\",\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"major\"}}]}},{{\"id\":\"{ID_B}\",\"summary\":\"Fix bug\",\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"patch\"}}]}}],\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"major\",\"oldVersion\":\"1.2.3\",\"newVersion\":\"2.0.0\",\"changesets\":[\"{ID_A}\",\"{ID_B}\"]}}]}}\n"
        )
    );
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
        stdout(&output),
        format!(
            "{{\"changesets\":[{{\"id\":\"{ID_B}\",\"summary\":\"Improve things\",\"releases\":[{{\"name\":\"pkg-b\",\"type\":\"patch\"}},{{\"name\":\"pkg-a\",\"type\":\"minor\"}}]}}],\"releases\":[{{\"name\":\"pkg-a\",\"type\":\"minor\",\"oldVersion\":\"3.1.4\",\"newVersion\":\"3.2.0\",\"changesets\":[\"{ID_B}\"]}},{{\"name\":\"pkg-b\",\"type\":\"patch\",\"oldVersion\":\"2.0.0\",\"newVersion\":\"2.0.1\",\"changesets\":[\"{ID_B}\"]}}]}}\n"
        )
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
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{{\"changesets\":[{{\"id\":\"{ID_B}\",\"summary\":\"Note only\",\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"none\"}}]}}],\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"none\",\"oldVersion\":\"1.2.3\",\"newVersion\":\"1.2.3\",\"changesets\":[\"{ID_B}\"]}}]}}\n"
        )
    );
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
    assert_eq!(
        stdout(&output),
        format!(
            "{{\"changesets\":[{{\"id\":\"{ID_B}\",\"summary\":\"\",\"releases\":[]}}],\"releases\":[]}}\n"
        )
    );
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

#[test]
fn version_dry_run_prints_the_same_plan_without_modifying_files() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let dry = changesette(dir.path(), &["version", "--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert_eq!(dir_snapshot(dir.path()), before);
    assert_eq!(stderr(&dry), "dry run: no files will be modified\n");

    let real = changesette(dir.path(), &["version"]);
    assert!(real.status.success(), "{}", stderr(&real));
    assert_eq!(stdout(&dry), stdout(&real));
}

#[test]
fn version_dry_run_via_the_short_flag_leaves_multiple_changesets_in_place() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_A, &[("ublacklist", "patch")], "Fix bug");
    write_changeset(
        dir.path(),
        ULID_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "-n"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(dir_snapshot(dir.path()), before);
    let out = stdout(&output);
    assert!(
        out.contains("\"oldVersion\":\"1.2.3\",\"newVersion\":\"1.3.0\""),
        "{out}"
    );
}

#[test]
fn version_dry_run_with_zero_changesets_prints_an_empty_plan() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["version", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), EMPTY_PLAN);
    assert!(
        stderr(&output).contains("no changesets found"),
        "{}",
        stderr(&output)
    );
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
