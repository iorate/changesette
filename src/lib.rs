use std::path::Path;

use anyhow::{Context, Result};

use crate::{
    config::Config,
    workspace::{Root, Workspace},
};

pub mod bump;
pub mod changelog;
pub mod changeset;
pub mod commands;
pub mod config;
pub mod dependency;
mod jsonc;
pub mod output;
pub mod package_json;
pub mod plan;
pub mod pre;
pub mod range;
pub mod release_plan;
pub mod snapshot;
pub mod workspace;

pub fn load(cwd: &Path, root: Option<&Path>, cli_ignore: &[String]) -> Result<(Workspace, Config)> {
    let root = if let Some(dir) = root {
        let dir = workspace::resolve_root(dir)
            .with_context(|| format!("invalid root directory {}", dir.display()))?;
        Root::new(dir)
    } else {
        let cwd = workspace::resolve_root(cwd)
            .with_context(|| format!("invalid working directory {}", cwd.display()))?;
        Root::find(&cwd)?
    };
    let config = config::load(&root.dir().join(".changeset"))?;
    let workspace = Workspace::load(root, &config, cli_ignore)?;
    Ok((workspace, config))
}
