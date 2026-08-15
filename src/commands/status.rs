use std::{env, path::Path};

use anyhow::Result;

use crate::{changeset, output, release_plan, workspace::Workspace};

/// Prints the packages to be bumped by `version` to stdout, grouped by bump
/// type in major, minor, patch order; packages named only with `none` are
/// omitted. With `verbose`, each package carries its new version and the
/// changeset files naming it. With `output_path`, writes the release plan to
/// that file as pretty-printed JSON instead and prints nothing. Modifies no
/// other file.
pub(crate) fn run(verbose: bool, output_path: Option<&Path>) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let changes = changeset::load(&changeset_dir)?;
    let (plan, _) = release_plan::compute(&workspace, &changes)?;

    if let Some(path) = output_path {
        return release_plan::write_file(path, &plan);
    }

    let mut text = String::from("Packages to be bumped:");
    for group in ["major", "minor", "patch"] {
        let releases: Vec<_> = plan
            .releases
            .iter()
            .filter(|release| release.bump == group)
            .collect();
        if releases.is_empty() {
            continue;
        }
        text.push_str(&format!("\n- {group}"));
        for release in releases {
            text.push_str(&format!("\n  - {}", release.name));
            if verbose {
                text.push_str(&format!(" -> {}", release.new_version));
                for id in &release.changesets {
                    text.push_str(&format!("\n    - .changeset/{id}.md"));
                }
            }
        }
    }
    output::print_line(&text)
}
