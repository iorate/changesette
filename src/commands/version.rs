use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::{output::display_path, plan, release_plan, snapshot::Snapshot};

/// Consumes every changeset: bumps each named package's package.json,
/// upserts its CHANGELOG.md section, and deletes the consumed files — in pre
/// mode planning prerelease versions and moving the consumed files to
/// `.changeset/pre/` instead.
pub(crate) fn run(
    ignore: &[String],
    allow_no_changesets: bool,
    output_path: Option<&Path>,
    snapshot: Option<&Snapshot>,
) -> Result<()> {
    let planned = plan::plan_version(ignore, snapshot)?;
    let pre = planned.in_pre();
    if let Some(pre) = pre {
        info!(
            "In pre mode with tag `{}`; versions will be prereleases.",
            pre.tag()
        );
    }
    let in_pre = pre.is_some();
    let exiting = planned.exiting_pre();
    if planned.changes.is_empty() && !exiting && !allow_no_changesets {
        bail!("no unreleased changesets found");
    }

    let pre_dir = planned.changeset_dir.join("pre");
    // Checked before the writes are applied: a rename failing afterwards
    // would leave the versions bumped with their changesets still pending.
    if in_pre {
        for change in &planned.consumed_changes {
            let path = pre_dir.join(&change.file_name);
            if path.try_exists().with_context(|| display_path(&path))? {
                bail!(
                    "{}: already exists; refusing to overwrite",
                    display_path(&path)
                );
            }
        }
    }

    let writes = plan::stage_writes(&planned.workspace, &planned.releases)?;
    for write in &writes {
        write.apply()?;
    }

    if in_pre {
        if !planned.consumed_changes.is_empty() {
            fs::create_dir_all(&pre_dir).with_context(|| display_path(&pre_dir))?;
        }
        for change in &planned.consumed_changes {
            let path = planned.changeset_dir.join(change.rel_path());
            let pre_path = pre_dir.join(&change.file_name);
            fs::rename(&path, &pre_path).with_context(|| {
                format!("{} -> {}", display_path(&path), display_path(&pre_path))
            })?;
        }
    } else {
        for change in &planned.consumed_changes {
            let path = planned.changeset_dir.join(change.rel_path());
            fs::remove_file(&path).with_context(|| display_path(&path))?;
        }
        // Deleted even with nothing to release, so that an exited pre mode
        // always ends here; a snapshot run keeps it, leaving the exit to the
        // next regular `version` once the throwaway tree is discarded.
        if let Some(pre) = &planned.pre
            && snapshot.is_none()
        {
            fs::remove_file(pre.path()).with_context(|| display_path(pre.path()))?;
        }
    }

    match output_path {
        Some(path) => release_plan::write_file(
            path,
            &release_plan::build(&planned.changes, &planned.releases, planned.pre.as_ref()),
        ),
        None => {
            if planned.changes.is_empty() && !exiting {
                info!("No unreleased changesets found.");
                return Ok(());
            }
            let mut bumped = false;
            for release in &planned.releases {
                if release.bump.is_some() {
                    info!(
                        "Bumped {} {} -> {}",
                        release.name, release.old_version, release.new_version
                    );
                    bumped = true;
                }
            }
            if !bumped && !planned.changes.is_empty() {
                info!("No packages to bump.");
            }
            Ok(())
        }
    }
}
