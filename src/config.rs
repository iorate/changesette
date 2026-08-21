use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// The effective settings from `.changeset/config.json`.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct Config {
    /// Whether private packages are versioned, per the changesets
    /// `privatePackages` setting: `true` means `{version: true}`, and a
    /// missing key, `false`, or an object without `version` means
    /// `{version: false}`, matching the upstream @changesets/config@4.0.0
    /// defaults.
    pub(crate) private_packages_version: bool,
}

/// Loads `changeset_dir/config.json` as the changesette-supported subset of
/// the changesets config format: "ignore" must be an array of strings, and
/// "privatePackages" a boolean or an object whose optional "version" is a
/// boolean. Unknown keys are ignored, as in upstream changesets. A missing
/// file is valid; every setting then takes its default.
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

    if let Some(ignore) = object.get("ignore") {
        let valid = ignore
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string));
        if !valid {
            bail!("\"ignore\" must be an array of strings")
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
    fn accepts_every_supported_shape() {
        load_ok(
            "{\n  \"ignore\": [\"pkg-a\", \"@scope/*\"],\n  \"privatePackages\": {\n    \"version\": true\n  }\n}\n",
        );
        load_ok("{ \"ignore\": [] }\n");
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
