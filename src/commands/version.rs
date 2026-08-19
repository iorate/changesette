use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    changeset, output, plan,
    pre::{self, PreJson, PreMode},
    release_plan,
    workspace::Workspace,
};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, and deletes
/// the consumed files (`none`-only and empty changesets included). Packages
/// named only with `none` keep their version and changelog; with zero
/// changesets, nothing changes. Each name in `ignore` must be a workspace
/// member; a changeset naming an ignored package is skipped — excluded from
/// the release plan and left on disk — and it is an error for such a
/// changeset to also name a package that is not ignored.
///
/// In pre mode, only the changesets not yet consumed in this pre-release
/// cycle are planned, the new versions are prereleases, and the consumed
/// files are moved to `.changeset/pre/` instead of deleted. Exiting pre mode,
/// the parked files are replanned along with the new ones into final versions
/// and pre.json is deleted, even when there is nothing to release.
///
/// Reports the applied bumps to stdout; with `output_path`, instead writes
/// the release plan, each bumped release carrying its new changelog entry,
/// as pretty-printed JSON to that file (or to stdout when the path is `-`).
pub(crate) fn run(ignore: &[String], output_path: Option<&Path>) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    for name in ignore {
        workspace.member(name).context("invalid `--ignore` value")?;
    }
    let changeset_dir = workspace.root().join(".changeset");

    let pre = PreJson::load(&changeset_dir)?;
    let in_pre = match &pre {
        Some(pre) if pre.mode() == PreMode::Pre => {
            pre::validate_tag(pre.tag())?;
            eprintln!(
                "warning: in pre mode with tag `{}`; versions will be prereleases. Run `changesette pre exit` first for a normal release.",
                pre.tag()
            );
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

    let mut consumed = Vec::new();
    for change in changes {
        let (ignored, not_ignored): (Vec<&str>, Vec<&str>) = change
            .releases
            .iter()
            .map(|(name, _)| name.as_str())
            .partition(|name| ignore.iter().any(|ignored| ignored == name));
        if ignored.is_empty() {
            consumed.push(change);
        } else if !not_ignored.is_empty() {
            bail!(
                "{}: cannot mix ignored packages ({}) and not ignored packages ({})",
                changeset_dir.join(change.rel_path()).display(),
                quote_list(&ignored),
                quote_list(&not_ignored)
            );
        }
    }

    let releases = plan::plan_releases(&workspace, &consumed, pre.as_ref(), ignore)?;

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
                return output::print_line("No unreleased changesets found.");
            }
            for release in &releases {
                if release.bump.is_some() {
                    output::print_line(&format!(
                        "Bumped {} {} -> {}",
                        release.name, release.old_version, release.new_version
                    ))?;
                }
            }
            Ok(())
        }
    }
}

fn quote_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
