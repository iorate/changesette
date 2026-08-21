use std::{fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{output, plan, release_plan};

/// Consumes every changeset: bumps each named package's package.json,
/// upserts its CHANGELOG.md section, and deletes the consumed files — in pre
/// mode planning prerelease versions and moving the consumed files to
/// `.changeset/pre/` instead.
pub(crate) fn run(
    ignore: &[String],
    allow_no_changesets: bool,
    output_path: Option<&Path>,
) -> Result<()> {
    let planned = plan::plan_version(ignore)?;
    if let Some(pre) = planned.in_pre() {
        output::eprint_line(&format!(
            "warning: in pre mode with tag `{}`; versions will be prereleases. Run `changesette pre exit` first for a normal release.",
            pre.tag()
        ))?;
    }
    let in_pre = planned.in_pre().is_some();
    let exiting = planned.exiting_pre();
    if planned.no_changes && !exiting && !allow_no_changesets {
        bail!("no unreleased changesets found");
    }

    let pre_dir = planned.changeset_dir.join("pre");
    // Checked before the writes are applied: a rename failing afterwards
    // would leave the versions bumped with their changesets still pending.
    if in_pre {
        for change in &planned.changes {
            let path = pre_dir.join(&change.file_name);
            if path
                .try_exists()
                .with_context(|| path.display().to_string())?
            {
                bail!("{}: already exists; refusing to overwrite", path.display());
            }
        }
    }

    let writes = plan::stage_writes(&planned.workspace, &planned.releases)?;
    for write in &writes {
        write.apply()?;
    }

    if in_pre {
        if !planned.changes.is_empty() {
            fs::create_dir_all(&pre_dir).with_context(|| pre_dir.display().to_string())?;
        }
        for change in &planned.changes {
            let path = planned.changeset_dir.join(change.rel_path());
            let pre_path = pre_dir.join(&change.file_name);
            fs::rename(&path, &pre_path)
                .with_context(|| format!("{} -> {}", path.display(), pre_path.display()))?;
        }
    } else {
        for change in &planned.changes {
            let path = planned.changeset_dir.join(change.rel_path());
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
        // Deleted even with nothing to release, so that an exited pre mode
        // always ends here.
        if let Some(pre) = &planned.pre {
            fs::remove_file(pre.path()).with_context(|| pre.path().display().to_string())?;
        }
    }

    match output_path {
        Some(path) => release_plan::write_file(
            path,
            &release_plan::build(&planned.changes, &planned.releases, planned.pre.as_ref()),
        ),
        None => {
            if planned.no_changes && !exiting {
                return output::eprint_line("No unreleased changesets found.");
            }
            let mut bumped = false;
            for release in &planned.releases {
                if release.bump.is_some() {
                    output::eprint_line(&format!(
                        "Bumped {} {} -> {}",
                        release.name, release.old_version, release.new_version
                    ))?;
                    bumped = true;
                }
            }
            if !bumped && !planned.no_changes {
                output::eprint_line("No packages to bump.")?;
            }
            Ok(())
        }
    }
}
