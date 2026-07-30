use std::{fs, io};

use anyhow::{Context, Result, bail};

use crate::changelog::document;

/// Prints the section of the given version (or the latest one when omitted)
/// from the working tree's CHANGELOG.md to stdout.
pub(crate) fn run(version: Option<String>) -> Result<()> {
    let text = match fs::read_to_string("CHANGELOG.md") {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => bail!("CHANGELOG.md not found"),
        Err(err) => return Err(err).context("CHANGELOG.md"),
    };
    println!("{}", document::extract_section(&text, version.as_deref())?);
    Ok(())
}
