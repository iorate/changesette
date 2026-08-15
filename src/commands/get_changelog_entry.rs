use std::{fs, io};

use anyhow::{Context, Result, bail};

use crate::{changelog, workspace::Workspace};

/// Prints the section of the given version from the named package's
/// CHANGELOG.md to stdout.
pub(crate) fn run(package: &str, version: &semver::Version) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
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
    println!("{section}");
    Ok(())
}
