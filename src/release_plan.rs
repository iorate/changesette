use std::{fs, io, path::PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    bump::{self, Bump},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    package_json::PackageJson,
    workspace::Workspace,
};

/// The JSON document that `version` prints to stdout, mirroring the upstream
/// `ReleasePlan` type (without `preState`, plus `changelogEntry` on each
/// release). Serialized as a single line.
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

/// A file update staged by [`compute`]: applying the plan writes the bumped
/// package.json back and replaces the member's CHANGELOG.md content.
pub(crate) struct PlannedWrite {
    /// The member's package.json with the new version already set.
    pub(crate) package_json: PackageJson,
    /// The member's CHANGELOG.md path.
    pub(crate) changelog_path: PathBuf,
    /// The full new CHANGELOG.md content.
    pub(crate) changelog_text: String,
}

/// Builds the release plan for the given changesets, validating that every
/// package they name is a workspace member, and stages the file writes that
/// would apply it. Reads each named member's package.json and CHANGELOG.md
/// but modifies nothing on disk.
pub(crate) fn compute(
    workspace: &Workspace,
    changes: &[LoadedChange],
) -> Result<(ReleasePlan, Vec<PlannedWrite>)> {
    let changeset_dir = workspace.root().join(".changeset");
    for change in changes {
        for (name, _) in &change.releases {
            workspace
                .member(name)
                .with_context(|| changeset_dir.join(&change.file_name).display().to_string())?;
        }
    }

    let mut releases = Vec::new();
    let mut writes = Vec::new();
    for (name, max_bump) in changeset::max_bumps(changes) {
        let member = workspace.member(name)?;
        let mut package_json = PackageJson::load(member.dir())?;
        let old_version = package_json.version().clone();
        let ids = changes
            .iter()
            .filter(|change| change.releases.iter().any(|(n, _)| n == name))
            .map(id)
            .collect();

        let (new_version, changelog_entry) = match max_bump {
            Some(max_bump) => {
                let next = bump::next_version(&old_version, max_bump);
                let entries: Vec<(Bump, &str)> = changes
                    .iter()
                    .filter_map(|change| {
                        change
                            .releases
                            .iter()
                            .find(|(n, _)| n == name)
                            .and_then(|(_, bump)| *bump)
                            .map(|bump| (bump, change.summary.as_str()))
                    })
                    .collect();
                let entry = render_entry(&entries);
                let section = render_section(&next, &entry);

                package_json.set_version(&next)?;
                let changelog_path = member.dir().join("CHANGELOG.md");
                let changelog_text = match fs::read_to_string(&changelog_path) {
                    Ok(text) => text,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
                    Err(err) => return Err(err).context(changelog_path.display().to_string()),
                };
                let new_changelog_text =
                    changelog::upsert_section(&changelog_text, name, &next.to_string(), &section);
                writes.push(PlannedWrite {
                    package_json,
                    changelog_path,
                    changelog_text: new_changelog_text,
                });
                (next, Some(entry))
            }
            None => (old_version.clone(), None),
        };
        releases.push(Release {
            name: name.to_owned(),
            bump: max_bump.map_or("none", Bump::as_str),
            old_version: old_version.to_string(),
            new_version: new_version.to_string(),
            changesets: ids,
            changelog_entry,
        });
    }

    let plan = ReleasePlan {
        changesets: changes
            .iter()
            .map(|change| ChangesetEntry {
                id: id(change),
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
    };
    Ok((plan, writes))
}

fn id(change: &LoadedChange) -> String {
    change
        .file_name
        .strip_suffix(".md")
        .unwrap_or(&change.file_name)
        .to_owned()
}
