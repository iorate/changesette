use std::{fmt::Write as _, path::Path};

use anyhow::Result;

use crate::{bump::Bump, output, plan, release_plan};

/// Prints the packages to be bumped by `version` — or, with `output_path`,
/// the release plan as JSON — following the same plan as `version` without
/// applying it.
pub(crate) fn run(verbose: bool, output_path: Option<&Path>) -> Result<()> {
    let planned = plan::plan_version(&[], None)?;

    if let Some(path) = output_path {
        return release_plan::write_file(
            path,
            &release_plan::build(&planned.changes, &planned.releases, planned.pre.as_ref()),
        );
    }

    let mut text = String::from("Packages to be bumped:");
    for group in [Bump::Major, Bump::Minor, Bump::Patch] {
        let group_releases: Vec<_> = planned
            .releases
            .iter()
            .filter(|release| release.bump == Some(group))
            .collect();
        if group_releases.is_empty() {
            continue;
        }
        let _ = write!(text, "\n- {}", group.as_str());
        for release in group_releases {
            let _ = write!(text, "\n  - {}", release.name);
            if verbose {
                let _ = write!(text, " -> {}", release.new_version);
                for id in &release.changeset_ids {
                    let _ = write!(text, "\n    - .changeset/{id}.md");
                }
            }
        }
    }
    output::print_line(&text)
}
