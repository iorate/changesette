use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Validates `changeset_dir/config.json` as the changesette-supported subset
/// of the changesets config format: "ignore" must be an array of strings,
/// and "privatePackages" a boolean or an object whose optional "version" is
/// a boolean. Unknown keys are ignored, as in upstream changesets. A missing
/// file is valid; every setting then takes its default.
pub(crate) fn validate(changeset_dir: &Path) -> Result<()> {
    let path = changeset_dir.join("config.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    validate_text(&text).with_context(|| path.display().to_string())
}

fn validate_text(text: &str) -> Result<()> {
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

    if let Some(private_packages) = object.get("privatePackages") {
        match private_packages {
            Value::Bool(_) => {}
            Value::Object(object) => {
                if let Some(version) = object.get("version") {
                    if !version.is_boolean() {
                        bail!("\"version\" in \"privatePackages\" must be a boolean")
                    }
                }
            }
            _ => bail!("\"privatePackages\" must be a boolean or an object"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_ok(text: &str) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), text).unwrap();
        validate(dir.path()).unwrap();
    }

    fn validate_err(text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), text).unwrap();
        let err = validate(dir.path()).err().unwrap();
        format!("{err:#}").replace(
            &dir.path().join("config.json").display().to_string(),
            ".changeset/config.json",
        )
    }

    #[test]
    fn accepts_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        validate(dir.path()).unwrap();
    }

    #[test]
    fn accepts_an_empty_object() {
        validate_ok("{}\n");
    }

    #[test]
    fn accepts_every_supported_shape() {
        validate_ok(
            "{\n  \"ignore\": [\"pkg-a\", \"@scope/*\"],\n  \"privatePackages\": {\n    \"version\": true\n  }\n}\n",
        );
        validate_ok("{ \"privatePackages\": true }\n");
        validate_ok("{ \"privatePackages\": false }\n");
        validate_ok("{ \"privatePackages\": {} }\n");
        validate_ok("{ \"ignore\": [] }\n");
    }

    #[test]
    fn ignores_unknown_keys() {
        validate_ok(
            "{\n  \"$schema\": \"https://unpkg.com/@changesets/config@4.0.0/schema.json\",\n  \"changelog\": \"@changesets/cli/changelog\",\n  \"commit\": false,\n  \"privatePackages\": {\n    \"version\": true,\n    \"tag\": false\n  }\n}\n",
        );
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
