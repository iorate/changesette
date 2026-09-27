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
        self.set_string(&[field.as_str().to_owned(), name.to_owned()], spec)
    }

    pub fn set_string(&mut self, path: &[String], value: &str) -> Result<()> {
        let (last, parents) = path.split_last().expect("a key path is not empty");
        let location = |key: &str, parent: Option<&String>| match parent {
            None => format!("top-level {key:?}"),
            Some(parent) => format!("{key:?} in {parent:?}"),
        };
        let mut object = self.object.clone();
        let mut parent = None;
        for key in parents {
            let location = location(key, parent);
            object = object_prop(&object, key, &location)
                .with_context(|| self.path.display().to_string())?
                .with_context(|| format!("{}: missing {location}", self.path.display()))?;
            parent = Some(key);
        }
        let location = location(last, parent);
        let lit = string_prop(&object, last, &location)
            .with_context(|| self.path.display().to_string())?
            .with_context(|| format!("{}: missing {location}", self.path.display()))?;
        set_string_value(&lit, value);
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
