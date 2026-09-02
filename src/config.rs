use std::{
    collections::BTreeSet,
    path::{Component, Path, Prefix},
};

use anyhow::{Context, Result, bail};
use fast_glob::{glob_match, validate};
use serde_json::{Map, Value};
use tracing::warn;

use crate::workspace::read_json;

#[derive(Debug, Default)]
pub(crate) struct Config {
    ignore: Vec<String>,
    fixed: Vec<Vec<String>>,
    linked: Vec<Vec<String>>,
    pub(crate) private_packages_version: bool,
    pub(crate) snapshot_use_calculated_version: bool,
    pub(crate) snapshot_prerelease_template: Option<String>,
    pub(crate) packages: Option<Vec<String>>,
}

impl Config {
    pub(crate) fn has_ignore(&self) -> bool {
        !self.ignore.is_empty()
    }

    pub(crate) fn resolve_ignore<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        expand_patterns(&self.ignore, names)
    }

    pub(crate) fn resolve_groups(&self, names: &[&str]) -> Result<ResolvedGroups> {
        let expand = |groups: &[Vec<String>]| -> Vec<Vec<String>> {
            groups
                .iter()
                .map(|patterns| expand_patterns(patterns, names.iter().copied()))
                .collect()
        };
        let fixed = expand(&self.fixed);
        let linked = expand(&self.linked);

        check_group_duplicates("fixed", &fixed)?;
        check_group_duplicates("linked", &linked)?;
        let fixed_names: BTreeSet<&String> = fixed.iter().flatten().collect();
        for name in linked.iter().flatten() {
            if fixed_names.contains(name) {
                bail!(
                    "package `{name}` is in both a \"fixed\" and a \"linked\" group; a package can be in only one of them"
                );
            }
        }

        for (key, groups) in [("fixed", &self.fixed), ("linked", &self.linked)] {
            for pattern in groups.iter().flatten() {
                // Each pattern is judged alone, unlike the ordered expansion
                // above, so it is handed to the matcher as written, its
                // leading `!` included.
                if !names.iter().any(|name| glob_match(pattern, *name)) {
                    warn!(
                        "{key}: the package or glob {pattern:?} does not match any package in the workspace"
                    );
                }
            }
        }

        Ok(ResolvedGroups { fixed, linked })
    }
}

pub(crate) struct ResolvedGroups {
    pub(crate) fixed: Vec<Vec<String>>,
    pub(crate) linked: Vec<Vec<String>>,
}

fn expand_patterns<'a>(
    patterns: &[String],
    names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut resolved = Vec::new();
    for name in names {
        let mut matched = false;
        for pattern in patterns {
            // A negated pattern un-matches what its body matches, which the
            // negation of fast-glob cannot express, so the leading `!`s are
            // split off instead of letting fast-glob apply them.
            let (negated, body) = split_negation(pattern);
            if glob_match(body, name) {
                matched = !negated;
            }
        }
        if matched {
            resolved.push(name.to_owned());
        }
    }
    resolved
}

fn split_negation(pattern: &str) -> (bool, &str) {
    let body = pattern.trim_start_matches('!');
    ((pattern.len() - body.len()) % 2 == 1, body)
}

// Each name appears at most once per expanded group, so a duplicate can only
// come from another group.
fn check_group_duplicates(key: &str, groups: &[Vec<String>]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in groups.iter().flatten() {
        if !seen.insert(name) {
            bail!(
                "package `{name}` is in multiple \"{key}\" groups; a package can belong to only one group"
            );
        }
    }
    Ok(())
}

pub(crate) fn load(changeset_dir: &Path) -> Result<Config> {
    let path = changeset_dir.join("config.json");
    let Some(value) = read_json(&path)? else {
        return Ok(Config::default());
    };
    load_value(&value).with_context(|| path.display().to_string())
}

