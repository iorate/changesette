use std::{fs, io};

use anyhow::{Context, Result, bail};

use crate::{changelog, output, workspace::Workspace};

pub(crate) fn run(workspace: &Workspace, package: &str, version: &semver::Version) -> Result<()> {
    let member = workspace.member(package)?;
    let path = member.dir().join("CHANGELOG.md");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!("{} not found", path.display())
        }
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let section = changelog::extract_section(&text, &version.to_string())
        .with_context(|| path.display().to_string())?;
    output::print_line(&section)?;
    Ok(())
}
