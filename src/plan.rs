use std::{collections::BTreeMap, env, fs, io, path::PathBuf};

use anyhow::{Context, Result, bail};
use semver::Version;

use crate::{
    bump::{self, Bump},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    config,
    package_json::PackageJson,
    pre::{self, PreJson, PreMode},
    skip::SkipSet,
    workspace::{Member, Workspace},
};

/// A planned `version` run: the changesets to consume and the releases to
/// apply, produced by `plan_version` without modifying anything on disk.
pub(crate) struct PlannedVersion {
    pub(crate) workspace: Workspace,
    pub(crate) changeset_dir: PathBuf,
    pub(crate) pre: Option<PreJson>,
    /// Every unreleased changeset, the ones naming only skipped packages
    /// included; the release plan reports all of them.
    pub(crate) changes: Vec<LoadedChange>,
    /// The changesets `version` consumes, skip-filtered.
    pub(crate) consumed_changes: Vec<LoadedChange>,
    pub(crate) releases: Vec<PlannedRelease>,
}

impl PlannedVersion {
    /// The pre state when in pre mode.
    pub(crate) fn in_pre(&self) -> Option<&PreJson> {
        self.pre.as_ref().filter(|pre| pre.mode() == PreMode::Pre)
    }

    pub(crate) fn exiting_pre(&self) -> bool {
        matches!(&self.pre, Some(pre) if pre.mode() == PreMode::Exit)
    }
}

/// Discovers the workspace containing the current directory and plans the
/// pending `version` run, resolving the ignore set from the config or
/// `cli_ignore` (using both is an error), modifying nothing on disk.
pub(crate) fn plan_version(cli_ignore: &[String]) -> Result<PlannedVersion> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let config = config::load(&changeset_dir)?;
    let config_ignore = config.resolve_ignore(workspace.members().iter().map(Member::name))?;
    let ignore = if config_ignore.is_empty() {
        for name in cli_ignore {
            workspace.member(name).context("invalid `--ignore` value")?;
        }
        cli_ignore.to_vec()
    } else if cli_ignore.is_empty() {
        config_ignore
    } else {
        bail!(
            "the --ignore option cannot be used while ignore is defined in .changeset/config.json; use only one of them"
        );
    };
    let skip = SkipSet::build(&workspace, &config, &ignore);

    let pre = PreJson::load(&changeset_dir)?;
    if let Some(pre) = pre.as_ref().filter(|pre| pre.mode() == PreMode::Pre) {
        pre::validate_tag(pre.tag())?;
    }

    let mut changes = changeset::load(&changeset_dir)?;
    if matches!(&pre, Some(pre) if pre.mode() == PreMode::Pre) {
        // The `pre/` changesets were already consumed in this pre-release
        // cycle.
        changes.retain(|change| !change.in_pre);
    }
    let consumed_changes = skip.filter_changes(&workspace, &changeset_dir, &changes)?;
    let releases = plan_releases(&workspace, &consumed_changes, pre.as_ref(), &skip)?;

    Ok(PlannedVersion {
        workspace,
        changeset_dir,
        pre,
        changes,
        consumed_changes,
        releases,
    })
}

pub(crate) struct PlannedRelease {
    pub(crate) name: String,
    /// The widest bump requested for the package; `None` when it is only
    /// ever named with the `none` type.
    pub(crate) bump: Option<Bump>,
    pub(crate) old_version: Version,
    pub(crate) new_version: Version,
    /// The ids of the changesets naming this package (`none` entries
    /// included), in load order (`pre/` changesets first).
    pub(crate) changeset_ids: Vec<String>,
    /// The body of the new `## <new_version>` section, without the heading;
    /// `None` for a `None` bump.
    pub(crate) changelog_entry: Option<String>,
}

/// Plans the releases requested by `changes` (plus, when exiting pre mode,
/// the members still on a pre-release version), modifying nothing on disk.
fn plan_releases(
    workspace: &Workspace,
    changes: &[LoadedChange],
    pre: Option<&PreJson>,
    skip: &SkipSet,
) -> Result<Vec<PlannedRelease>> {
    let mut max_bumps = changeset::max_bumps(changes);
    if matches!(pre, Some(pre) if pre.mode() == PreMode::Exit) {
        rescue_prereleases(workspace, skip, &mut max_bumps)?;
    }

    let mut releases = Vec::new();
    for (name, max_bump) in max_bumps {
        let member = workspace.member(name)?;
        let package_json = PackageJson::load(member.dir())?;
        let old_version = package_json
            .version()
            .with_context(|| {
                format!(
                    "{}: missing top-level \"version\"",
                    package_json.path().display()
                )
            })?
            .clone();
        let changeset_ids = changes
            .iter()
            .filter(|change| change.releases.iter().any(|(n, _)| n == name))
            .map(LoadedChange::id)
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
                let new_version = match pre {
                    Some(pre) if pre.mode() == PreMode::Pre => {
                        bump::next_pre_version(&old_version, max_bump, pre.tag())
                    }
                    _ => bump::next_version(&old_version, max_bump),
                };
                (new_version, Some(render_entry(&summaries)))
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

// Forces a patch bump on every member left on a pre-release version that no
// changeset releases. `next_version` then merely drops the pre-release, and
// the empty summary list renders a heading-only changelog section.
fn rescue_prereleases<'a>(
    workspace: &'a Workspace,
    skip: &SkipSet,
    max_bumps: &mut BTreeMap<&'a str, Option<Bump>>,
) -> Result<()> {
    for member in workspace.members() {
        if skip.contains(member.name()) || max_bumps.get(member.name()).is_some_and(Option::is_some)
        {
            continue;
        }
        let package_json = PackageJson::load(member.dir())?;
        if package_json
            .version()
            .is_some_and(|version| !version.pre.is_empty())
        {
            max_bumps.insert(member.name(), Some(Bump::Patch));
        }
    }
    Ok(())
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

/// Stages the package.json and CHANGELOG.md writes that apply `releases`,
/// modifying nothing on disk.
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