fn load_value(value: &Value) -> Result<Config> {
    let Some(object) = value.as_object() else {
        bail!("the root value must be an object")
    };

    let mut ignore = Vec::new();
    if let Some(value) = object.get("ignore") {
        let patterns = value.as_array().and_then(|items| {
            items
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<&str>>>()
        });
        let Some(patterns) = patterns else {
            bail!("\"ignore\" must be an array of strings")
        };
        for pattern in patterns {
            validate(pattern).with_context(|| format!("invalid ignore pattern {pattern:?}"))?;
            ignore.push(pattern.to_owned());
        }
    }

    let mut groups = [Vec::new(), Vec::new()];
    for (key, parsed) in ["fixed", "linked"].into_iter().zip(&mut groups) {
        let Some(value) = object.get(key) else {
            continue;
        };
        let raw_groups = value.as_array().and_then(|groups| {
            groups
                .iter()
                .map(|group| {
                    group.as_array().and_then(|items| {
                        items
                            .iter()
                            .map(Value::as_str)
                            .collect::<Option<Vec<&str>>>()
                    })
                })
                .collect::<Option<Vec<Vec<&str>>>>()
        });
        let Some(raw_groups) = raw_groups else {
            bail!("\"{key}\" must be an array of arrays of strings")
        };
        for (index, raw_group) in raw_groups.into_iter().enumerate() {
            let mut group = Vec::new();
            for pattern in raw_group {
                validate(pattern).with_context(|| {
                    format!("invalid pattern {pattern:?} in \"{key}\"[{index}]")
                })?;
                group.push(pattern.to_owned());
            }
            parsed.push(group);
        }
    }
    let [fixed, linked] = groups;

    let private_packages_version = match object.get("privatePackages") {
        None => false,
        Some(Value::Bool(version)) => *version,
        Some(Value::Object(object)) => match object.get("version") {
            None => false,
            Some(Value::Bool(version)) => *version,
            Some(_) => bail!("\"version\" in \"privatePackages\" must be a boolean"),
        },
        Some(_) => bail!("\"privatePackages\" must be a boolean or an object"),
    };

    let mut snapshot_use_calculated_version = false;
    let mut snapshot_prerelease_template = None;
    match object.get("snapshot") {
        None => {}
        Some(Value::Object(snapshot)) => {
            match snapshot.get("useCalculatedVersion") {
                None => {}
                Some(Value::Bool(use_calculated_version)) => {
                    snapshot_use_calculated_version = *use_calculated_version;
                }
                Some(_) => bail!("\"useCalculatedVersion\" in \"snapshot\" must be a boolean"),
            }
            match snapshot.get("prereleaseTemplate") {
                None => {}
                Some(Value::String(template)) if !template.is_empty() => {
                    snapshot_prerelease_template = Some(template.clone());
                }
                Some(_) => {
                    bail!("\"prereleaseTemplate\" in \"snapshot\" must be a non-empty string")
                }
            }
        }
        Some(_) => bail!("\"snapshot\" must be an object"),
    }

    let packages = load_packages(object)?;

    Ok(Config {
        ignore,
        fixed,
        linked,
        private_packages_version,
        snapshot_use_calculated_version,
        snapshot_prerelease_template,
        packages,
    })
}

fn load_packages(object: &Map<String, Value>) -> Result<Option<Vec<String>>> {
    let changesette = match object.get("changesette") {
        None => return Ok(None),
        Some(Value::Object(changesette)) => changesette,
        Some(_) => bail!("\"changesette\" must be an object"),
    };
    let Some(value) = changesette.get("packages") else {
        return Ok(None);
    };
    let dirs = value.as_array().and_then(|items| {
        items
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<&str>>>()
    });
    let Some(dirs) = dirs else {
        bail!("\"packages\" in \"changesette\" must be an array of strings")
    };
    let mut packages = Vec::new();
    for dir in dirs {
        validate_rel_dir(dir)
            .with_context(|| format!("invalid \"changesette.packages\" entry {dir:?}"))?;
        packages.push(dir.to_owned());
    }
    Ok(Some(packages))
}

fn validate_rel_dir(text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("an empty path is not supported")
    }
    #[cfg(windows)]
    if text.contains('\\') {
        bail!("`\\` is not supported; use `/` as the separator")
    }
    if text.starts_with('/') {
        bail!("absolute paths are not supported")
    }
    if has_drive_prefix(text.split('/').next().unwrap_or_default()) {
        bail!("drive-prefixed paths are not supported")
    }
    Ok(())
}

