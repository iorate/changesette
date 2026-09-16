mod util;

use std::fs;

use changesette::{
    bump::Bump,
    config::{self, Config, ResolvedGroups, UpdateInternalDependencies},
};
use util::capture_output;

fn load_ok(text: &str) -> Config {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.json"), text).unwrap();
    config::load(dir.path()).unwrap()
}

fn load_err(text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.json"), text).unwrap();
    format!("{:#}", config::load(dir.path()).unwrap_err())
}

fn resolve(text: &str, names: &[&str]) -> Vec<String> {
    load_ok(text).resolve_ignore(names.iter().copied())
}

fn resolve_groups(text: &str, names: &[&str]) -> (ResolvedGroups, String) {
    let mut groups = None;
    let output = capture_output(|| groups = Some(load_ok(text).resolve_groups(names).unwrap()));
    (groups.unwrap(), output)
}

fn packages_config(entry: &str) -> String {
    format!(
        "{{ \"changesette\": {{ \"packages\": [{}] }} }}\n",
        serde_json::to_string(entry).unwrap()
    )
}

#[test]
fn a_missing_or_empty_config_yields_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(config::load(dir.path()).unwrap(), Config::default());
    assert_eq!(load_ok("{}\n"), Config::default());
}

#[test]
fn resolves_private_packages_version() {
    for (text, expected) in [
        ("{ \"privatePackages\": { \"version\": true } }\n", true),
        ("{ \"privatePackages\": true }\n", true),
        ("{ \"privatePackages\": { \"version\": false } }\n", false),
        ("{ \"privatePackages\": false }\n", false),
        ("{ \"privatePackages\": {} }\n", false),
        ("{}\n", false),
    ] {
        assert_eq!(load_ok(text).private_packages_version, expected, "{text}");
    }
}

#[test]
fn resolves_ignore_names_and_globs() {
    assert_eq!(
        resolve("{ \"ignore\": [\"pkg-a\"] }\n", &["pkg-a", "pkg-b"]),
        ["pkg-a"]
    );
    assert_eq!(
        resolve(
            "{ \"ignore\": [\"@scope/*\"] }\n",
            &["@scope/a", "@scope/b", "pkg-a"]
        ),
        ["@scope/a", "@scope/b"]
    );
    assert_eq!(
        resolve(
            "{ \"ignore\": [\"pkg-{a,b}\"] }\n",
            &["pkg-a", "pkg-b", "pkg-c"]
        ),
        ["pkg-a", "pkg-b"]
    );
    assert!(resolve("{ \"ignore\": [\"missing-*\"] }\n", &["pkg-a"]).is_empty());
}

#[test]
fn resolves_negation_in_pattern_order() {
    assert_eq!(
        resolve(
            "{ \"ignore\": [\"pkg-*\", \"!pkg-a\"] }\n",
            &["pkg-a", "pkg-b"]
        ),
        ["pkg-b"]
    );
    assert_eq!(
        resolve(
            "{ \"ignore\": [\"!pkg-b\", \"pkg-*\"] }\n",
            &["pkg-a", "pkg-b"]
        ),
        ["pkg-a", "pkg-b"]
    );
    assert_eq!(
        resolve(
            "{ \"ignore\": [\"pkg-*\", \"!!pkg-a\"] }\n",
            &["pkg-a", "pkg-b"]
        ),
        ["pkg-a", "pkg-b"]
    );
}

#[test]
fn resolves_group_globs_with_negation() {
    let (groups, output) = resolve_groups(
        "{ \"fixed\": [[\"pkg-*\", \"!pkg-b\"]], \"linked\": [[\"pkg-b\"]] }\n",
        &["pkg-a", "pkg-b", "pkg-c"],
    );
    assert_eq!(groups.fixed, [["pkg-a", "pkg-c"]]);
    assert_eq!(groups.linked, [["pkg-b"]]);
    assert_eq!(output, "");
}

#[test]
fn resolves_empty_groups_without_warnings() {
    let (groups, output) = resolve_groups("{}\n", &["pkg-a"]);
    assert!(groups.fixed.is_empty());
    assert!(groups.linked.is_empty());
    assert_eq!(output, "");
}

#[test]
fn rejects_a_package_in_two_groups() {
    for text in [
        "{ \"fixed\": [[\"pkg-*\"], [\"pkg-b\"]] }\n",
        "{ \"linked\": [[\"pkg-a\", \"pkg-b\"], [\"pkg-b\"]] }\n",
        "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]], \"linked\": [[\"pkg-b\"]] }\n",
    ] {
        let err = load_ok(text)
            .resolve_groups(&["pkg-a", "pkg-b"])
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("pkg-b"), "{text}: {err:#}");
    }
}

