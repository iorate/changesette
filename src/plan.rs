use std::{fs, io, path::PathBuf};

use anyhow::{Context, Result};
use semver::Version;

use crate::{
    bump::{self, Bump},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    package_json::PackageJson,
    workspace::Workspace,
};

pub(crate) struct PlannedRelease {
    pub(crate) name: String,
    /// The widest bump requested for the package; `None` when it is only
    /// ever named with the `none` type.
    pub(crate) bump: Option<Bump>,
    pub(crate) old_version: Version,
    pub(crate) new_version: Version,
    /// The ids of the changesets naming this package (`none` entries
    /// included), in file-name order.
    pub(crate) changeset_ids: Vec<String>,
    /// The body of the new `## <new_version>` section, without the heading.
    /// `None` for a `None` bump.
    pub(crate) changelog_entry: Option<String>,
}

/// Plans the release of each package named by `changes`, validating that
/// every named package is a workspace member. Modifies nothing on disk.
pub(crate) fn plan_releases(
    workspace: &Workspace,
    changes: &[LoadedChange],
) -> Result<Vec<PlannedRelease>> {
    let changeset_dir = workspace.root().join(".changeset");
    for change in changes {
        for (name, _) in &change.releases {
            workspace
                .member(name)
                .with_context(|| changeset_dir.join(&change.file_name).display().to_string())?;
        }
    }

    let mut releases = Vec::new();
    for (name, max_bump) in changeset::max_bumps(changes) {
        let member = workspace.member(name)?;
        let package_json = PackageJson::load(member.dir())?;
        let old_version = package_json.version().clone();
        let changeset_ids = changes
            .iter()
            .filter(|change| change.releases.iter().any(|(n, _)| n == name))
            .map(|change| change.id().to_owned())
            .collect();

        let (new_version, changelog_entry) = match max_bump {
            Some(max_bump) => {
                let summaries: Vec<(Bump, &str)> = changes
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
                (
                    bump::next_version(&old_version, max_bump),
                    Some(render_entry(&summaries)),
                )
            }
            None => (old_version.clone(), None),
        };
        releases.push(PlannedRelease {
            name: name.to_owned(),
            bump: max_bump,
            old_version,
            new_version,
            changeset_ids,
            changelog_entry,
        });
    }
    Ok(releases)
}

pub(crate) struct StagedWrite {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
}

impl StagedWrite {
    pub(crate) fn apply(&self) -> Result<()> {
        fs::write(&self.path, &self.content).with_context(|| self.path.display().to_string())
    }
}

/// Stages the writes that apply `releases`: each bumped package's
/// package.json with the new version set and its CHANGELOG.md with the new
/// section upserted. Modifies nothing on disk.
pub(crate) fn stage_writes(
    workspace: &Workspace,
    releases: &[PlannedRelease],
) -> Result<Vec<StagedWrite>> {
    let mut writes = Vec::new();
    for release in releases {
        let Some(entry) = &release.changelog_entry else {
            continue;
        };
        let member = workspace.member(&release.name)?;
        let mut package_json = PackageJson::load(member.dir())?;
        package_json.set_version(&release.new_version)?;
        writes.push(StagedWrite {
            path: package_json.path().to_owned(),
            content: package_json.text(),
        });

        let changelog_path = member.dir().join("CHANGELOG.md");
        let changelog_text = match fs::read_to_string(&changelog_path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err).context(changelog_path.display().to_string()),
        };
        let section = render_section(&release.new_version, entry);
        writes.push(StagedWrite {
            path: changelog_path,
            content: changelog::upsert_section(
                &changelog_text,
                &release.name,
                &release.new_version.to_string(),
                &section,
            ),
        });
    }
    Ok(writes)
}
