use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use semver::Version;
use tracing::debug;

use crate::{
    bump::{self, Bump},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    config::{self, ResolvedGroups},
    output::display_path,
    package_json::PackageJson,
    pre::{self, PreJson, PreMode},
    skip::SkipSet,
    snapshot::{Snapshot, SnapshotVersions},
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

fn pre_state(pre: Option<&PreJson>) -> Option<&PreJson> {
    pre.filter(|pre| pre.mode() == PreMode::Pre)
}

impl PlannedVersion {
    /// The pre state when in pre mode.
    pub(crate) fn in_pre(&self) -> Option<&PreJson> {
        pre_state(self.pre.as_ref())
    }

    pub(crate) fn exiting_pre(&self) -> bool {
        matches!(&self.pre, Some(pre) if pre.mode() == PreMode::Exit)
    }
}

/// Discovers the workspace containing the current directory and plans the
/// pending `version` run, resolving the ignore set from the config or
/// `cli_ignore` (using both is an error), modifying nothing on disk;
/// `snapshot` is an error in pre mode.
pub(crate) fn plan_version(
    cli_ignore: &[String],
    snapshot: Option<&Snapshot>,
) -> Result<PlannedVersion> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let config = config::load(&changeset_dir)?;
    let skip = SkipSet::load(&workspace, &config, cli_ignore)?;
    let names: Vec<&str> = workspace.members().iter().map(Member::name).collect();
    let groups = config
        .resolve_groups(&names)
        .with_context(|| display_path(&changeset_dir.join("config.json")))?;

    let pre = PreJson::load(&changeset_dir)?;
    let in_pre = pre_state(pre.as_ref());
    if let Some(pre) = in_pre {
        if snapshot.is_some() {
            bail!(
                "snapshot releases are not allowed in pre mode; run `changesette pre exit` first"
            );
        }
        pre::validate_tag(pre.tag())?;
    }
    let snapshot_versions = snapshot
        .map(|snapshot| SnapshotVersions::resolve(snapshot, &config))
        .transpose()?;

    let mut changes = changeset::load(&changeset_dir)?;
    if in_pre.is_some() {
        // The `pre/` changesets were already consumed in this pre-release
        // cycle.
        changes.retain(|change| !change.in_pre);
    }
    let consumed_changes = skip.filter_changes(&workspace, &changeset_dir, &changes)?;
    let releases = plan_releases(
        &workspace,
        &consumed_changes,
        pre.as_ref(),
        &skip,
        snapshot_versions.as_ref(),
        &groups,
    )?;

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
    /// included), in load order (root changesets first, then `pre/`).
    pub(crate) changeset_ids: Vec<String>,
    /// The body of the new `## <new_version>` section, without the heading;
    /// `None` for a `None` bump.
    pub(crate) changelog_entry: Option<String>,
}

