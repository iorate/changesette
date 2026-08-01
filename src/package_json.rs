use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use jsonc_parser::{
    ParseOptions,
    cst::{CstObject, CstRootNode, CstStringLit},
};

/// A loaded `package.json`. Saving preserves the original formatting,
/// changing only the rewritten values.
pub(crate) struct PackageJson {
    path: PathBuf,
    root: CstRootNode,
    name: String,
    version_lit: CstStringLit,
    version: semver::Version,
}

impl PackageJson {
    /// Loads `package.json` under `dir`, validating its top-level `name` and
    /// `version`.
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("package.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                bail!("no package.json found in the current directory")
            }
            Err(err) => return Err(err).context(path.display().to_string()),
        };
        Self::parse(path, &text).context("package.json")
    }

    fn parse(path: PathBuf, text: &str) -> Result<Self> {
        let root = CstRootNode::parse(text, &ParseOptions::default())?;
        let object = root
            .object_value()
            .context("the root value must be an object")?;

        let name_lit = string_prop(&object, "name", "top-level \"name\"")?
            .context("missing top-level \"name\"")?;
        let name = name_lit
            .decoded_value()
            .context("top-level \"name\" must be a valid string")?;
        ensure!(!name.is_empty(), "top-level \"name\" must not be empty");

        let version_lit = string_prop(&object, "version", "top-level \"version\"")?
            .context("missing top-level \"version\"")?;
        let raw_version = version_lit
            .decoded_value()
            .context("top-level \"version\" must be a valid string")?;
        let version = raw_version.parse().with_context(|| {
            format!("top-level \"version\" ({raw_version:?}) is not a valid semver version")
        })?;

        Ok(Self {
            path,
            root,
            name,
            version_lit,
            version,
        })
    }

    /// The top-level `name`.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The current top-level `version`.
    pub(crate) fn version(&self) -> &semver::Version {
        &self.version
    }

    /// Sets the top-level `version`.
    pub(crate) fn set_version(&mut self, version: &semver::Version) -> Result<()> {
        set_string_value(&self.version_lit, &version.to_string());
        self.version = version.clone();
        Ok(())
    }

    /// Writes the possibly modified text back to the file it was loaded from.
    pub(crate) fn save(&self) -> Result<()> {
        fs::write(&self.path, self.root.to_string())
            .with_context(|| self.path.display().to_string())
    }
}

// Returns the string literal at `object[key]`, or `Ok(None)` if the key is
// absent; a non-string value is an error naming `location`.
fn string_prop(object: &CstObject, key: &str, location: &str) -> Result<Option<CstStringLit>> {
    let Some(prop) = object.get(key) else {
        return Ok(None);
    };
    let lit = prop
        .value()
        .and_then(|value| value.as_string_lit())
        .with_context(|| format!("{location} must be a string"))?;
    Ok(Some(lit))
}

// Replaces the literal's raw text with `"<value>"`; `value` must not contain
// characters that need escaping (semver versions never do).
fn set_string_value(lit: &CstStringLit, value: &str) {
    lit.set_raw_value(format!("\"{value}\""));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> PathBuf {
        Path::new("tests/fixtures/package-json").join(case)
    }

    fn rewrite(case: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        fs::copy(
            fixture(case).join("package.json"),
            dir.path().join("package.json"),
        )
        .unwrap();
        let mut package_json = PackageJson::load(dir.path()).unwrap();
        package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap();
        package_json.save().unwrap();
        fs::read_to_string(dir.path().join("package.json")).unwrap()
    }

    fn load_err(case: &str) -> String {
        format!("{:#}", PackageJson::load(&fixture(case)).err().unwrap())
    }

    #[test]
    fn reads_name_and_version() {
        let package_json = PackageJson::load(&fixture("two-space")).unwrap();
        assert_eq!(package_json.name(), "ublacklist");
        assert_eq!(package_json.version(), &semver::Version::new(10, 0, 2));
    }

    #[test]
    fn set_version_updates_the_read_value() {
        let mut package_json = PackageJson::load(&fixture("two-space")).unwrap();
        package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap();
        assert_eq!(package_json.version(), &semver::Version::new(10, 1, 0));
    }

    #[test]
    fn rewrites_a_two_space_indented_file() {
        insta::assert_snapshot!(rewrite("two-space"));
    }

    #[test]
    fn rewrites_a_four_space_indented_file_without_a_final_newline() {
        let rewritten = rewrite("four-space-no-final-newline");
        assert!(!rewritten.ends_with('\n'));
        insta::assert_snapshot!(rewritten);
    }

    #[test]
    fn rewrites_a_tab_indented_file() {
        insta::assert_snapshot!(rewrite("tabs"));
    }

    #[test]
    fn leaves_nested_version_keys_untouched() {
        insta::assert_snapshot!(rewrite("scripts-version-key"));
    }

    #[test]
    fn rejects_a_missing_package_json() {
        insta::assert_snapshot!(load_err("no-package-json"));
    }

    #[test]
    fn rejects_a_missing_version() {
        insta::assert_snapshot!(load_err("version-missing"));
    }

    #[test]
    fn rejects_a_non_string_version() {
        insta::assert_snapshot!(load_err("version-not-string"));
    }

    #[test]
    fn rejects_an_invalid_semver_version() {
        insta::assert_snapshot!(load_err("version-invalid-semver"));
    }

    #[test]
    fn rejects_a_missing_name() {
        insta::assert_snapshot!(load_err("name-missing"));
    }
}
