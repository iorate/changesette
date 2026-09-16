use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::info;

use crate::{bump::Bump, output, plan::PlannedVersion};

#[derive(Serialize)]
pub struct ReleasePlan {
    pub changesets: Vec<ChangesetEntry>,
    pub releases: Vec<Release>,
    #[serde(rename = "preState", skip_serializing_if = "Option::is_none")]
    pub pre_state: Option<PreState>,
}

#[derive(Serialize)]
pub struct PreState {
    pub mode: &'static str,
    pub tag: String,
}

#[derive(Serialize)]
pub struct ChangesetEntry {
    pub id: String,
    pub summary: String,
    pub releases: Vec<ReleaseRef>,
}

#[derive(Serialize)]
pub struct ReleaseRef {
    pub name: String,
    #[serde(rename = "type")]
    pub bump: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub bump: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_version: Option<String>,
    pub changesets: Vec<String>,
    pub dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog_entry: Option<String>,
}

#[must_use]
pub fn build(planned: &PlannedVersion) -> ReleasePlan {
    let mut releases: Vec<Release> = planned
        .releases
        .iter()
        .map(|release| Release {
            name: Some(release.name.clone()),
            bump: release.bump.map_or("none", Bump::as_str),
            old_version: Some(release.old_version.to_string()),
            new_version: Some(release.new_version.to_string()),
            changesets: release.changeset_ids.clone(),
            dir: release.dir.as_str().to_owned(),
            changelog_entry: release.changelog_entry.clone(),
        })
        .collect();
    let released: BTreeSet<_> = planned
        .releases
        .iter()
        .map(|release| &release.dir)
        .collect();
    let rewritten: BTreeSet<_> = planned
        .dependency_updates
        .iter()
        .filter(|update| update.new.is_some() && !released.contains(&update.dependent))
        .map(|update| &update.dependent)
        .collect();
    releases.extend(rewritten.into_iter().map(|dir| {
        let package = &planned.workspace[dir];
        let version = package.version().map(ToString::to_string);
        Release {
            name: package.name().map(str::to_owned),
            bump: "none",
            old_version: version.clone(),
            new_version: version,
            changesets: Vec::new(),
            dir: dir.as_str().to_owned(),
            changelog_entry: None,
        }
    }));
    releases.sort_by(|a, b| a.dir.cmp(&b.dir));
    ReleasePlan {
        changesets: planned
            .changes
            .iter()
            .map(|change| ChangesetEntry {
                id: change.id(),
                summary: change.summary.clone(),
                releases: change
                    .releases
                    .iter()
                    .map(|(name, bump)| ReleaseRef {
                        name: name.clone(),
                        bump: bump.map_or("none", Bump::as_str),
                    })
                    .collect(),
            })
            .collect(),
        releases,
        pre_state: planned.pre.as_ref().map(|pre| PreState {
            mode: pre.mode().as_str(),
            tag: pre.tag().to_owned(),
        }),
    }
}

pub fn write_file(path: &Path, plan: &ReleasePlan) -> Result<()> {
    if path == Path::new("-") {
        return output::print_json(plan);
    }
    let json = serde_json::to_string_pretty(plan)? + "\n";
    fs::write(path, json).with_context(|| path.display().to_string())?;
    info!("Wrote the release plan to {}", path.display());
    Ok(())
}
