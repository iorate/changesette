use std::fs;

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::{
    pre::{self, PreJson, PreMode},
    workspace::Workspace,
};

pub(crate) fn enter(workspace: &Workspace, tag: &str) -> Result<()> {
    let changeset_dir = workspace.changeset_dir();
    let pre = PreJson::load(&changeset_dir)?;

    // Checked before the tag, so that an existing pre.json with an invalid
    // tag still reports the state rather than the tag.
    if matches!(&pre, Some(pre) if pre.mode() == PreMode::Pre) {
        bail!("already in pre mode; run `changesette pre exit` to exit");
    }
    pre::validate_tag(tag)?;

    if let Some(mut pre) = pre {
        pre.set_mode(PreMode::Pre);
        pre.set_tag(tag);
        fs::write(pre.path(), pre.text()).with_context(|| pre.path().display().to_string())?;
    } else {
        fs::create_dir_all(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;
        pre::write_new(&changeset_dir, tag)?;
    }

    info!(
        "Entered pre mode with tag `{tag}`\nRun `changesette version` to bump to prerelease versions"
    );
    Ok(())
}

// Exiting twice is deliberately not an error.
pub(crate) fn exit(workspace: &Workspace) -> Result<()> {
    let changeset_dir = workspace.changeset_dir();
    let Some(mut pre) = PreJson::load(&changeset_dir)? else {
        bail!("not in pre mode; run `changesette pre enter <tag>` to enter");
    };

    pre.set_mode(PreMode::Exit);
    fs::write(pre.path(), pre.text()).with_context(|| pre.path().display().to_string())?;

    info!("Exited pre mode\nRun `changesette version` to bump to final versions");
    Ok(())
}
