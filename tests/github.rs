mod util;

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::json;
use tempfile::TempDir;
use util::{dir_snapshot, write_changeset};
use wiremock::{
    Mock, MockBuilder, MockServer, Request, ResponseTemplate,
    matchers::{bearer_token, method, path, query_param, query_param_is_missing},
};

const CHANGESET: &str = "changesette-01H455WZ0H1X9PE0QB0MV1P1KG.md";
const OTHER_CHANGESET: &str = "changesette-01H455VB4PEX5VSKNK084SN02Q.md";
const MERGED_AT: &str = "2026-01-01T00:00:00Z";

fn integration_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join(".changeset")).unwrap();
    fs::write(
        dir.path().join(".changeset/changesette.toml"),
        "[github]\nrepo = \"o/r\"\n",
    )
    .unwrap();
    dir
}

fn run_version(dir: &Path, server_uri: &str, token: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_changesette"));
    command
        .arg("version")
        .args(args)
        .current_dir(dir)
        .env("GITHUB_API_URL", server_uri)
        .env_remove("GITHUB_TOKEN");
    if let Some(token) = token {
        command.env("GITHUB_TOKEN", token);
    }
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn commits_mock(file_name: &str) -> MockBuilder {
    Mock::given(method("GET"))
        .and(path("/repos/o/r/commits"))
        .and(query_param("path", format!(".changeset/{file_name}")))
        .and(query_param("per_page", "100"))
}

fn pulls_mock(sha: &str) -> MockBuilder {
    Mock::given(method("GET")).and(path(format!("/repos/o/r/commits/{sha}/pulls")))
}

fn commits_response(shas: &[&str]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        shas.iter()
            .map(|sha| json!({ "sha": sha }))
            .collect::<Vec<_>>(),
    )
}

fn pulls_response(pulls: &[(u64, Option<&str>)]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        pulls
            .iter()
            .map(|(number, merged_at)| json!({ "number": number, "merged_at": merged_at }))
            .collect::<Vec<_>>(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_deduped_ascending_pr_links_across_commits() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .respond_with(commits_response(&["sha1", "sha2"]))
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .respond_with(pulls_response(&[
            (23, Some(MERGED_AT)),
            (12, Some(MERGED_AT)),
        ]))
        .mount(&server)
        .await;
    pulls_mock("sha2")
        .respond_with(pulls_response(&[
            (34, Some(MERGED_AT)),
            (12, Some(MERGED_AT)),
        ]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    insta::assert_snapshot!(fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn renders_a_linkless_entry_for_prless_commits_and_unmerged_prs() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .respond_with(commits_response(&["sha1", "sha2"]))
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .respond_with(pulls_response(&[]))
        .mount(&server)
        .await;
    pulls_mock("sha2")
        .respond_with(pulls_response(&[(5, None)]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("- Add feature"), "{changelog}");
    assert!(!changelog.contains("pull/"), "{changelog}");
}

#[tokio::test(flavor = "multi_thread")]
async fn degrades_with_a_warning_when_no_commits_are_found() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .respond_with(commits_response(&[]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert!(
        stderr(&output).contains(&format!(
            "warning: no commits found for .changeset/{CHANGESET}; generating the entry without \
             PR links"
        )),
        "{}",
        stderr(&output)
    );
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(!changelog.contains("pull/"), "{changelog}");
}

#[tokio::test(flavor = "multi_thread")]
async fn accesses_anonymously_without_a_token() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    let no_authorization = |request: &Request| !request.headers.contains_key("authorization");
    commits_mock(CHANGESET)
        .and(no_authorization)
        .respond_with(commits_response(&["sha1"]))
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .and(no_authorization)
        .respond_with(pulls_response(&[(12, Some(MERGED_AT))]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), None, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output)
            .contains("note: GITHUB_TOKEN is not set; accessing the GitHub API anonymously"),
        "{}",
        stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_the_token_as_a_bearer_header() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .and(bearer_token("test-token"))
        .respond_with(commits_response(&["sha1"]))
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .and(bearer_token("test-token"))
        .respond_with(pulls_response(&[(12, Some(MERGED_AT))]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("anonymously"),
        "{}",
        stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_on_an_api_error_without_touching_any_file() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({ "message": "rate limit exceeded" })),
        )
        .mount(&server)
        .await;
    let before = dir_snapshot(dir.path());

    let output = run_version(dir.path(), &server.uri(), None, &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("403"), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("GITHUB_TOKEN"),
        "{}",
        stderr(&output)
    );
    assert_eq!(dir_snapshot(dir.path()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_link_header_pagination() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .and(query_param_is_missing("page"))
        .respond_with(commits_response(&["sha1"]).append_header(
            "Link",
            format!(
                "<{}/repos/o/r/commits?path=.changeset/{CHANGESET}&per_page=100&page=2>; \
                 rel=\"next\"",
                server.uri()
            ),
        ))
        .expect(1)
        .mount(&server)
        .await;
    commits_mock(CHANGESET)
        .and(query_param("page", "2"))
        .respond_with(commits_response(&["sha2"]))
        .expect(1)
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .respond_with(pulls_response(&[(1, Some(MERGED_AT))]))
        .mount(&server)
        .await;
    pulls_mock("sha2")
        .respond_with(pulls_response(&[(2, Some(MERGED_AT))]))
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("pull/1)"), "{changelog}");
    assert!(changelog.contains("pull/2)"), "{changelog}");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn memoizes_pull_lookups_per_commit() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), OTHER_CHANGESET, "patch", "Fix bug");
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(OTHER_CHANGESET)
        .respond_with(commits_response(&["shared"]))
        .mount(&server)
        .await;
    commits_mock(CHANGESET)
        .respond_with(commits_response(&["shared"]))
        .mount(&server)
        .await;
    pulls_mock("shared")
        .respond_with(pulls_response(&[(7, Some(MERGED_AT))]))
        .expect(1)
        .mount(&server)
        .await;

    let output = run_version(dir.path(), &server.uri(), Some("test-token"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_calls_the_api_but_modifies_nothing() {
    let server = MockServer::start().await;
    let dir = integration_dir();
    write_changeset(dir.path(), CHANGESET, "minor", "Add feature");
    commits_mock(CHANGESET)
        .respond_with(commits_response(&["sha1", "sha2"]))
        .expect(1)
        .mount(&server)
        .await;
    pulls_mock("sha1")
        .respond_with(pulls_response(&[
            (23, Some(MERGED_AT)),
            (12, Some(MERGED_AT)),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    pulls_mock("sha2")
        .respond_with(pulls_response(&[
            (34, Some(MERGED_AT)),
            (12, Some(MERGED_AT)),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    let before = dir_snapshot(dir.path());

    let output = run_version(
        dir.path(),
        &server.uri(),
        Some("test-token"),
        &["--dry-run"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1.3.0\n");
    assert_eq!(dir_snapshot(dir.path()), before);
    let err = stderr(&output);
    assert!(err.contains("would insert into CHANGELOG.md:"), "{err}");
    assert!(
        err.contains(
            "- [#12](https://github.com/o/r/pull/12) [#23](https://github.com/o/r/pull/23) \
             [#34](https://github.com/o/r/pull/34) - Add feature"
        ),
        "{err}"
    );
    server.verify().await;
}
