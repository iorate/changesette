mod util;

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use util::{
    dir_snapshot, expected_path, package_dir, read_pre_json, two_package_workspace_dir,
    workspace_dir, write_changeset, write_config, write_pre_json,
};

const CHANGELOG: &str = "# ublacklist\n\n## 1.1.0\n\n### Minor Changes\n\n- Add feature\n\n## 1.0.0\n\n### Patch Changes\n\n- Fix bug\n";

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_changesette"));
    command
        .args(args)
        .current_dir(dir)
        .env_remove("CHANGESETTE_ROOT");
    command
}

fn changesette(dir: &Path, args: &[&str]) -> Output {
    command(dir, args).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn added_path(err: &str) -> &str {
    err.lines()
        .find_map(|line| line.strip_prefix("Added "))
        .unwrap_or_else(|| panic!("unexpected output: {err:?}"))
}

fn assert_changeset_path(line: &str) {
    let path = Path::new(line);
    assert!(path.is_absolute(), "unexpected path: {line}");
    assert_eq!(
        path.parent().and_then(Path::file_name),
        Some(".changeset".as_ref()),
        "unexpected path: {line}"
    );
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".md"))
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
fn init_creates_backfills_and_then_reports_initialized() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        format!(
            "Created {}\nCreated {}\n",
            expected_path(dir.path(), ".changeset/README.md"),
            expected_path(dir.path(), ".changeset/config.json")
        )
    );
    let readme = fs::read_to_string(dir.path().join(".changeset/README.md")).unwrap();
    assert!(readme.starts_with("# Changesets\n"), "{readme}");
    let config = fs::read_to_string(dir.path().join(".changeset/config.json")).unwrap();
    assert_eq!(
        config,
        "{\n  \"fixed\": [],\n  \"linked\": [],\n  \"privatePackages\": {\n    \"version\": false\n  },\n  \"ignore\": [],\n  \"snapshot\": {\n    \"useCalculatedVersion\": false\n  }\n}\n"
    );

    fs::write(dir.path().join(".changeset/README.md"), "custom\n").unwrap();
    fs::remove_file(dir.path().join(".changeset/config.json")).unwrap();
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "Created {}\n",
            expected_path(dir.path(), ".changeset/config.json")
        )
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".changeset/README.md")).unwrap(),
        "custom\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".changeset/config.json")).unwrap(),
        config
    );

    let before = dir_snapshot(dir.path());
    let output = changesette(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        format!(
            "{} is already initialized\n",
            expected_path(dir.path(), ".changeset")
        )
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[test]
fn init_creates_the_directory_at_the_workspace_root() {
    let dir = workspace_dir();
    let output = changesette(&dir.path().join("packages/a"), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "Created {}\nCreated {}\n",
            expected_path(dir.path(), ".changeset/README.md"),
            expected_path(dir.path(), ".changeset/config.json")
        )
    );
    assert!(dir.path().join(".changeset/README.md").is_file());
    assert!(!dir.path().join("packages/a/.changeset").exists());
}

