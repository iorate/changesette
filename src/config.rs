use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use wax::{Glob, Program};

use crate::workspace::read_json;

/// The effective settings from `.changeset/config.json`.
#[derive(Debug, Default)]
pub(crate) struct Config {
    // The parsed patterns of the `ignore` setting as (negated, glob) pairs;
    // `resolve_ignore` expands them into package names.
    ignore: Vec<(bool, Glob<'static>)>,
    /// Whether private packages are versioned, per the `privatePackages`
    /// setting.
    pub(crate) private_packages_version: bool,
}

impl Config {
    /// Whether the `ignore` setting defines any patterns.
    pub(crate) fn has_ignore(&self) -> bool {
        !self.ignore.is_empty()
    }

    /// Expands the `ignore` patterns against `names` and returns the matching
    /// names in input order, with a `!`-prefixed pattern un-ignoring the
    /// names it matches, in order; a pattern matching no name matches
    /// nothing.
    pub(crate) fn resolve_ignore<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        let mut resolved = Vec::new();
        for name in names {
            let mut ignored = false;
            for (negated, glob) in &self.ignore {
                if *negated {
                    if ignored && glob.is_match(name) {
                        ignored = false;
                    }
                } else if !ignored && glob.is_match(name) {
                    ignored = true;
                }
            }
            if ignored {
                resolved.push(name.to_owned());
            }
        }
        resolved
    }
}

fn parse_ignore_pattern(pattern: &str) -> Result<(bool, Glob<'static>)> {
    // wax does not parse the leading `!`, so it is stripped here and the
    // negation applied by resolve_ignore.
    let (negated, body) = match pattern.strip_prefix('!') {
        Some(body) => (true, body),
        None => (false, pattern),
    };
    let glob = Glob::new(body)
        .map(Glob::into_owned)
        .with_context(|| format!("invalid ignore pattern {pattern:?}"))?;
    Ok((negated, glob))
}

/// Loads the supported subset of `changeset_dir/config.json`, treating a
/// missing file as all defaults and ignoring unknown keys.
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
            ignore.push(parse_ignore_pattern(pattern)?);
        }
    }

    for key in ["fixed", "linked"] {
        if let Some(value) = object.get(key) {
            if value.as_array().is_none_or(|groups| !groups.is_empty()) {
                bail!("\"{key}\" is not yet implemented");
            }
        }
    }

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

    Ok(Config {
        ignore,
        private_packages_version,
    })
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
    }

    #[test]
    fn rejects_a_non_empty_fixed() {
        insta::assert_snapshot!(validate_err("{ \"fixed\": [[\"pkg-a\", \"pkg-b\"]] }\n"));
    }

    #[test]
    fn rejects_a_non_empty_linked() {
        insta::assert_snapshot!(validate_err("{ \"linked\": [[\"pkg-a\", \"pkg-b\"]] }\n"));
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
}
