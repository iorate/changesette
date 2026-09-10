use std::fs;

use changesette::pre::{PreJson, PreMode, validate_tag};
use tempfile::TempDir;

fn changeset_dir(pre_json: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("pre.json"), pre_json).unwrap();
    dir
}

fn load(pre_json: &str) -> PreJson {
    PreJson::load(changeset_dir(pre_json).path())
        .unwrap()
        .unwrap()
}

#[test]
fn loads_pre_and_exit_modes() {
    let pre = load("{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n");
    assert_eq!(pre.mode(), PreMode::Pre);
    assert_eq!(pre.tag(), "beta");
    assert!(pre.path().ends_with("pre.json"));
    let pre = load("{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n");
    assert_eq!(pre.mode(), PreMode::Exit);
}

#[test]
fn returns_none_without_pre_json() {
    let dir = tempfile::tempdir().unwrap();
    assert!(PreJson::load(dir.path()).unwrap().is_none());
}

#[test]
fn rejects_the_v2_format() {
    for pre_json in [
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"initialVersions\": {\n    \"pkg-a\": \"1.0.0\"\n  }\n}\n",
        "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"changesets\": []\n}\n",
    ] {
        let err = format!(
            "{:#}",
            PreJson::load(changeset_dir(pre_json).path()).err().unwrap()
        );
        assert!(err.contains("changesets v2"), "{pre_json:?}: {err}");
    }
}

#[test]
fn rejects_a_malformed_pre_json() {
    for pre_json in [
        "[]\n",
        "{\n  \"tag\": \"beta\"\n}\n",
        "{\n  \"mode\": \"pré\",\n  \"tag\": \"beta\"\n}\n",
        "{\n  \"mode\": 1,\n  \"tag\": \"beta\"\n}\n",
        "{\n  \"mode\": \"pre\"\n}\n",
        "{\n  \"mode\": \"pre\",\n  \"tag\": null\n}\n",
    ] {
        assert!(
            PreJson::load(changeset_dir(pre_json).path()).is_err(),
            "{pre_json:?}"
        );
    }
}

#[test]
fn rewrites_in_place_keeping_unknown_fields_and_formatting() {
    let pre = load("{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"someday\": [1, 2, 3]\n}\n");
    assert_eq!(pre.mode(), PreMode::Pre);
    assert_eq!(pre.tag(), "beta");

    let mut pre = load(
        "{ // pre state\n\t\"tag\":\t\"alpha\",\n\t\"mode\": \"exit\",\n\t\"someday\": [1, 2, 3]\n}",
    );
    assert_eq!(pre.mode(), PreMode::Exit);
    assert_eq!(pre.tag(), "alpha");
    pre.set_mode(PreMode::Pre);
    pre.set_tag("beta.2");
    assert_eq!(pre.mode(), PreMode::Pre);
    assert_eq!(pre.tag(), "beta.2");
    assert_eq!(
        pre.text(),
        "{ // pre state\n\t\"tag\":\t\"beta.2\",\n\t\"mode\": \"pre\",\n\t\"someday\": [1, 2, 3]\n}"
    );
}

#[test]
fn validate_tag_accepts_dotted_and_numeric_tags() {
    for tag in ["beta", "beta.2", "1", "rc-0"] {
        assert!(validate_tag(tag).is_ok(), "{tag} should be accepted");
    }
}

#[test]
fn validate_tag_rejects_invalid_tags() {
    for tag in ["", " ", "beta 2", "beta_2", "ベータ", "01", "beta."] {
        assert!(validate_tag(tag).is_err(), "{tag:?} should be rejected");
    }
}
