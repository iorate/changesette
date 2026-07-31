use std::{fs, io};

use anyhow::{Context, Result, bail};

use crate::changelog;

/// Prints the section of the given version from the working tree's
/// CHANGELOG.md to stdout.
pub(crate) fn run(version: &str) -> Result<()> {
    let text = match fs::read_to_string("CHANGELOG.md") {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => bail!("CHANGELOG.md not found"),
        Err(err) => return Err(err).context("CHANGELOG.md"),
    };
    println!("{}", changelog::extract_section(&text, version)?);
    Ok(())
}
