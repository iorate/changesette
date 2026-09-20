use std::{fmt::Write as _, path::Path};

use anyhow::Result;

use crate::{bump::Bump, config::Config, output, plan, release_plan, workspace::Workspace};

pub fn run(
    workspace: Workspace,
    config: &Config,
    verbose: bool,
    allow_unreleased_dependencies: bool,
    output_path: Option<&Path>,
) -> Result<()> {
    let planned = plan::plan_version(workspace, config, None, allow_unreleased_dependencies)?;

    if let Some(path) = output_path {
        return release_plan::write_file(path, &release_plan::build(&planned));
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
                for (name, version) in &release.updated_dependencies {
                    let _ = write!(text, "\n    - updated dependency {name}@{version}");
                }
            }
        }
    }
    output::print_line(&text)
}
