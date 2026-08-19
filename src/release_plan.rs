use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{bump::Bump, changeset::LoadedChange, plan::PlannedRelease};

/// The JSON document that `version` and `status` write with `--output`,
/// mirroring the upstream `ReleasePlan` type (without `preState`, plus
/// `changelogEntry` on each release).
#[derive(Serialize)]
pub(crate) struct ReleasePlan {
    /// The consumed changesets, in file-name order.
    pub(crate) changesets: Vec<ChangesetEntry>,
    /// One entry per package named by any changeset, in package-name order.
    pub(crate) releases: Vec<Release>,
}

/// A consumed changeset.
#[derive(Serialize)]
pub(crate) struct ChangesetEntry {
    /// The changeset file name without the `.md` extension.
    pub(crate) id: String,
    /// The summary text of the changeset.
    pub(crate) summary: String,
    /// The packages the changeset names, in frontmatter order.
    pub(crate) releases: Vec<ReleaseRef>,
}

/// One package-to-bump entry in a changeset's frontmatter.
#[derive(Serialize)]
pub(crate) struct ReleaseRef {
    /// The package name.
    pub(crate) name: String,
    /// The requested bump type: `major`, `minor`, `patch`, or `none`.
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
}

/// The version change planned for one package.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Release {
    /// The package name.
    pub(crate) name: String,
    /// The widest bump requested for the package: `major`, `minor`, `patch`,
    /// or `none`.
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
    /// The package version before this run.
    pub(crate) old_version: String,
    /// The package version after this run; equals `old_version` for `none`.
    pub(crate) new_version: String,
    /// The ids of the changesets naming this package (`none` entries
    /// included), in file-name order.
    pub(crate) changesets: Vec<String>,
    /// The changelog entry written for this release: the body of the new
    /// `## <new_version>` section, without the heading. Absent for `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) changelog_entry: Option<String>,
}

/// Builds the JSON document from the consumed changesets and their planned
/// releases.
pub(crate) fn build(changes: &[LoadedChange], releases: &[PlannedRelease]) -> ReleasePlan {
    ReleasePlan {
        changesets: changes
            .iter()
            .map(|change| ChangesetEntry {
                id: change.id().to_owned(),
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
    }
}

/// Writes `plan` to `path` as pretty-printed JSON without a trailing newline.
pub(crate) fn write_file(path: &Path, plan: &ReleasePlan) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(plan)?).with_context(|| path.display().to_string())
}
