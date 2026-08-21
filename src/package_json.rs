use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use jsonc_parser::{
    ParseOptions,
    cst::{CstRootNode, CstStringLit},
};

use crate::jsonc::{set_string_value, string_prop};

/// A loaded `package.json`. Saving preserves the original formatting,
/// changing only the rewritten values.
pub(crate) struct PackageJson {
    path: PathBuf,
    root: CstRootNode,
    name: String,
    version_lit: Option<CstStringLit>,
    version: Option<semver::Version>,
    private: bool,
}

impl PackageJson {
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("package.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                bail!("{} not found", path.display())
            }
            Err(err) => return Err(err).context(path.display().to_string()),
        };
        let context = path.display().to_string();
        Self::parse(path, &text).context(context)
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

        let version_lit = string_prop(&object, "version", "top-level \"version\"")?;
        let version = match &version_lit {
            Some(version_lit) => {
                let raw_version = version_lit
                    .decoded_value()
                    .context("top-level \"version\" must be a valid string")?;
                Some(raw_version.parse().with_context(|| {
                    format!("top-level \"version\" ({raw_version:?}) is not a valid semver version")
                })?)
            }
            None => None,
        };

        let private = match object.get("private") {
            Some(prop) => prop
                .value()
                .and_then(|value| value.as_boolean_lit())
                .context("top-level \"private\" must be a boolean")?
                .value(),
            None => false,
        };

        Ok(Self {
            path,
            root,
            name,
            version_lit,
            version,
            private,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn version(&self) -> Option<&semver::Version> {
        self.version.as_ref()
    }

    pub(crate) fn private(&self) -> bool {
        self.private
    }

    pub(crate) fn set_version(&mut self, version: &semver::Version) -> Result<()> {
        let Some(version_lit) = &self.version_lit else {
            bail!("{}: missing top-level \"version\"", self.path.display())
        };
        set_string_value(version_lit, &version.to_string());
        self.version = Some(version.clone());
        Ok(())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn text(&self) -> String {
        self.root.to_string()
    }
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
        fs::write(package_json.path(), package_json.text()).unwrap();
        fs::read_to_string(dir.path().join("package.json")).unwrap()
    }

    fn load_err(case: &str) -> String {
        format!("{:#}", PackageJson::load(&fixture(case)).err().unwrap())
    }

    #[test]
    fn reads_name_and_version() {
        let package_json = PackageJson::load(&fixture("two-space")).unwrap();
        assert_eq!(package_json.name(), "ublacklist");
        assert_eq!(
            package_json.version(),
            Some(&semver::Version::new(10, 0, 2))
        );
    }

    #[test]
    fn set_version_updates_the_read_value() {
        let mut package_json = PackageJson::load(&fixture("two-space")).unwrap();
        package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap();
        assert_eq!(
            package_json.version(),
            Some(&semver::Version::new(10, 1, 0))
        );
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
    fn reads_a_missing_version_as_none() {
        let package_json = PackageJson::load(&fixture("version-missing")).unwrap();
        assert_eq!(package_json.version(), None);
    }

    #[test]
    fn rejects_set_version_without_a_version_key() {
        let mut package_json = PackageJson::load(&fixture("version-missing")).unwrap();
        let err = package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap_err();
        insta::assert_snapshot!(format!("{err:#}"));
    }

    #[test]
    fn reads_private_true() {
        let package_json = PackageJson::load(&fixture("private-true")).unwrap();
        assert!(package_json.private());
    }

    #[test]
    fn reads_private_false() {
        let package_json = PackageJson::load(&fixture("private-false")).unwrap();
        assert!(!package_json.private());
    }

    #[test]
    fn reads_a_missing_private_as_false() {
        let package_json = PackageJson::load(&fixture("scripts-version-key")).unwrap();
        assert!(!package_json.private());
    }

    #[test]
    fn rejects_a_non_boolean_private() {
        insta::assert_snapshot!(load_err("private-not-boolean"));
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