#[test]
fn add_with_flags_creates_a_changeset() {
    let dir = package_dir();
    let output = changesette(
        dir.path(),
        &["add", "--minor", "ublacklist", "-m", "Add feature"],
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
    assert!(!dir.path().join(".changeset/README.md").exists());
    assert!(!dir.path().join(".changeset/config.json").exists());
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
fn add_accepts_comma_separated_and_repeated_package_flags() {
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

    let output = changesette(
        dir.path(),
        &[
            "add", "--patch", "pkg-c", "--patch", "pkg-b", "-m", "Fix bugs",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let content = fs::read_to_string(dir.path().join(added_path(&err))).unwrap();
    assert_eq!(
        content,
        "---\npkg-c: patch\npkg-b: patch\n---\n\nFix bugs\n"
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
    assert!(!dir.path().join(".changeset").exists());
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
fn get_changelog_entry_prints_the_requested_version() {
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.0.0"]);
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
fn get_changelog_entry_fails_without_a_changelog_file() {
    let dir = package_dir();
    let output = changesette(dir.path(), &["get-changelog-entry", "ublacklist", "1.0.0"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("CHANGELOG.md not found"));
}

#[test]
fn set_summary_rewrites_the_summary() {
    let dir = package_dir();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/brave-owls-run.md"),
        "---\nublacklist: minor\n---\n\nOld summary\n",
    )
    .unwrap();
    let output = changesette(
        dir.path(),
        &["set-summary", "brave-owls-run", "  New summary  "],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        format!(
            "Updated {}\n",
            expected_path(dir.path(), ".changeset/brave-owls-run.md")
        )
    );
    let content = fs::read_to_string(dir.path().join(".changeset/brave-owls-run.md")).unwrap();
    assert_eq!(content, "---\nublacklist: minor\n---\n\nNew summary\n");
}

#[test]
fn set_summary_fails_for_an_unknown_id() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        "known.md",
        &[("ublacklist", "minor")],
        "Summary",
    );
    let output = changesette(dir.path(), &["set-summary", "unknown", "New summary"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no changeset with id `unknown`"),
        "{}",
        stderr(&output)
    );
    let content = fs::read_to_string(dir.path().join(".changeset/known.md")).unwrap();
    assert_eq!(content, "---\n\"ublacklist\": minor\n---\n\nSummary\n");
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

fn mixed_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"1.0.0\",\n  \"private\": true\n}\n",
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
fn get_packages_lists_skipped_members_only_with_all() {
    let dir = mixed_workspace_dir();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"},{\"name\":\"pkg-c\",\"version\":\"2.0.0\",\"private\":false,\"dir\":\"packages/c\"}]\n"
    );

    let output = changesette(dir.path(), &["get-packages", "--all"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"},{\"name\":\"pkg-b\",\"version\":\"1.0.0\",\"private\":true,\"dir\":\"packages/b\"},{\"name\":\"pkg-c\",\"version\":\"2.0.0\",\"private\":false,\"dir\":\"packages/c\"}]\n"
    );
}

#[test]
fn get_packages_debug_reports_the_member_list() {
    let dir = workspace_dir();
    let output = changesette(dir.path(), &["get-packages", "--log-level", "debug"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains(&format!(
            "debug: workspace {} (npm): members: pkg-a (packages/a)",
            expected_path(dir.path(), "")
        )),
        "{err}"
    );
}

#[test]
fn get_packages_lists_nothing_without_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "[]\n");
    assert_eq!(stderr(&output), "");
}

const FILE_A: &str = "boldly-brave-otter.md";
const FILE_B: &str = "calmly-tidy-fox.md";
const ID_B: &str = "calmly-tidy-fox";

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
fn version_bumps_and_writes_the_changelog() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
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
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
    assert!(dir.path().join(".changeset/README.md").exists());
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
            "}}\n"
        ),
        id
    )
}

fn compact_plan(id: &str) -> String {
    format!(
        concat!(
            "{{\"changesets\":[{{\"id\":\"{0}\",\"summary\":\"Add feature\",",
            "\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"minor\"}}]}}],",
            "\"releases\":[{{\"name\":\"ublacklist\",\"type\":\"minor\",",
            "\"oldVersion\":\"1.2.3\",\"newVersion\":\"1.3.0\",",
            "\"changesets\":[\"{0}\"],",
            "\"changelogEntry\":\"### Minor Changes\\n\\n- Add feature\"}}]}}"
        ),
        id
    )
}

#[test]
fn version_output_writes_the_pretty_plan_and_applies_the_changesets() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
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
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn version_output_dash_writes_the_compact_plan_to_stdout() {
    let dir = package_dir();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let output = changesette(dir.path(), &["version", "--output", "-"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), compact_plan(ID_B) + "\n");
    assert_eq!(stderr(&output), "");
    assert!(!dir.path().join("-").exists());
    assert!(!dir.path().join(".changeset").join(FILE_B).exists());
}

#[test]
fn version_ignore_accepts_comma_separated_and_repeated_packages() {
    let cases: [&[&str]; 2] = [
        &["version", "--ignore", "pkg-a,pkg-b"],
        &["version", "--ignore", "pkg-a", "--ignore", "pkg-b"],
    ];
    for args in cases {
        let dir = two_package_workspace_dir();
        write_changeset(dir.path(), FILE_A, &[("pkg-a", "minor")], "Improve pkg-a");
        write_changeset(dir.path(), FILE_B, &[("pkg-b", "patch")], "Fix pkg-b");
        let before = dir_snapshot(dir.path());
        let output = changesette(dir.path(), args);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stdout(&output), "");
        assert_eq!(dir_snapshot(dir.path()), before, "{args:?}");
    }
}

#[test]
fn version_warns_on_an_unmatched_group_pattern() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"missing-*\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let output = changesette(dir.path(), &["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "warning: fixed: the package or glob \"missing-*\" does not match any package in the workspace\nBumped pkg-a 3.1.4 -> 3.1.5\n"
    );
}