// Plans the releases requested by `changes` (plus, when exiting pre mode,
// the members still on a pre-release version), modifying nothing on disk.
fn plan_releases(
    workspace: &Workspace,
    changes: &[LoadedChange],
    pre: Option<&PreJson>,
    skip: &SkipSet,
    snapshot: Option<&SnapshotVersions>,
    groups: &ResolvedGroups,
) -> Result<Vec<PlannedRelease>> {
    let mut max_bumps = changeset::max_bumps(changes);
    // The upstream applies the group passes before the pre exit rescue, so a
    // rescued member does not pull its group along.
    let overrides = apply_groups(
        workspace,
        groups,
        skip,
        pre_state(pre).map(PreJson::tag),
        &mut max_bumps,
    )?;
    if matches!(pre, Some(pre) if pre.mode() == PreMode::Exit) {
        rescue_prereleases(workspace, skip, groups, &mut max_bumps)?;
    }

    let mut releases = Vec::new();
    for (name, max_bump) in max_bumps {
        let member = workspace.member(name)?;
        let old_version = match overrides.old_versions.get(name) {
            Some(version) => version.clone(),
            None => member.version().clone(),
        };
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
                let new_version = match snapshot {
                    Some(snapshot) => snapshot.apply(&old_version, max_bump),
                    None => match pre_state(pre) {
                        Some(pre) => match overrides.pre_counters.get(name) {
                            Some(&counter) => bump::next_pre_version_with(
                                &old_version,
                                max_bump,
                                pre.tag(),
                                counter,
                            ),
                            None => bump::next_pre_version(&old_version, max_bump, pre.tag()),
                        },
                        None => bump::next_version(&old_version, max_bump),
                    },
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

// The per-member adjustments the group passes make beyond `max_bumps`.
struct GroupOverrides {
    // The `old_version` a group member plans against instead of its own:
    // the highest current version in its group.
    old_versions: BTreeMap<String, Version>,
    // The pre-release counter a group member uses in pre mode: the highest
    // `pre_counter` in its group.
    pre_counters: BTreeMap<String, u64>,
}

// Applies the `fixed` and `linked` group semantics to `max_bumps`, following
// the upstream matchFixedConstraint and applyLinks: when a group has a
// releasing member, `fixed` releases every non-skipped member at the group's
// widest bump, while `linked` only aligns the members already releasing;
// both plan against the group's highest current version. One pass per kind
// reaches the fixed point because config validation keeps the groups
// disjoint and changesette adds no dependents.
fn apply_groups<'a>(
    workspace: &'a Workspace,
    groups: &ResolvedGroups,
    skip: &SkipSet,
    pre_tag: Option<&str>,
    max_bumps: &mut BTreeMap<&'a str, Option<Bump>>,
) -> Result<GroupOverrides> {
    let mut old_versions = BTreeMap::new();
    for group in &groups.fixed {
        let Some(max_bump) = group_max_bump(group, max_bumps) else {
            continue;
        };
        let highest = group_highest_version(workspace, group)?;
        for name in group {
            if skip.contains(name) {
                continue;
            }
            let previous = max_bumps.insert(workspace.member(name)?.name(), Some(max_bump));
            if previous.flatten() != Some(max_bump) {
                debug!(
                    "`{name}`: the \"fixed\" group raises the bump to {} (planning against {highest})",
                    max_bump.as_str()
                );
            }
            old_versions.insert(name.clone(), highest.clone());
        }
    }
    for group in &groups.linked {
        let Some(max_bump) = group_max_bump(group, max_bumps) else {
            continue;
        };
        let highest = group_highest_version(workspace, group)?;
        for name in group {
            if let Some(entry) = max_bumps.get_mut(name.as_str())
                && entry.is_some()
            {
                if *entry != Some(max_bump) {
                    debug!(
                        "`{name}`: the \"linked\" group raises the bump to {} (planning against {highest})",
                        max_bump.as_str()
                    );
                }
                *entry = Some(max_bump);
                old_versions.insert(name.clone(), highest.clone());
            }
        }
    }

    let mut pre_counters = BTreeMap::new();
    if let Some(tag) = pre_tag {
        // The old_version override alone would miss a member whose version
        // is low but whose counter is high, so the counter is aligned
        // separately, as the upstream getPreInfo does with preVersions.
        for group in groups.fixed.iter().chain(&groups.linked) {
            let mut counter = 0;
            for name in group {
                counter = counter.max(bump::pre_counter(workspace.member(name)?.version(), tag));
            }
            for name in group {
                pre_counters.insert(name.clone(), counter);
            }
        }
    }

    Ok(GroupOverrides {
        old_versions,
        pre_counters,
    })
}

// The widest bump among the group's releasing members, or `None` when no
// member releases (a `none`-only entry does not count as releasing).
fn group_max_bump(group: &[String], max_bumps: &BTreeMap<&str, Option<Bump>>) -> Option<Bump> {
    group
        .iter()
        .filter_map(|name| max_bumps.get(name.as_str()).copied().flatten())
        .max()
}

// The highest current version among all group members, the skipped ones
// included as in the upstream getCurrentHighestVersion.
fn group_highest_version(workspace: &Workspace, group: &[String]) -> Result<Version> {
    let mut highest: Option<&Version> = None;
    for name in group {
        let version = workspace.member(name)?.version();
        if highest.is_none_or(|h| version > h) {
            highest = Some(version);
        }
    }
    Ok(highest
        .expect("a group with a releasing member is nonempty")
        .clone())
}

// Forces a patch bump on every member left on a pre-release version that no
// changeset releases, and on every non-skipped member of a `fixed` /
// `linked` group containing such a version — each at its own version, as the
// upstream group override of preVersions makes the exit rescue do.
// `next_version` then merely drops the pre-release, and the empty summary
// list renders a heading-only changelog section.
fn rescue_prereleases<'a>(
    workspace: &'a Workspace,
    skip: &SkipSet,
    groups: &ResolvedGroups,
    max_bumps: &mut BTreeMap<&'a str, Option<Bump>>,
) -> Result<()> {
    let mut group_rescued = BTreeSet::new();
    for group in groups.fixed.iter().chain(&groups.linked) {
        // The skipped members count here too, like in the upstream
        // getHighestPreVersion; only the rescue itself excludes them.
        let mut on_prerelease = false;
        for name in group {
            if !workspace.member(name)?.version().pre.is_empty() {
                on_prerelease = true;
                break;
            }
        }
        if on_prerelease {
            group_rescued.extend(group.iter().map(String::as_str));
        }
    }
    for member in workspace.members() {
        if skip.contains(member.name()) || max_bumps.get(member.name()).is_some_and(Option::is_some)
        {
            continue;
        }
        if group_rescued.contains(member.name()) || !member.version().pre.is_empty() {
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
        fs::write(&self.path, &self.content).with_context(|| display_path(&self.path))
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
            Err(err) => return Err(err).context(display_path(&changelog_path)),
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
