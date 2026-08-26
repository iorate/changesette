use std::{env, fs};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::{changeset, output::display_path, workspace::Workspace};

/// Rewrites the summary of the changeset with the given id, re-rendering the
/// file in the canonical form `add` writes.
pub(crate) fn run(id: &str, summary: &str) -> Result<()> {
    let cwd = env::current_dir()?;
    let workspace = Workspace::discover(&cwd)?;
    let changeset_dir = workspace.root().join(".changeset");

    let changes = changeset::load(&changeset_dir)?;
    let Some(change) = changes.iter().find(|change| change.id() == id) else {
        bail!("no changeset with id `{id}`")
    };

    let content = changeset::render(&change.releases, summary)?;
    let path = changeset_dir.join(change.rel_path());
    fs::write(&path, content).with_context(|| display_path(&path))?;

    info!("Updated {}", workspace.display_path(&cwd, &path));
    Ok(())
}
