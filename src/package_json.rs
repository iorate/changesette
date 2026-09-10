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

pub struct PackageJson {
    path: PathBuf,
    root: CstRootNode,
    version_lit: Option<CstStringLit>,
}

impl PackageJson {
    pub fn load(dir: &Path) -> Result<Self> {
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

    pub fn set_version(&mut self, version: &semver::Version) -> Result<()> {
        let Some(version_lit) = &self.version_lit else {
            bail!("{}: missing top-level \"version\"", self.path.display())
        };
        set_string_value(version_lit, &version.to_string());
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.root.to_string()
    }
}
