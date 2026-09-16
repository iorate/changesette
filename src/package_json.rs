use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use jsonc_parser::{
    ParseOptions,
    cst::{CstObject, CstRootNode},
};

use crate::{
    jsonc::{object_prop, set_string_value, string_prop},
    workspace::DependencyField,
};

pub struct PackageJson {
    path: PathBuf,
    root: CstRootNode,
    object: CstObject,
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
        Ok(Self { path, root, object })
    }

    pub fn set_version(&mut self, version: &nodejs_semver::Version) -> Result<()> {
        let Some(version_lit) = string_prop(&self.object, "version", "top-level \"version\"")
            .with_context(|| self.path.display().to_string())?
        else {
            bail!("{}: missing top-level \"version\"", self.path.display())
        };
        set_string_value(&version_lit, &version.to_string());
        Ok(())
    }

    pub fn set_dependency(&mut self, field: DependencyField, name: &str, spec: &str) -> Result<()> {
        let field = field.as_str();
        let lit = object_prop(&self.object, field, &format!("top-level {field:?}"))
            .and_then(|deps| match deps {
                Some(deps) => string_prop(&deps, name, &format!("{name:?} in {field:?}")),
                None => Ok(None),
            })
            .with_context(|| self.path.display().to_string())?;
        let Some(lit) = lit else {
            bail!("{}: missing {name:?} in {field:?}", self.path.display())
        };
        set_string_value(&lit, spec);
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
