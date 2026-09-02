use std::{fs, io, io::Write};

use anyhow::{Context, Result};
use tracing::info;

use crate::workspace::Workspace;

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

pub(crate) fn run(workspace: &Workspace) -> Result<()> {
    let changeset_dir = workspace.changeset_dir();
    fs::create_dir_all(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;

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
                    .with_context(|| path.display().to_string())?;
                info!("Created {}", path.display());
                created = true;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err).context(path.display().to_string()),
        }
    }

    if !created {
        info!("{} is already initialized", changeset_dir.display());
    }
    Ok(())
}
