use std::{env, fs};

use anyhow::{Context, Result};

use crate::{changeset, output, release_plan, workspace::Workspace};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, deletes the
/// consumed files (`none`-only and empty changesets included), and prints the
/// release plan to stdout as single-line JSON, each bumped release carrying
/// its new changelog entry. Packages named only with
/// `none` keep their version and changelog. With zero changesets, prints an
/// empty plan and touches nothing. With `dry_run`, prints the same JSON but
/// touches no file.
pub(crate) fn run(dry_run: bool) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let changes = changeset::load(&changeset_dir)?;
    let (plan, writes) = release_plan::compute(&workspace, &changes)?;

    if changes.is_empty() {
        eprintln!("note: no changesets found; nothing to do");
    }
    if dry_run {
        eprintln!("dry run: no files will be modified");
    } else {
        for write in writes {
            write.package_json.save()?;
            fs::write(&write.changelog_path, write.changelog_text)
                .with_context(|| write.changelog_path.display().to_string())?;
        }
        for change in &changes {
            let path = changeset_dir.join(&change.file_name);
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
    }

    output::print_line(&serde_json::to_string(&plan)?)?;
    Ok(())
}
