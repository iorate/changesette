use std::{fs, io};

use anyhow::{Context, Result, bail};

use crate::{
    changelog,
    output::{self, display_path},
    workspace::Workspace,
};

/// Prints the body of the given version's section, without the `## <version>`
/// heading, from the named package's CHANGELOG.md to stdout.
pub(crate) fn run(package: &str, version: &semver::Version) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let member = workspace.member(package)?;
    let path = member.dir().join("CHANGELOG.md");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!("{} not found", display_path(&path))
        }
        Err(err) => return Err(err).context(display_path(&path)),
    };
    let section = changelog::extract_section(&text, &version.to_string())
        .with_context(|| display_path(&path))?;
    output::print_line(&section)?;
    Ok(())
}
