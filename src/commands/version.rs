use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{changeset, output, release_plan, workspace::Workspace};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, and deletes
/// the consumed files (`none`-only and empty changesets included). Packages
/// named only with `none` keep their version and changelog; with zero
/// changesets, nothing changes. Each name in `ignore` must be a workspace
/// member; a changeset naming an ignored package is skipped — excluded from
/// the release plan and left on disk — and it is an error for such a
/// changeset to also name a package that is not ignored. Reports the applied
/// bumps to stdout; with `output_path`, prints nothing and instead writes the
/// release plan, each bumped release carrying its new changelog entry, to
/// that file as pretty-printed JSON.
pub(crate) fn run(ignore: &[String], output_path: Option<&Path>) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    for name in ignore {
        workspace.member(name).context("invalid `--ignore` value")?;
    }
    let changeset_dir = workspace.root().join(".changeset");
    let changes = changeset::load(&changeset_dir)?;
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
                changeset_dir.join(&change.file_name).display(),
                quote_list(&ignored),
                quote_list(&not_ignored)
            );
        }
    }

    let (plan, writes) = release_plan::compute(&workspace, &consumed)?;

    for write in writes {
        write.package_json.save()?;
        fs::write(&write.changelog_path, write.changelog_text)
            .with_context(|| write.changelog_path.display().to_string())?;
    }
    for change in &consumed {
        let path = changeset_dir.join(&change.file_name);
        fs::remove_file(&path).with_context(|| path.display().to_string())?;
    }

    match output_path {
        Some(path) => release_plan::write_file(path, &plan),
        None => {
            if no_changes {
                return output::print_line("No unreleased changesets found.");
            }
            for release in &plan.releases {
                if release.bump != "none" {
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
