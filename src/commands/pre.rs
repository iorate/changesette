use std::{env, fs};

use anyhow::{Context, Result, bail};

use crate::{
    output,
    pre::{self, PreJson, PreMode},
    workspace::Workspace,
};

/// Enters pre-release mode by writing `.changeset/pre.json` with `tag`; it
/// is an error to already be in pre mode.
pub(crate) fn enter(tag: &str) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let pre = PreJson::load(&changeset_dir)?;

    // Checked before the tag, so that an existing pre.json with an invalid
    // tag still reports the state rather than the tag.
    if matches!(&pre, Some(pre) if pre.mode() == PreMode::Pre) {
        bail!("already in pre mode; run `changesette pre exit` to exit");
    }
    pre::validate_tag(tag)?;

    match pre {
        Some(mut pre) => {
            pre.set_mode(PreMode::Pre);
            pre.set_tag(tag);
            fs::write(pre.path(), pre.text()).with_context(|| pre.path().display().to_string())?;
        }
        None => {
            fs::create_dir_all(&changeset_dir)
                .with_context(|| changeset_dir.display().to_string())?;
            pre::write_new(&changeset_dir, tag)?;
        }
    }

    output::eprint_line(&format!(
        "Entered pre mode with tag `{tag}`\nRun `changesette version` to bump to prerelease versions"
    ))
}

/// Exits pre-release mode by flipping `.changeset/pre.json` to the exited
/// state; it is an error not to have a pre.json, while exiting twice
/// succeeds.
pub(crate) fn exit() -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let Some(mut pre) = PreJson::load(&changeset_dir)? else {
        bail!("not in pre mode; run `changesette pre enter <tag>` to enter");
    };

    pre.set_mode(PreMode::Exit);
    fs::write(pre.path(), pre.text()).with_context(|| pre.path().display().to_string())?;

    output::eprint_line("Exited pre mode\nRun `changesette version` to bump to final versions")
}
