use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use jsonc_parser::{
    ParseOptions,
    cst::{CstRootNode, CstStringLit},
};

use crate::package_json::{set_string_value, string_prop};

/// A loaded `package-lock.json`. Saving preserves the original formatting,
/// changing only the rewritten values.
pub(crate) struct PackageLock {
    path: PathBuf,
    root: CstRootNode,
    version_lits: Vec<CstStringLit>,
}

impl PackageLock {
    /// Loads `package-lock.json` under `dir`; a missing file yields
    /// `Ok(None)`.
    pub(crate) fn load(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join("package-lock.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context(path.display().to_string()),
        };
        Self::parse(path, &text)
            .context("package-lock.json")
            .map(Some)
    }

    fn parse(path: PathBuf, text: &str) -> Result<Self> {
        let root = CstRootNode::parse(text, &ParseOptions::default())?;
        let object = root
            .object_value()
            .context("the root value must be an object")?;

        let mut version_lits = Vec::new();
        if let Some(lit) = string_prop(&object, "version", "top-level \"version\"")? {
            version_lits.push(lit);
        }
        if let Some(root_entry) = object
            .object_value("packages")
            .and_then(|packages| packages.object_value(""))
            && let Some(lit) = string_prop(&root_entry, "version", "packages.\"\".version")?
        {
            version_lits.push(lit);
        }

        Ok(Self {
            path,
            root,
            version_lits,
        })
    }

    /// Sets the top-level `version` and the `packages.""` entry's `version`,
    /// whichever are present.
    pub(crate) fn set_version(&mut self, version: &semver::Version) -> Result<()> {
        for lit in &self.version_lits {
            set_string_value(lit, &version.to_string());
        }
        Ok(())
    }

    /// Writes the possibly modified text back to the file it was loaded from.
    pub(crate) fn save(&self) -> Result<()> {
        fs::write(&self.path, self.root.to_string())
            .with_context(|| self.path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> PathBuf {
        Path::new("tests/fixtures/package-lock").join(case)
    }

    fn rewrite(case: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        fs::copy(
            fixture(case).join("package-lock.json"),
            dir.path().join("package-lock.json"),
        )
        .unwrap();
        let mut package_lock = PackageLock::load(dir.path()).unwrap().unwrap();
        package_lock
            .set_version(&semver::Version::new(10, 1, 0))
            .unwrap();
        package_lock.save().unwrap();
        fs::read_to_string(dir.path().join("package-lock.json")).unwrap()
    }

    fn load_err(case: &str) -> String {
        format!("{:#}", PackageLock::load(&fixture(case)).err().unwrap())
    }

    #[test]
    fn rewrites_both_version_fields_of_a_v3_lockfile() {
        insta::assert_snapshot!(rewrite("v3"));
    }

    #[test]
    fn rewrites_only_the_top_level_version_of_a_v1_lockfile() {
        insta::assert_snapshot!(rewrite("v1"));
    }

    #[test]
    fn returns_none_when_the_file_does_not_exist() {
        assert!(PackageLock::load(&fixture("absent")).unwrap().is_none());
    }

    #[test]
    fn rejects_a_non_object_root() {
        insta::assert_snapshot!(load_err("root-not-object"));
    }

    #[test]
    fn rejects_a_non_string_version() {
        insta::assert_snapshot!(load_err("version-not-string"));
    }
}
