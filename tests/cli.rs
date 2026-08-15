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
fn add_with_both_flags_creates_a_changeset() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(
        dir.path(),
        &["add", "--bump", "minor", "--message", "Add feature"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    let line = out.strip_suffix('\n').unwrap();
    assert!(
        !line.contains('\n'),
        "stdout must be only the path: {out:?}"
    );
    assert_changeset_path(line);
    assert!(dir.path().join(".changeset").is_dir());
    let content = fs::read_to_string(dir.path().join(line)).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nAdd feature\n");
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
        &["add", "--bump", "minor", "--message", "Add feature"],
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
    let output = changesette(dir.path(), &["--bump", "minor", "--message", "Add feature"]);
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
        &["add", "--bump", "minor", "--message", "Add feature"],
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
        &["add", "--bump", "patch", "-m", "line1\nline2"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
    assert_eq!(content, "---\nublacklist: patch\n---\n\nline1\nline2\n");
}

#[test]
fn add_without_message_fails_naming_the_missing_flag() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--bump", "minor"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--message"), "{err}");
    assert!(!err.contains("--bump"), "{err}");
}

#[test]
fn add_without_bump_fails_naming_the_missing_flag() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "-m", "Add feature"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--bump"), "{err}");
    assert!(!err.contains("--message"), "{err}");
}

#[test]
fn add_without_any_flags_fails_naming_both_missing_flags() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--bump, --message"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn add_rejects_an_empty_message() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["add", "--bump", "minor", "-m", ""]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read_dir(dir.path().join(".changeset")).unwrap().count(),
        0
    );
}

#[test]
fn add_rejects_an_unknown_bump_type() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["add", "--bump", "huge", "-m", "Add feature"]);
    assert!(!output.status.success());
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
fn get_version_prints_the_version() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-version", "ublacklist"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.2.3\n");
}

#[test]
fn get_version_without_a_package_fails() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-version"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("<PACKAGE>"), "{}", stderr(&output));
}

#[test]
fn get_version_fails_for_an_unknown_package() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-version", "other"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("`other`"), "{}", stderr(&output));
}

#[test]
fn get_version_fails_without_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["get-version", "ublacklist"]);
    assert!(!output.status.success());
}

#[test]
fn get_version_resolves_a_workspace_member() {
    let dir = workspace_dir();
    let output = changesette(dir.path(), &["get-version", "pkg-a"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "3.1.4\n");
}

#[test]
fn get_version_resolves_the_workspace_root_from_a_subdirectory() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["get-version", "pkg-a"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "3.1.4\n");
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

#[test]
fn version_with_zero_changesets_does_nothing() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
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
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    fs::write(dir.path().join(".changeset/README.md"), "# Changesets\n").unwrap();
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
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
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("package-lock.json")).unwrap(),
        package_lock
    );
}

#[test]
fn version_uses_the_max_bump_across_changesets() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_A, "major", "Rework everything");
    write_changeset(dir.path(), ULID_B, "patch", "Fix bug");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "2.0.0\n");
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## 2.0.0"), "{changelog}");
    assert!(changelog.contains("### Major Changes"), "{changelog}");
    assert!(changelog.contains("### Patch Changes"), "{changelog}");
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
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    let package_json = fs::read_to_string(dir.path().join("package.json")).unwrap();
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();

    fs::write(dir.path().join("package.json"), package_json).unwrap();
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap(),
        changelog
    );
}

#[test]
fn version_dry_run_prints_the_plan_without_modifying_files() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert_eq!(dir_snapshot(dir.path()), before);
    assert_eq!(
        stderr(&output),
        format!(
            "dry run: no files will be modified\n\
             would consume 1 changeset:\n\
             \x20\x20.changeset/{ULID_B} (minor)\n\
             would update package.json: 1.2.3 -> 1.3.0\n\
             would insert into CHANGELOG.md:\n\
             \n\
             ## 1.3.0\n\
             \n\
             ### Minor Changes\n\
             \n\
             - Add feature\n"
        )
    );
}

#[test]
fn version_dry_run_via_the_short_flag_consumes_multiple_changesets() {
    let dir = package_dir();
    write_changeset(dir.path(), ULID_A, "patch", "Fix bug");
    write_changeset(dir.path(), ULID_B, "minor", "Add feature");
    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["version", "-n"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert_eq!(dir_snapshot(dir.path()), before);
    let err = stderr(&output);
    assert!(err.contains("would consume 2 changesets:"), "{err}");
    assert!(
        err.contains("would update package.json: 1.2.3 -> 1.3.0"),
        "{err}"
    );
}

#[test]
fn version_dry_run_with_zero_changesets_behaves_like_a_normal_run() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    let output = changesette(dir.path(), &["version", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("no changesets found"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("dry run"), "{}", stderr(&output));
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
        "get-version",
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
