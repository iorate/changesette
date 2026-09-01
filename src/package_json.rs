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

/// A loaded `package.json` whose serialization preserves the original
/// formatting, changing only the rewritten values.
pub(crate) struct PackageJson {
    path: PathBuf,
    root: CstRootNode,
    version_lit: Option<CstStringLit>,
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
        if let Some(version_lit) = &version_lit {
            let raw_version = version_lit
                .decoded_value()
                .context("top-level \"version\" must be a valid string")?;
            raw_version.parse::<semver::Version>().with_context(|| {
                format!("top-level \"version\" ({raw_version:?}) is not a valid semver version")
            })?;
        }

        Ok(Self {
            path,
            root,
            version_lit,
        })
    }

    pub(crate) fn set_version(&mut self, version: &semver::Version) -> Result<()> {
        let Some(version_lit) = &self.version_lit else {
            bail!("{}: missing top-level \"version\"", self.path.display())
        };
        set_string_value(version_lit, &version.to_string());
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
        format!("{:#}", PackageJson::load(&fixture(case)).err().unwrap()).replace('\\', "/")
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
    fn rejects_set_version_without_a_version_key() {
        let mut package_json = PackageJson::load(&fixture("version-missing")).unwrap();
        let err = package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap_err();
        insta::assert_snapshot!(format!("{err:#}").replace('\\', "/"));
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