#[test]
fn version_log_level_warn_keeps_warnings_and_drops_info() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"missing-*\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let output = changesette(dir.path(), &["version", "--log-level", "warn"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "warning: fixed: the package or glob \"missing-*\" does not match any package in the workspace\n"
    );
}

#[test]
fn version_log_level_error_drops_warnings_but_reports_failures() {
    let dir = two_package_workspace_dir();
    write_config(dir.path(), "{ \"fixed\": [[\"pkg-a\", \"missing-*\"]] }\n");
    write_changeset(dir.path(), FILE_A, &[("pkg-a", "patch")], "Fix pkg-a");
    let output = changesette(dir.path(), &["version", "--log-level", "error"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");

    let output = changesette(dir.path(), &["version", "--log-level", "error"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).starts_with("error: "),
        "{}",
        stderr(&output)
    );
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
    write_changeset(dir.path(), FILE_A, &[("pkg-b", "major")], "Rework");
    write_changeset(dir.path(), FILE_B, &[("pkg-a", "minor")], "Add feature");
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
        FILE_A,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    write_changeset(dir.path(), FILE_B, &[("ublacklist", "none")], "Note only");
    for flag in ["--verbose", "-v"] {
        let output = changesette(dir.path(), &["status", flag]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            stdout(&output),
            format!(
                "Packages to be bumped:\n- minor\n  - ublacklist -> 1.3.0\n    - .changeset/{FILE_A}\n    - .changeset/{FILE_B}\n"
            )
        );
    }
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

const PRE_JSON: &str = "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n";
const EXITED_PRE_JSON: &str = "{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n";

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
fn rejects_invalid_command_lines() {
    let cases: [&[&str]; 12] = [
        &["publish"],
        &["current"],
        &["changelog", "1.0.0"],
        &["add", "--bump", "minor", "-m", "Add feature"],
        &["add", "-b", "minor", "-m", "Add feature"],
        &[
            "add",
            "--empty",
            "--minor",
            "ublacklist",
            "-m",
            "Add feature",
        ],
        &["version", "--dry-run"],
        &["version", "-n"],
        &["get-changelog-entry", "ublacklist"],
        &["get-changelog-entry", "ublacklist", "1.0"],
        &["version", "--snapshot-prerelease-template", "{datetime}"],
        &["version", "--snapshot", ""],
    ];
    let dir = package_dir();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    write_changeset(
        dir.path(),
        FILE_B,
        &[("ublacklist", "minor")],
        "Add feature",
    );
    let before = dir_snapshot(dir.path());
    for args in cases {
        let output = changesette(dir.path(), args);
        assert!(!output.status.success(), "{args:?} should be rejected");
        assert_eq!(dir_snapshot(dir.path()), before, "{args:?}");
    }
}

#[test]
fn get_packages_warns_about_an_invalid_workspaces_type_under_yarn() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("yarn.lock"), "").unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": \"packages/*\" }\n",
    )
    .unwrap();
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"root\",\"version\":\"1.0.0\",\"private\":false,\"dir\":\".\"}]\n"
    );
    assert_eq!(
        stderr(&output),
        format!(
            "warning: {}: \"workspaces\" must be an array of strings or an object whose \"packages\" is an array of strings: ignored\n",
            expected_path(dir.path(), "package.json")
        )
    );
}

