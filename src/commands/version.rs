use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    changeset, config, output, plan,
    pre::{self, PreJson, PreMode},
    release_plan,
    skip::SkipSet,
    workspace::{Member, Workspace},
};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, and deletes
/// the consumed files (`none`-only and empty changesets included). Packages
/// named only with `none` keep their version and changelog. With zero
/// changesets it is an error unless `allow_no_changesets` is set, and nothing
/// changes either way; exiting pre mode is exempt from the error. A package
/// is skipped when the ignore set names it, when it is private and the
/// config does not version private packages, or when its package.json has no
/// version field; a changeset naming only skipped packages is excluded from
/// the release plan and left on disk, and it is an error for a changeset to
/// mix skipped and not skipped packages. The ignore set is the config
/// `ignore` patterns resolved against the workspace members, or the `ignore`
/// argument (each name must be a workspace member) when that resolution is
/// empty; passing `ignore` while the resolution is not empty is an error.
///
/// In pre mode, only the changesets not yet consumed in this pre-release
/// cycle are planned, the new versions are prereleases, and the consumed
/// files are moved to `.changeset/pre/` instead of deleted. Exiting pre mode,
/// the parked files are replanned along with the new ones into final versions
/// and pre.json is deleted, even when there is nothing to release.
///
/// Reports the applied bumps to stderr; with `output_path`, instead writes
/// the release plan, each bumped release carrying its new changelog entry,
/// as pretty-printed JSON to that file (or to stdout when the path is `-`).
pub(crate) fn run(
    ignore: &[String],
    allow_no_changesets: bool,
    output_path: Option<&Path>,
) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let config = config::load(&changeset_dir)?;
    let config_ignore = config.resolve_ignore(workspace.members().iter().map(Member::name))?;
    let ignore = if config_ignore.is_empty() {
        for name in ignore {
            workspace.member(name).context("invalid `--ignore` value")?;
        }
        ignore.to_vec()
    } else if ignore.is_empty() {
        config_ignore
    } else {
        bail!(
            "the --ignore option cannot be used while ignore is defined in .changeset/config.json; use only one of them"
        );
    };
    let skip = SkipSet::build(&workspace, &config, &ignore)?;

    let pre = PreJson::load(&changeset_dir)?;
    let in_pre = match &pre {
        Some(pre) if pre.mode() == PreMode::Pre => {
            pre::validate_tag(pre.tag())?;
            output::eprint_line(&format!(
                "warning: in pre mode with tag `{}`; versions will be prereleases. Run `changesette pre exit` first for a normal release.",
                pre.tag()
            ))?;
            true
        }
        _ => false,
    };
    let exiting = matches!(&pre, Some(pre) if pre.mode() == PreMode::Exit);

    let mut changes = changeset::load(&changeset_dir)?;
    if in_pre {
        changes.retain(|change| !change.in_pre);
    }
    let no_changes = changes.is_empty();
    if no_changes && !exiting && !allow_no_changesets {
        bail!("no unreleased changesets found");
    }

    let consumed = skip.filter_changes(&workspace, &changeset_dir, changes)?;

    let releases = plan::plan_releases(&workspace, &consumed, pre.as_ref(), &skip)?;

    let pre_dir = changeset_dir.join("pre");
    // Checked before the writes are applied: a rename failing afterwards
    // would leave the versions bumped with their changesets still pending.
    if in_pre {
        for change in &consumed {
            let path = pre_dir.join(&change.file_name);
            if path
                .try_exists()
                .with_context(|| path.display().to_string())?
            {
                bail!("{}: already exists; refusing to overwrite", path.display());
            }
        }
    }

    let writes = plan::stage_writes(&workspace, &releases)?;
    for write in &writes {
        write.apply()?;
    }

    if in_pre {
        if !consumed.is_empty() {
            fs::create_dir_all(&pre_dir).with_context(|| pre_dir.display().to_string())?;
        }
        for change in &consumed {
            let path = changeset_dir.join(change.rel_path());
            let pre_path = pre_dir.join(&change.file_name);
            fs::rename(&path, &pre_path)
                .with_context(|| format!("{} -> {}", path.display(), pre_path.display()))?;
        }
    } else {
        for change in &consumed {
            let path = changeset_dir.join(change.rel_path());
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
        // Deleted even with nothing to release, so that an exited pre mode
        // always ends here.
        if let Some(pre) = &pre {
            fs::remove_file(pre.path()).with_context(|| pre.path().display().to_string())?;
        }
    }

    match output_path {
        Some(path) => release_plan::write_file(
            path,
            &release_plan::build(&consumed, &releases, pre.as_ref()),
        ),
        None => {
            if no_changes && !exiting {
                return output::eprint_line("No unreleased changesets found.");
            }
            let mut bumped = false;
            for release in &releases {
                if release.bump.is_some() {
                    output::eprint_line(&format!(
                        "Bumped {} {} -> {}",
                        release.name, release.old_version, release.new_version
                    ))?;
                    bumped = true;
                }
            }
            if !bumped && !no_changes {
                output::eprint_line("No packages to bump.")?;
            }
            Ok(())
        }
    }
}
