use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{bump::Bump, changeset::LoadedChange, output, plan::PlannedRelease, pre::PreJson};

/// The JSON document that `version` and `status` write with `--output`,
/// mirroring the upstream `ReleasePlan` type (plus `changelogEntry` on each
/// release).
#[derive(Serialize)]
pub(crate) struct ReleasePlan {
    pub(crate) changesets: Vec<ChangesetEntry>,
    pub(crate) releases: Vec<Release>,
    /// The state of `.changeset/pre.json`, absent when there is no such file.
    #[serde(rename = "preState", skip_serializing_if = "Option::is_none")]
    pub(crate) pre_state: Option<PreState>,
}

#[derive(Serialize)]
pub(crate) struct PreState {
    pub(crate) mode: &'static str,
    pub(crate) tag: String,
}

#[derive(Serialize)]
pub(crate) struct ChangesetEntry {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) releases: Vec<ReleaseRef>,
}

#[derive(Serialize)]
pub(crate) struct ReleaseRef {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Release {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
    pub(crate) old_version: String,
    pub(crate) new_version: String,
    /// The ids of the changesets naming this package (`none` entries
    /// included), in load order (root changesets first, then `pre/`).
    pub(crate) changesets: Vec<String>,
    /// The body of the new `## <new_version>` section, without the heading;
    /// absent for `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) changelog_entry: Option<String>,
}

pub(crate) fn build(
    changes: &[LoadedChange],
    releases: &[PlannedRelease],
    pre: Option<&PreJson>,
) -> ReleasePlan {
    ReleasePlan {
        changesets: changes
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
        releases: releases
            .iter()
            .map(|release| Release {
                name: release.name.clone(),
                bump: release.bump.map_or("none", Bump::as_str),
                old_version: release.old_version.to_string(),
                new_version: release.new_version.to_string(),
                changesets: release.changeset_ids.clone(),
                changelog_entry: release.changelog_entry.clone(),
            })
            .collect(),
        pre_state: pre.map(|pre| PreState {
            mode: pre.mode().as_str(),
            tag: pre.tag().to_owned(),
        }),
    }
}

pub(crate) fn write_file(path: &Path, plan: &ReleasePlan) -> Result<()> {
    if path == Path::new("-") {
        return output::print_json(plan);
    }
    fs::write(path, serde_json::to_string_pretty(plan)?).with_context(|| path.display().to_string())
}
