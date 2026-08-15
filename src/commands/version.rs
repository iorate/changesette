use std::{env, fs, path::Path};

use anyhow::{Context, Result};

use crate::{changeset, output, release_plan, workspace::Workspace};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, and deletes
/// the consumed files (`none`-only and empty changesets included). Packages
/// named only with `none` keep their version and changelog; with zero
/// changesets, nothing changes. Prints a completion message to stdout; with
/// `output_path`, prints nothing and instead writes the release plan, each
/// bumped release carrying its new changelog entry, to that file as
/// pretty-printed JSON.
pub(crate) fn run(output_path: Option<&Path>) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let changes = changeset::load(&changeset_dir)?;
    let (plan, writes) = release_plan::compute(&workspace, &changes)?;

    for write in writes {
        write.package_json.save()?;
        fs::write(&write.changelog_path, write.changelog_text)
            .with_context(|| write.changelog_path.display().to_string())?;
    }
    for change in &changes {
        let path = changeset_dir.join(&change.file_name);
        fs::remove_file(&path).with_context(|| path.display().to_string())?;
    }

    match output_path {
        Some(path) => release_plan::write_file(path, &plan),
        None => output::print_line(if changes.is_empty() {
            "No unreleased changesets found."
        } else {
            "All files have been updated. Review them and commit at your leisure"
        }),
    }
}
