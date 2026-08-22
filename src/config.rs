use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use wax::{Glob, Program};

/// The effective settings from `.changeset/config.json`.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct Config {
    /// The raw patterns of the `ignore` setting; `resolve_ignore` expands
    /// them into package names.
    ignore: Vec<String>,
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
    /// names it matches, in order; a non-negated literal entry matching no
    /// name is an error as in upstream changesets, while a glob matching
    /// nothing is not.
    pub(crate) fn resolve_ignore<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<String>> {
        let patterns = self
            .ignore
            .iter()
            .map(|pattern| parse_ignore_pattern(pattern))
            .collect::<Result<Vec<_>>>()?;
        let mut matched = vec![false; patterns.len()];
        let mut resolved = Vec::new();
        for name in names {
            let mut ignored = false;
            for ((negated, glob), matched) in patterns.iter().zip(&mut matched) {
                let is_match = glob.is_match(name);
                *matched = *matched || is_match;
                if *negated {
                    if ignored && is_match {
                        ignored = false;
                    }
                } else if !ignored && is_match {
                    ignored = true;
                }
            }
            if ignored {
                resolved.push(name.to_owned());
            }
        }
        // Upstream changesets refuses any ignore entry matching no package;
        // only entries that name a single package are validated here, keeping
        // a glob reserved for future packages usable.
        for (pattern, ((negated, glob), matched)) in
            self.ignore.iter().zip(patterns.iter().zip(matched))
        {
            if !negated && !matched && glob.text().is_invariant() {
                bail!(
                    "package `{pattern}` is specified in the \"ignore\" option in .changeset/config.json but is not a workspace member"
                );
            }
        }
        Ok(resolved)
    }
}

fn parse_ignore_pattern(pattern: &str) -> Result<(bool, Glob<'_>)> {
    // wax does not parse the leading `!`, so it is stripped here and the
    // negation applied by resolve_ignore.
    let (negated, body) = match pattern.strip_prefix('!') {
        Some(body) => (true, body),
        None => (false, pattern),
    };
    let glob = Glob::new(body).with_context(|| format!("invalid ignore pattern {pattern:?}"))?;
    Ok((negated, glob))
}

/// Loads the supported subset of `changeset_dir/config.json`, treating a
/// missing file as all defaults and ignoring unknown keys.
pub(crate) fn load(changeset_dir: &Path) -> Result<Config> {
    let path = changeset_dir.join("config.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    load_text(&text).with_context(|| path.display().to_string())
}

fn load_text(text: &str) -> Result<Config> {
    let value: Value = serde_json::from_str(text)?;
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
            parse_ignore_pattern(pattern)?;
            ignore.push(pattern.to_owned());
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
        load_ok(text).resolve_ignore(names.iter().copied()).unwrap()
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

    #[test]
    fn a_missing_file_yields_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), Config::default());
    }

    #[test]
    fn accepts_an_empty_object() {
        assert_eq!(load_ok("{}\n"), Config::default());
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
    fn tolerates_an_ignore_glob_matching_nothing() {
        assert!(resolve("{ \"ignore\": [\"missing-*\"] }\n", &["pkg-a"]).is_empty());
    }

    #[test]
    fn tolerates_a_negated_ignore_literal_matching_nothing() {
        assert_eq!(
            resolve(
                "{ \"ignore\": [\"pkg-*\", \"!missing\"] }\n",
                &["pkg-a", "pkg-b"]
            ),
            ["pkg-a", "pkg-b"]
        );
    }

    #[test]
    fn rejects_an_ignore_literal_matching_nothing() {
        let config = load_ok("{ \"ignore\": [\"pkg-serverr\"] }\n");
        let err = config
            .resolve_ignore(["pkg-server", "pkg-client"])
            .unwrap_err();
        insta::assert_snapshot!(format!("{err:#}"));
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