#[test]
fn warns_on_a_group_pattern_matching_nothing() {
    let (groups, output) = resolve_groups(
        "{ \"fixed\": [[\"pkg-a\", \"missing-*\"]] }\n",
        &["pkg-a", "pkg-b"],
    );
    assert_eq!(groups.fixed, [["pkg-a"]]);
    assert!(output.starts_with("warning: "), "{output}");
    assert!(output.contains("\"missing-*\""), "{output}");
    assert_eq!(output.lines().count(), 1, "{output}");

    let (groups, output) = resolve_groups(
        "{ \"linked\": [[\"pkg-*\", \"!pkg-*\"]] }\n",
        &["pkg-a", "pkg-b"],
    );
    assert_eq!(groups.linked, [[] as [&str; 0]]);
    assert!(output.starts_with("warning: "), "{output}");
    assert!(output.contains("\"!pkg-*\""), "{output}");

    let (groups, output) = resolve_groups(
        "{ \"linked\": [[\"pkg-*\", \"!pkg-b\"]] }\n",
        &["pkg-a", "pkg-b"],
    );
    assert_eq!(groups.linked, [["pkg-a"]]);
    assert_eq!(output, "");
}

#[test]
fn resolves_snapshot_settings() {
    let config = load_ok(
        "{\n  \"snapshot\": {\n    \"useCalculatedVersion\": true,\n    \"prereleaseTemplate\": \"{tag}-{timestamp}\"\n  }\n}\n",
    );
    assert!(config.snapshot_use_calculated_version);
    assert_eq!(
        config.snapshot_prerelease_template.as_deref(),
        Some("{tag}-{timestamp}")
    );
    let config = load_ok("{ \"snapshot\": {} }\n");
    assert!(!config.snapshot_use_calculated_version);
    assert!(config.snapshot_prerelease_template.is_none());
}

#[test]
fn resolves_update_internal_dependencies() {
    for (text, expected) in [
        (
            "{ \"updateInternalDependencies\": \"patch\" }\n",
            UpdateInternalDependencies::Patch,
        ),
        (
            "{ \"updateInternalDependencies\": \"minor\" }\n",
            UpdateInternalDependencies::Minor,
        ),
    ] {
        assert_eq!(
            load_ok(text).update_internal_dependencies,
            expected,
            "{text}"
        );
    }
}

#[test]
fn update_internal_dependencies_allows_bumps_at_or_above_its_level() {
    for bump in [Bump::Patch, Bump::Minor, Bump::Major] {
        assert!(UpdateInternalDependencies::Patch.allows(bump), "{bump:?}");
    }
    assert!(!UpdateInternalDependencies::Minor.allows(Bump::Patch));
    assert!(UpdateInternalDependencies::Minor.allows(Bump::Minor));
    assert!(UpdateInternalDependencies::Minor.allows(Bump::Major));
}

#[test]
fn resolves_bump_versions_with_workspace_protocol_only() {
    assert!(
        load_ok("{ \"bumpVersionsWithWorkspaceProtocolOnly\": true }\n")
            .bump_versions_with_workspace_protocol_only
    );
    assert!(
        !load_ok("{ \"bumpVersionsWithWorkspaceProtocolOnly\": false }\n")
            .bump_versions_with_workspace_protocol_only
    );
}

#[test]
fn resolves_changesette_manage_internal_dependencies() {
    assert!(
        load_ok("{ \"changesette\": { \"manageInternalDependencies\": true } }\n")
            .manage_internal_dependencies
    );
    assert!(
        !load_ok("{ \"changesette\": { \"manageInternalDependencies\": false } }\n")
            .manage_internal_dependencies
    );
    assert!(load_ok("{ \"changesette\": {} }\n").manage_internal_dependencies);
}

#[test]
fn resolves_changesette_packages() {
    let config =
        load_ok("{ \"changesette\": { \"packages\": [\"packages/a\", \"packages/b\", \".\"] } }\n");
    assert_eq!(
        config.packages.as_deref(),
        Some(
            &[
                "packages/a".to_owned(),
                "packages/b".to_owned(),
                ".".to_owned()
            ][..]
        )
    );
    let config = load_ok("{ \"changesette\": { \"packages\": [] } }\n");
    assert_eq!(config.packages.as_deref(), Some(&[][..]));
    assert!(load_ok("{ \"changesette\": {} }\n").packages.is_none());
    assert!(
        load_ok("{ \"changesette\": { \"other\": true } }\n")
            .packages
            .is_none()
    );
}

