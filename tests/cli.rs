use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;

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
fn add_with_both_flags_creates_a_changeset() {
    let dir = package_dir();
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
    assert_eq!(content, "---\n\"ublacklist\": minor\n---\n\nAdd feature\n");
}

#[test]
fn add_accepts_a_multi_line_message_via_the_short_flag() {
    let dir = package_dir();
    let output = changesette(
        dir.path(),
        &["add", "--bump", "patch", "-m", "line1\nline2"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    let content = fs::read_to_string(dir.path().join(out.trim_end())).unwrap();
    assert_eq!(content, "---\n\"ublacklist\": patch\n---\n\nline1\nline2\n");
}

#[test]
fn add_without_message_fails_naming_the_missing_flag() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["add", "--bump", "minor"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--message"), "{err}");
    assert!(!err.contains("--bump"), "{err}");
}

#[test]
fn add_without_bump_fails_naming_the_missing_flag() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["add", "-m", "Add feature"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("--bump"), "{err}");
    assert!(!err.contains("--message"), "{err}");
}

#[test]
fn add_without_any_flags_fails_naming_both_missing_flags() {
    let dir = package_dir();
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
    let output = changesette(dir.path(), &["add", "--bump", "minor", "-m", ""]);
    assert!(!output.status.success());
    assert!(fs::read_dir(dir.path().join(".changeset")).is_err());
}

#[test]
fn add_rejects_an_unknown_bump_type() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["add", "--bump", "huge", "-m", "Add feature"]);
    assert!(!output.status.success());
}

#[test]
fn current_prints_the_version() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["current"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.2.3\n");
}

#[test]
fn current_fails_without_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["current"]);
    assert!(!output.status.success());
}

#[test]
fn changelog_prints_the_latest_section_by_default() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["changelog"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "### Minor Changes\n\n- Add feature\n");
}

#[test]
fn changelog_prints_the_requested_version() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["changelog", "1.0.0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "### Patch Changes\n\n- Fix bug\n");
}

#[test]
fn changelog_fails_for_a_missing_version() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["changelog", "9.9.9"]);
    assert!(!output.status.success());
}

#[test]
fn changelog_fails_without_a_changelog_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["changelog"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("CHANGELOG.md not found"));
}

#[test]
fn version_is_not_implemented_yet() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["version"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("not implemented"));
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
    for subcommand in ["add", "version", "current", "changelog"] {
        assert!(out.contains(subcommand), "{out}");
    }
}

#[test]
fn rejects_an_unknown_subcommand() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["publish"]);
    assert!(!output.status.success());
}
