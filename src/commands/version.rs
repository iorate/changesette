use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::{config::Config, plan, release_plan, snapshot::Snapshot, workspace::Workspace};

#[derive(clap::Args)]
pub(crate) struct VersionArgs {
    /// The packages to skip, leaving their changesets in place (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    pub(crate) ignore: Vec<String>,
    /// Create a snapshot release: bump to throwaway `0.0.0-<suffix>` versions instead
    #[arg(
        long,
        value_name = "TAG",
        num_args = 0..=1,
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    #[expect(clippy::option_option)]
    pub(crate) snapshot: Option<Option<String>>,
    /// The snapshot suffix template; the placeholders are {tag}, {timestamp}, and {datetime}
    #[arg(
        long,
        value_name = "TEMPLATE",
        requires = "snapshot",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub(crate) snapshot_prerelease_template: Option<String>,
    /// Succeed even when there are no unreleased changesets
    #[arg(short, long)]
    pub(crate) allow_no_changesets: bool,
    /// Write the release plan to the file (or stdout with `-`) as JSON
    #[arg(short, long, value_name = "FILE")]
    pub(crate) output: Option<PathBuf>,
}

pub(crate) fn run(workspace: Workspace, config: &Config, args: VersionArgs) -> Result<()> {
    let snapshot = args.snapshot.map(|tag| Snapshot {
        tag,
        template: args.snapshot_prerelease_template,
    });
    let planned = plan::plan_version(workspace, config, &args.ignore, snapshot.as_ref())?;
    let pre = planned.in_pre();
    if let Some(pre) = pre {
        info!(
            "In pre mode with tag `{}`; versions will be prereleases.",
            pre.tag()
        );
    }
    let in_pre = pre.is_some();
    let exiting = planned.exiting_pre();
    if planned.changes.is_empty() && !exiting && !args.allow_no_changesets {
        bail!("no unreleased changesets found");
    }

    let pre_dir = planned.changeset_dir.join("pre");
    // Checked before the writes are applied: a rename failing afterwards
    // would leave the versions bumped with their changesets still pending.
    if in_pre {
        for change in &planned.consumed_changes {
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
        if !planned.consumed_changes.is_empty() {
            fs::create_dir_all(&pre_dir).with_context(|| pre_dir.display().to_string())?;
        }
        for change in &planned.consumed_changes {
            let path = planned.changeset_dir.join(change.rel_path());
            let pre_path = pre_dir.join(&change.file_name);
            fs::rename(&path, &pre_path)
                .with_context(|| format!("{} -> {}", path.display(), pre_path.display()))?;
        }
    } else {
        for change in &planned.consumed_changes {
            let path = planned.changeset_dir.join(change.rel_path());
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
        // Deleted even with nothing to release, so that an exited pre mode
        // always ends here; a snapshot run keeps it, leaving the exit to the
        // next regular `version` once the throwaway tree is discarded.
        if let Some(pre) = &planned.pre
            && snapshot.is_none()
        {
            fs::remove_file(pre.path()).with_context(|| pre.path().display().to_string())?;
        }
    }

    if let Some(path) = &args.output {
        return release_plan::write_file(
            path,
            &release_plan::build(&planned.changes, &planned.releases, planned.pre.as_ref()),
        );
    }

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