#[test]
fn ignores_unknown_keys() {
    let config = load_ok(
        "{\n  \"$schema\": \"https://unpkg.com/@changesets/config@4.0.0/schema.json\",\n  \"changelog\": \"@changesets/cli/changelog\",\n  \"commit\": false,\n  \"privatePackages\": {\n    \"version\": true,\n    \"tag\": false\n  }\n}\n",
    );
    assert!(config.private_packages_version);
}

#[test]
fn rejects_an_unparsable_config() {
    for text in ["", "{\n", "[]\n"] {
        let err = load_err(text);
        assert!(err.contains("config.json"), "{text:?}: {err}");
    }
}

#[test]
fn rejects_wrong_types() {
    for (text, needle) in [
        ("{ \"ignore\": \"pkg-a\" }\n", "\"ignore\""),
        ("{ \"ignore\": [\"pkg-a\", 1] }\n", "\"ignore\""),
        ("{ \"ignore\": [\"pkg-[\"] }\n", "\"pkg-[\""),
        ("{ \"fixed\": \"pkg-a\" }\n", "\"fixed\""),
        ("{ \"fixed\": [\"pkg-a\"] }\n", "\"fixed\""),
        ("{ \"linked\": [[\"pkg-a\", 1]] }\n", "\"linked\""),
        ("{ \"linked\": [[\"pkg-a\"], [\"pkg-[\"]] }\n", "\"pkg-[\""),
        ("{ \"privatePackages\": \"all\" }\n", "\"privatePackages\""),
        (
            "{ \"privatePackages\": { \"version\": \"yes\" } }\n",
            "\"version\"",
        ),
        ("{ \"snapshot\": true }\n", "\"snapshot\""),
        (
            "{ \"snapshot\": { \"useCalculatedVersion\": \"yes\" } }\n",
            "\"useCalculatedVersion\"",
        ),
        (
            "{ \"snapshot\": { \"prereleaseTemplate\": 1 } }\n",
            "\"prereleaseTemplate\"",
        ),
        (
            "{ \"snapshot\": { \"prereleaseTemplate\": \"\" } }\n",
            "\"prereleaseTemplate\"",
        ),
        (
            "{ \"updateInternalDependencies\": \"major\" }\n",
            "\"updateInternalDependencies\"",
        ),
        (
            "{ \"updateInternalDependencies\": true }\n",
            "\"updateInternalDependencies\"",
        ),
        (
            "{ \"bumpVersionsWithWorkspaceProtocolOnly\": \"yes\" }\n",
            "\"bumpVersionsWithWorkspaceProtocolOnly\"",
        ),
        ("{ \"changesette\": [] }\n", "\"changesette\""),
        (
            "{ \"changesette\": { \"manageInternalDependencies\": \"no\" } }\n",
            "\"manageInternalDependencies\"",
        ),
        (
            "{ \"changesette\": { \"packages\": null } }\n",
            "\"packages\"",
        ),
        (
            "{ \"changesette\": { \"packages\": [\"packages/a\", 1] } }\n",
            "\"packages\"",
        ),
        (
            "{ \"changesette\": { \"packages\": [\"/x\"] } }\n",
            "\"/x\"",
        ),
    ] {
        let err = load_err(text);
        assert!(err.contains("config.json"), "{text}: {err}");
        assert!(err.contains(needle), "{text}: {err}");
    }
}

#[test]
fn accepts_changesette_packages_entries_by_syntax_only() {
    let entries = [
        "a",
        "a/b",
        "a/!b",
        ".",
        "../a",
        "a/../b",
        "a/./b",
        "..",
        "./a",
        "a//b",
        "a/",
        "./",
        "C:/x",
        "C:x",
        "packages/*",
        "a?",
        "[a]",
        "a]",
        "{a,b}",
        "a}",
        "!a",
        "**",
    ];
    for entry in entries {
        assert_eq!(
            load_ok(&packages_config(entry)).packages.as_deref(),
            Some(&[entry.to_owned()][..])
        );
    }
    #[cfg(unix)]
    for entry in ["a\\b", "\\a", "a\\", "\\"] {
        assert_eq!(
            load_ok(&packages_config(entry)).packages.as_deref(),
            Some(&[entry.to_owned()][..])
        );
    }
}

#[test]
fn rejects_empty_absolute_and_backslash_entries() {
    assert!(load_err(&packages_config("")).contains("empty"));
    for entry in ["/a", "/"] {
        assert!(
            load_err(&packages_config(entry)).contains("absolute"),
            "{entry}"
        );
    }
    #[cfg(windows)]
    for entry in ["a\\b", "\\a", "a\\", "\\"] {
        assert!(
            load_err(&packages_config(entry)).contains("`\\`"),
            "{entry}"
        );
    }
}
