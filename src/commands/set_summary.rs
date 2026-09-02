use std::fs;

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::{changeset, workspace::Workspace};

pub(crate) fn run(workspace: &Workspace, id: &str, summary: &str) -> Result<()> {
    let changeset_dir = workspace.changeset_dir();

    let changes = changeset::load(&changeset_dir)?;
    let Some(change) = changes.iter().find(|change| change.id() == id) else {
        bail!("no changeset with id `{id}`")
    };

    let content = changeset::render(&change.releases, summary)?;
    let path = changeset_dir.join(change.rel_path());
    fs::write(&path, content).with_context(|| path.display().to_string())?;

    info!("Updated {}", path.display());
    Ok(())
}