fn has_drive_prefix(part: &str) -> bool {
    matches!(
        Path::new(part).components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn load_ok(text: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), text).unwrap();
        load(dir.path()).unwrap()
    }

    fn private_packages_version(text: &str) -> bool {
        load_ok(text).private_packages_version
    }

    fn resolve(text: &str, names: &[&str]) -> Vec<String> {
        load_ok(text).resolve_ignore(names.iter().copied())
    }

    fn validate_err(text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), text).unwrap();
        let err = load(dir.path()).err().unwrap();
        format!("{err:#}").replace(
            &dir.path().join("config.json").display().to_string(),
            ".changeset/config.json",
        )
    }

    fn assert_default(config: &Config) {
        assert!(!config.has_ignore());
        assert!(!config.private_packages_version);
        assert!(!config.snapshot_use_calculated_version);
        assert!(config.snapshot_prerelease_template.is_none());
        assert!(config.packages.is_none());
    }

    #[test]
    fn a_missing_file_yields_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_default(&load(dir.path()).unwrap());
    }

    #[test]
    fn accepts_an_empty_object() {
        assert_default(&load_ok("{}\n"));
    }

    #[test]
    fn resolves_private_packages_version() {
        assert!(private_packages_version(
            "{ \"privatePackages\": { \"version\": true } }\n"
        ));
        assert!(private_packages_version("{ \"privatePackages\": true }\n"));
        assert!(!private_packages_version(
            "{ \"privatePackages\": { \"version\": false } }\n"
        ));
        assert!(!private_packages_version(
            "{ \"privatePackages\": false }\n"
        ));
        assert!(!private_packages_version("{ \"privatePackages\": {} }\n"));
        assert!(!private_packages_version("{}\n"));
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
    }

    #[test]
    fn resolves_negation_in_pattern_order() {
        assert_eq!(
            resolve(
                "{ \"ignore\": [\"pkg-*\", \"!pkg-b\"] }\n",
                &["pkg-a", "pkg-b"]
            ),
            ["pkg-a"]
        );
        assert_eq!(
            resolve(
                "{ \"ignore\": [\"!pkg-b\", \"pkg-*\"] }\n",
                &["pkg-a", "pkg-b"]
            ),
            ["pkg-a", "pkg-b"]
        );
    }

    #[test]
    fn treats_a_doubled_negation_as_a_plain_pattern() {
        assert_eq!(
            resolve(
                "{ \"ignore\": [\"pkg-*\", \"!!pkg-a\"] }\n",
                &["pkg-a", "pkg-b"]
            ),
            ["pkg-a", "pkg-b"]
        );
    }

    #[test]
    fn tolerates_an_ignore_pattern_matching_nothing() {
        assert!(resolve("{ \"ignore\": [\"missing-*\"] }\n", &["pkg-a"]).is_empty());
    }

    #[test]
    fn rejects_an_invalid_ignore_pattern() {
        insta::assert_snapshot!(validate_err("{ \"ignore\": [\"pkg-[\"] }\n"));
    }

    #[test]
    fn accepts_every_supported_shape() {
        load_ok(
            "{\n  \"ignore\": [\"pkg-a\", \"@scope/*\"],\n  \"privatePackages\": {\n    \"version\": true\n  }\n}\n",
        );
        load_ok("{ \"ignore\": [] }\n");
        load_ok("{ \"fixed\": [], \"linked\": [] }\n");
        load_ok("{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]], \"linked\": [[\"@scope/*\"]] }\n");
    }

    fn capture_output(f: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

        impl std::io::Write for Buffer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .event_format(crate::output::Formatter)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    fn resolve_groups(text: &str, names: &[&str]) -> (ResolvedGroups, String) {
        let mut groups = None;
        let output = capture_output(|| groups = Some(load_ok(text).resolve_groups(names).unwrap()));
        (groups.unwrap(), output)
    }

    fn resolve_groups_err(text: &str, names: &[&str]) -> String {
        let err = load_ok(text).resolve_groups(names).err().unwrap();
        format!("{err:#}")
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
    fn rejects_a_package_in_multiple_fixed_groups() {
        insta::assert_snapshot!(resolve_groups_err(
            "{ \"fixed\": [[\"pkg-*\"], [\"pkg-b\"]] }\n",
            &["pkg-a", "pkg-b"],
        ));
    }

    #[test]
    fn rejects_a_package_in_multiple_linked_groups() {
        insta::assert_snapshot!(resolve_groups_err(
            "{ \"linked\": [[\"pkg-a\", \"pkg-b\"], [\"pkg-b\"]] }\n",
            &["pkg-a", "pkg-b"],
        ));
    }

    #[test]
    fn rejects_a_package_in_both_fixed_and_linked_groups() {
        insta::assert_snapshot!(resolve_groups_err(
            "{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]], \"linked\": [[\"pkg-b\"]] }\n",
            &["pkg-a", "pkg-b"],
        ));
    }

    #[test]
    fn warns_on_a_group_pattern_matching_nothing() {
        let (groups, output) = resolve_groups(
            "{ \"fixed\": [[\"pkg-a\", \"missing-*\"]] }\n",
            &["pkg-a", "pkg-b"],
        );
        assert_eq!(groups.fixed, [["pkg-a"]]);
        assert_eq!(
            output,
            "warning: fixed: the package or glob \"missing-*\" does not match any package in the workspace\n"
        );
    }

    #[test]
    fn warns_on_a_negated_pattern_whose_body_matches_everything() {
        let (groups, output) = resolve_groups(
            "{ \"linked\": [[\"pkg-*\", \"!pkg-*\"]] }\n",
            &["pkg-a", "pkg-b"],
        );
        assert_eq!(groups.linked, [[] as [&str; 0]]);
        assert_eq!(
            output,
            "warning: linked: the package or glob \"!pkg-*\" does not match any package in the workspace\n"
        );
    }

    #[test]
    fn does_not_warn_on_a_negated_pattern_whose_body_misses_a_name() {
        let (groups, output) = resolve_groups(
            "{ \"linked\": [[\"pkg-*\", \"!pkg-b\"]] }\n",
            &["pkg-a", "pkg-b"],
        );
        assert_eq!(groups.linked, [["pkg-a"]]);
        assert_eq!(output, "");
    }

    #[test]
    fn rejects_a_non_array_fixed() {
        insta::assert_snapshot!(validate_err("{ \"fixed\": \"pkg-a\" }\n"));
    }

    #[test]
    fn rejects_a_non_array_group() {
        insta::assert_snapshot!(validate_err("{ \"fixed\": [\"pkg-a\"] }\n"));
    }

    #[test]
    fn rejects_a_non_string_group_item() {
        insta::assert_snapshot!(validate_err("{ \"linked\": [[\"pkg-a\", 1]] }\n"));
    }

    #[test]
    fn rejects_an_invalid_group_pattern() {
        insta::assert_snapshot!(validate_err("{ \"linked\": [[\"pkg-a\"], [\"pkg-[\"]] }\n"));
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
    fn rejects_a_non_object_snapshot() {
        insta::assert_snapshot!(validate_err("{ \"snapshot\": true }\n"));
    }

    #[test]
    fn rejects_a_non_boolean_snapshot_use_calculated_version() {
        insta::assert_snapshot!(validate_err(
            "{ \"snapshot\": { \"useCalculatedVersion\": \"yes\" } }\n"
        ));
    }

    #[test]
    fn rejects_a_non_string_snapshot_prerelease_template() {
        insta::assert_snapshot!(validate_err(
            "{ \"snapshot\": { \"prereleaseTemplate\": 1 } }\n"
        ));
    }

    #[test]
    fn rejects_an_empty_snapshot_prerelease_template() {
        insta::assert_snapshot!(validate_err(
            "{ \"snapshot\": { \"prereleaseTemplate\": \"\" } }\n"
        ));
    }

    #[test]
    fn ignores_unknown_keys() {
        let config = load_ok(
            "{\n  \"$schema\": \"https://unpkg.com/@changesets/config@4.0.0/schema.json\",\n  \"changelog\": \"@changesets/cli/changelog\",\n  \"commit\": false,\n  \"privatePackages\": {\n    \"version\": true,\n    \"tag\": false\n  }\n}\n",
        );
        assert!(config.private_packages_version);
    }

    #[test]
    fn rejects_an_empty_file() {
        insta::assert_snapshot!(validate_err(""));
    }

    #[test]
    fn rejects_invalid_json() {
        insta::assert_snapshot!(validate_err("{\n"));
    }

    #[test]
    fn rejects_a_non_object_root() {
        insta::assert_snapshot!(validate_err("[]\n"));
    }

    #[test]
    fn rejects_a_non_array_ignore() {
        insta::assert_snapshot!(validate_err("{ \"ignore\": \"pkg-a\" }\n"));
    }

    #[test]
    fn rejects_a_non_string_ignore_item() {
        insta::assert_snapshot!(validate_err("{ \"ignore\": [\"pkg-a\", 1] }\n"));
    }

    #[test]
    fn rejects_a_non_boolean_non_object_private_packages() {
        insta::assert_snapshot!(validate_err("{ \"privatePackages\": \"all\" }\n"));
    }

    #[test]
    fn rejects_a_non_boolean_private_packages_version() {
        insta::assert_snapshot!(validate_err(
            "{ \"privatePackages\": { \"version\": \"yes\" } }\n"
        ));
    }

    #[test]
    fn resolves_changesette_packages() {
        let config = load_ok(
            "{ \"changesette\": { \"packages\": [\"packages/a\", \"packages/b\", \".\"] } }\n",
        );
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
    fn rejects_a_non_object_changesette() {
        insta::assert_snapshot!(validate_err("{ \"changesette\": [] }\n"));
    }

    #[test]
    fn rejects_a_null_changesette_packages() {
        insta::assert_snapshot!(validate_err(
            "{ \"changesette\": { \"packages\": null } }\n"
        ));
    }

    #[test]
    fn rejects_a_non_string_changesette_packages_entry() {
        insta::assert_snapshot!(validate_err(
            "{ \"changesette\": { \"packages\": [\"packages/a\", 1] } }\n"
        ));
    }

    #[test]
    fn rejects_an_invalid_changesette_packages_entry() {
        insta::assert_snapshot!(validate_err(
            "{ \"changesette\": { \"packages\": [\"/x\"] } }\n"
        ));
    }

    fn rel_dir_err(text: &str) -> String {
        format!("{:#}", validate_rel_dir(text).unwrap_err())
    }

    #[test]
    fn accepts_non_canonical_spellings() {
        for text in [
            "a", "a/b", "a/!b", ".", "../a", "a/../b", "..", "./a", "a//b", "a/", "./",
        ] {
            validate_rel_dir(text).unwrap();
        }
    }

    #[test]
    fn rejects_an_empty_path() {
        assert!(rel_dir_err("").contains("empty"));
    }

    #[test]
    fn rejects_an_absolute_path() {
        for text in ["/a", "/"] {
            assert!(rel_dir_err(text).contains("absolute"), "{text}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_backslash() {
        for text in ["a\\b", "\\a", "a\\", "\\"] {
            assert!(rel_dir_err(text).contains("`\\`"), "{text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_backslash_is_an_ordinary_name_character_on_unix() {
        for text in ["a\\b", "\\a", "a\\", "\\"] {
            validate_rel_dir(text).unwrap();
        }
    }

    #[test]
    fn accepts_glob_like_names() {
        for text in ["packages/*", "a?", "[a]", "a]", "{a,b}", "a}", "!a", "**"] {
            validate_rel_dir(text).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_leading_drive_prefix() {
        for text in ["C:/x", "C:x", "C:"] {
            assert!(rel_dir_err(text).contains("drive"), "{text}");
        }
        for text in ["packages/C:/x", "a/C:x", "../C:/x"] {
            validate_rel_dir(text).unwrap();
        }
        assert!(has_drive_prefix("C:"));
        assert!(has_drive_prefix("C:x"));
        assert!(!has_drive_prefix("a"));
    }

    #[cfg(unix)]
    #[test]
    fn a_drive_like_segment_is_an_ordinary_name_on_unix() {
        for text in ["C:/x", "C:x"] {
            validate_rel_dir(text).unwrap();
        }
        assert!(!has_drive_prefix("C:"));
    }
}
