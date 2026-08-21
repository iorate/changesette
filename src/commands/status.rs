use std::{env, path::Path};

use anyhow::Result;

use crate::{
    bump::Bump,
    changeset, config, output, plan,
    pre::{self, PreJson, PreMode},
    release_plan,
    skip::SkipSet,
    workspace::Workspace,
};

/// Prints the packages to be bumped by `version` to stdout; packages named
/// only with `none` are omitted. With `verbose`, also shows each package's
/// new version and the changeset files naming it. With `output_path`, writes
/// the release plan as pretty-printed JSON to that file (or to stdout when
/// the path is `-`) instead. Follows the same plan as `version`, pre mode
/// included. Modifies no file other than `output_path`.
pub(crate) fn run(verbose: bool, output_path: Option<&Path>) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let config = config::load(&changeset_dir)?;
    let skip = SkipSet::build(&workspace, &config, &[])?;

    let pre = PreJson::load(&changeset_dir)?;
    let in_pre = match &pre {
        Some(pre) if pre.mode() == PreMode::Pre => {
            pre::validate_tag(pre.tag())?;
            true
        }
        _ => false,
    };

    let mut changes = changeset::load(&changeset_dir)?;
    if in_pre {
        changes.retain(|change| !change.in_pre);
    }
    let changes = skip.filter_changes(&workspace, &changeset_dir, changes)?;
    let releases = plan::plan_releases(&workspace, &changes, pre.as_ref(), &skip)?;

    if let Some(path) = output_path {
        return release_plan::write_file(
            path,
            &release_plan::build(&changes, &releases, pre.as_ref()),
        );
    }

    let mut text = String::from("Packages to be bumped:");
    for group in [Bump::Major, Bump::Minor, Bump::Patch] {
        let group_releases: Vec<_> = releases
            .iter()
            .filter(|release| release.bump == Some(group))
            .collect();
        if group_releases.is_empty() {
            continue;
        }
        text.push_str(&format!("\n- {}", group.as_str()));
        for release in group_releases {
            text.push_str(&format!("\n  - {}", release.name));
            if verbose {
                text.push_str(&format!(" -> {}", release.new_version));
                for id in &release.changeset_ids {
                    text.push_str(&format!("\n    - .changeset/{id}.md"));
                }
            }
        }
    }
    output::print_line(&text)
}