const PKG_A_JSON: &str =
    "[{\"name\":\"pkg-a\",\"version\":\"3.1.4\",\"private\":false,\"dir\":\"packages/a\"}]\n";

#[test]
fn root_option_takes_a_relative_directory() {
    let dir = workspace_dir();
    let cwd = dir.path().join("packages/a");
    let output = changesette(
        &cwd,
        &["add", "--patch", "pkg-a", "-m", "Fix", "--root", "../.."],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let path = added_path(&err);
    assert!(
        Path::new(path).starts_with(expected_path(dir.path(), ".changeset")),
        "{path}"
    );
    assert_changeset_path(path);
    assert!(Path::new(path).is_file());
}

#[test]
fn root_env_takes_the_directory() {
    let dir = workspace_dir();
    let other = tempfile::tempdir().unwrap();
    let output = command(other.path(), &["get-packages"])
        .env("CHANGESETTE_ROOT", dir.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), PKG_A_JSON);
}

#[test]
fn root_option_wins_over_the_env() {
    let dir = workspace_dir();
    let other = package_dir();
    let output = command(dir.path(), &["get-packages", "--root", "."])
        .env("CHANGESETTE_ROOT", other.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), PKG_A_JSON);
}

#[test]
fn an_empty_root_env_is_unset() {
    let dir = workspace_dir();
    let output = command(&dir.path().join("packages/a"), &["get-packages"])
        .env("CHANGESETTE_ROOT", "")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), PKG_A_JSON);
}

#[test]
fn root_option_rejects_a_missing_directory() {
    let dir = workspace_dir();
    let output = changesette(dir.path(), &["get-packages", "--root", "missing"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.starts_with("error: invalid root directory missing: "),
        "{err}"
    );
}

fn write_config_packages(dir: &Path, packages: &[&str]) {
    fs::create_dir_all(dir.join(".changeset")).unwrap();
    let list: Vec<String> = packages.iter().map(|dir| format!("\"{dir}\"")).collect();
    fs::write(
        dir.join(".changeset/config.json"),
        format!(
            "{{ \"changesette\": {{ \"packages\": [{}] }} }}\n",
            list.join(", ")
        ),
    )
    .unwrap();
}

#[test]
fn config_packages_round_trip_through_get_packages_all() {
    let dir = mixed_workspace_dir();
    let output = changesette(dir.path(), &["get-packages", "--all"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let expected = stdout(&output);
    let packages: Vec<serde_json::Value> = serde_json::from_str(&expected).unwrap();
    let dirs: Vec<&str> = packages
        .iter()
        .map(|package| package["dir"].as_str().unwrap())
        .collect();
    write_config_packages(dir.path(), &dirs);
    fs::write(dir.path().join("package.json"), "{}\n").unwrap();
    let output = changesette(dir.path(), &["get-packages", "--all"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), expected);
}

#[test]
fn config_packages_entry_errors_name_the_config() {
    let dir = workspace_dir();
    write_config_packages(dir.path(), &["/x"]);
    let output = changesette(dir.path(), &["get-packages"]);
    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        format!(
            "error: {}: invalid \"changesette.packages\" entry \"/x\": absolute paths are not supported\n",
            expected_path(dir.path(), ".changeset/config.json")
        )
    );
}
