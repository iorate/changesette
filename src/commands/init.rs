use std::{env, fs};

use anyhow::{Context, Result};

use crate::{output, workspace::Workspace};

const README: &str = "# Changesets

This directory holds changeset files recording pending changes to this package,
managed by [changesette](https://github.com/iorate/changesette).
`changesette add` creates a changeset here; `changesette version` consumes all
pending changesets to bump the version and update CHANGELOG.md.
";

/// Creates the `.changeset/` directory at the workspace root with a README.md
/// describing it, and reports the creation to stdout; does nothing and prints
/// nothing if the directory already exists.
pub(crate) fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let workspace = Workspace::discover(&cwd)?;
    let changeset_dir = workspace.root().join(".changeset");
    if changeset_dir.is_dir() {
        return Ok(());
    }
    fs::create_dir(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;
    let readme_path = changeset_dir.join("README.md");
    fs::write(&readme_path, README).with_context(|| readme_path.display().to_string())?;
    output::print_line(&format!(
        "Initialized {}",
        workspace.display_path(&cwd, &changeset_dir).display()
    ))?;
    Ok(())
}
