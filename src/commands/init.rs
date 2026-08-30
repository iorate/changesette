use std::{fs, io, io::Write, path::Path};

use anyhow::{Context, Result};
use tracing::info;

use crate::{output::display_path, workspace::Workspace};

const README: &str = "# Changesets

This directory holds changeset files recording pending changes to the packages
in this repository, managed by
[changesette](https://github.com/iorate/changesette).
`changesette add` creates a changeset here; `changesette version` consumes all
pending changesets to bump each released package's version and update its
CHANGELOG.md.
";

const CONFIG: &str = "{
  \"fixed\": [],
  \"linked\": [],
  \"privatePackages\": {
    \"version\": false
  },
  \"ignore\": [],
  \"snapshot\": {
    \"useCalculatedVersion\": false
  }
}
";

/// Creates the `.changeset/` directory at the workspace root with a default
/// README.md and config.json, creating whichever of them are missing.
pub(crate) fn run(cwd: &Path, workspace: &Workspace) -> Result<()> {
    let changeset_dir = workspace.root().join(".changeset");
    fs::create_dir_all(&changeset_dir).with_context(|| display_path(&changeset_dir))?;

    let mut created = false;
    for (file_name, content) in [("README.md", README), ("config.json", CONFIG)] {
        let path = changeset_dir.join(file_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())
                    .with_context(|| display_path(&path))?;
                info!("Created {}", workspace.display_path(cwd, &path));
                created = true;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err).context(display_path(&path)),
        }
    }

    if !created {
        info!(
            "{} is already initialized",
            workspace.display_path(cwd, &changeset_dir)
        );
    }
    Ok(())
}
