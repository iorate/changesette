use anyhow::{Result, bail};

/// Always fails: consuming changesets is not implemented yet.
pub(crate) fn run() -> Result<()> {
    bail!("the version command is not implemented yet")
}
