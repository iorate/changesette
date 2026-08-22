use std::{env, fs, io, io::Write};

use anyhow::{Context, Result};

use crate::{output, workspace::Workspace};

const README: &str = "# Changesets

This directory holds changeset files recording pending changes to this package,
managed by [changesette](https://github.com/iorate/changesette).
`changesette add` creates a changeset here; `changesette version` consumes all
pending changesets to bump the version and update CHANGELOG.md.
";

const CONFIG: &str = "{
  \"fixed\": [],
  \"linked\": [],
  \"privatePackages\": {
    \"version\": false
  },
  \"ignore\": []
}
";

/// Creates the `.changeset/` directory at the workspace root with a default
/// README.md and config.json, creating whichever of them are missing.
pub(crate) fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let workspace = Workspace::discover(&cwd)?;
    let changeset_dir = workspace.root().join(".changeset");
    fs::create_dir_all(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;

    let mut lines = Vec::new();
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
                lines.push(format!(
                    "Created {}",
                    workspace.display_path(&cwd, &path).display()
                ));
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err).context(path.display().to_string()),
        }
    }

    if !lines.is_empty() {
        output::eprint_line(&lines.join("\n"))?;
    }
    Ok(())
}
