use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use nodejs_semver::Version;
use tracing::debug;

use crate::{
    bump::{self, Bump, Prerelease},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    config::{Config, ResolvedGroups},
    package_json::PackageJson,
    pre::{PreJson, PreMode},
    snapshot::{Snapshot, SnapshotVersions},
    workspace::{Package, Versionable, Workspace},
};

pub struct PlannedVersion {
    pub workspace: Workspace,
    pub changeset_dir: PathBuf,
    pub pre: Option<PreJson>,
    // The release plan reports every unreleased changeset, the ones naming
    // only skipped packages included, so `changes` stays unfiltered beside
    // the skip-filtered `consumed_changes`.
    pub changes: Vec<LoadedChange>,
    pub consumed_changes: Vec<LoadedChange>,
    pub releases: Vec<PlannedRelease>,
}

fn pre_state(pre: Option<&PreJson>) -> Option<&PreJson> {
    pre.filter(|pre| pre.mode() == PreMode::Pre)
}

impl PlannedVersion {
    #[must_use]
    pub fn in_pre(&self) -> Option<&PreJson> {
        pre_state(self.pre.as_ref())
    }

    #[must_use]
    pub fn exiting_pre(&self) -> bool {
        matches!(&self.pre, Some(pre) if pre.mode() == PreMode::Exit)
    }
}

pub fn plan_version(
    workspace: Workspace,
    config: &Config,
    snapshot: Option<&Snapshot>,
) -> Result<PlannedVersion> {
    let changeset_dir = workspace.changeset_dir();
    let config_path = changeset_dir.join("config.json");
    let names: Vec<&str> = workspace
        .packages()
        .filter_map(Package::name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let groups = config
        .resolve_groups(&names)
        .with_context(|| config_path.display().to_string())?;

    let pre = PreJson::load(&changeset_dir)?;
    let in_pre = pre_state(pre.as_ref());
    let mut pre_tag = None;
    if let Some(pre) = in_pre {
        if snapshot.is_some() {
            bail!(
                "snapshot releases are not allowed in pre mode; run `changesette pre exit` first"
            );
        }
        pre_tag = Some(
            Prerelease::new(pre.tag())
                .with_context(|| format!("invalid pre tag {:?}", pre.tag()))?,
        );
    }
    let snapshot_versions = snapshot
        .map(|snapshot| SnapshotVersions::resolve(snapshot, config))
        .transpose()?;

    let mut changes = changeset::load(&changeset_dir)?;
    if in_pre.is_some() {
        // The `pre/` changesets were already consumed in this pre-release
        // cycle.
        changes.retain(|change| !change.in_pre);
    }
    let consumed_changes = filter_changes(&workspace, &changeset_dir, &changes)?;
    let releases = plan_releases(
        &workspace,
        &config_path,
        &consumed_changes,
        pre.as_ref(),
        pre_tag.as_ref(),
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

fn filter_changes(
    workspace: &Workspace,
    changeset_dir: &Path,
    changes: &[LoadedChange],
) -> Result<Vec<LoadedChange>> {
    // Membership is checked before the skip judgment so that a changeset
    // naming an unknown package always reports that rather than a
    // mixed-changeset error.
    for change in changes {
        let path = changeset_dir.join(change.rel_path());
        for (name, _) in &change.releases {
            let package = workspace
                .package(name)
                .with_context(|| path.display().to_string())?;
            if package.version().is_none() {
                bail!(
                    "{}: package `{name}` has no version in package.json",
                    path.display()
                );
            }
        }
    }

    let mut consumed = Vec::new();
    for change in changes {
        let mut skipped = Vec::new();
        let mut not_skipped = Vec::new();
        for (name, _) in &change.releases {
            if workspace.package(name)?.versionable().is_some() {
                not_skipped.push(name.as_str());
            } else {
                skipped.push(name.as_str());
            }
        }
        if skipped.is_empty() {
            consumed.push(change.clone());
        } else if !not_skipped.is_empty() {
            bail!(
                "{}: cannot mix skipped packages ({}) and not skipped packages ({})",
                changeset_dir.join(change.rel_path()).display(),
                quote_list(&skipped),
                quote_list(&not_skipped)
            );
        }
    }
    Ok(consumed)
}

fn quote_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub struct PlannedRelease {
    pub name: String,
    pub bump: Option<Bump>,
    pub old_version: Version,
    pub new_version: Version,
    pub changeset_ids: Vec<String>,
    pub changelog_entry: Option<String>,
}

fn plan_releases(
    workspace: &Workspace,
    config_path: &Path,
    changes: &[LoadedChange],
    pre: Option<&PreJson>,
    pre_tag: Option<&Prerelease>,
    snapshot: Option<&SnapshotVersions>,
    groups: &ResolvedGroups,
) -> Result<Vec<PlannedRelease>> {
    let mut max_bumps = changeset::max_bumps(changes);
    // The group passes run before the pre exit rescue so that a rescued
    // package does not pull its group along.
    let overrides = apply_groups(workspace, groups, pre_tag, &mut max_bumps)
        .with_context(|| config_path.display().to_string())?;
    if matches!(pre, Some(pre) if pre.mode() == PreMode::Exit) {
        rescue_prereleases(workspace, groups, &mut max_bumps)
            .with_context(|| config_path.display().to_string())?;
    }

    let mut releases = Vec::new();
    for (name, max_bump) in max_bumps {
        let versionable = resolve_versionable(workspace, name)?;
        let old_version = match overrides.old_versions.get(name) {
            Some(version) => version.clone(),
            None => versionable.version().clone(),
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
                    None => match pre_tag {
                        Some(tag) => match overrides.pre_counters.get(name) {
                            Some(&counter) => {
                                bump::next_pre_version_with(&old_version, max_bump, tag, counter)
                            }
                            None => bump::next_pre_version(&old_version, max_bump, tag),
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

fn resolve_versionable<'a>(workspace: &'a Workspace, name: &str) -> Result<Versionable<'a>> {
    workspace
        .package(name)?
        .versionable()
        .with_context(|| format!("package `{name}` is not versionable"))
}

// The skipped packages of a group count too: their versions bound the group
// even though the bump application excludes them.
fn group_version<'a>(workspace: &'a Workspace, kind: &str, name: &str) -> Result<&'a Version> {
    workspace
        .package(name)?
        .version()
        .with_context(|| format!("package `{name}` in a {kind:?} group has no version"))
}

struct GroupOverrides {
    old_versions: BTreeMap<String, Version>,
    pre_counters: BTreeMap<String, u64>,
}

// One pass per kind reaches the fixed point because config validation keeps
// the groups disjoint and changesette adds no dependents.
fn apply_groups<'a>(
    workspace: &'a Workspace,
    groups: &ResolvedGroups,
    pre_tag: Option<&Prerelease>,
    max_bumps: &mut BTreeMap<&'a str, Option<Bump>>,
) -> Result<GroupOverrides> {
    let mut old_versions = BTreeMap::new();
    for group in &groups.fixed {
        let Some(max_bump) = group_max_bump(group, max_bumps) else {
            continue;
        };
        let highest = group_highest_version(workspace, "fixed", group)?;
        for name in group {
            let Some(versionable) = workspace.package(name)?.versionable() else {
                continue;
            };
            let previous = max_bumps.insert(versionable.name(), Some(max_bump));
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
        let highest = group_highest_version(workspace, "linked", group)?;
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
        // The old_version override alone would miss a package whose version
        // is low but whose counter is high, so the counter is aligned
        // separately.
        for (kind, groups) in [("fixed", &groups.fixed), ("linked", &groups.linked)] {
            for group in groups {
                let mut counter = 0;
                for name in group {
                    counter = counter.max(bump::pre_counter(
                        group_version(workspace, kind, name)?,
                        tag,
                    ));
                }
                for name in group {
                    pre_counters.insert(name.clone(), counter);
                }
            }
        }
    }

    Ok(GroupOverrides {
        old_versions,
        pre_counters,
    })
}

fn group_max_bump(group: &[String], max_bumps: &BTreeMap<&str, Option<Bump>>) -> Option<Bump> {
    group
        .iter()
        .filter_map(|name| max_bumps.get(name.as_str()).copied().flatten())
        .max()
}

fn group_highest_version(workspace: &Workspace, kind: &str, group: &[String]) -> Result<Version> {
    let mut highest: Option<&Version> = None;
    for name in group {
        let version = group_version(workspace, kind, name)?;
        if highest.is_none_or(|h| version > h) {
            highest = Some(version);
        }
    }
    Ok(highest
        .expect("a group with a releasing package is nonempty")
        .clone())
}

// Each rescued package is planned at its own version: `next_version` then
// merely drops the pre-release, and the empty summary list renders a
// heading-only changelog section.
fn rescue_prereleases<'a>(
    workspace: &'a Workspace,
    groups: &ResolvedGroups,
    max_bumps: &mut BTreeMap<&'a str, Option<Bump>>,
) -> Result<()> {
    let mut group_rescued = BTreeSet::new();
    for (kind, groups) in [("fixed", &groups.fixed), ("linked", &groups.linked)] {
        for group in groups {
            let mut on_prerelease = false;
            for name in group {
                if group_version(workspace, kind, name)?.is_prerelease() {
                    on_prerelease = true;
                    break;
                }
            }
            if on_prerelease {
                group_rescued.extend(group.iter().map(String::as_str));
            }
        }
    }
    for versionable in workspace.versionables() {
        if max_bumps
            .get(versionable.name())
            .is_some_and(Option::is_some)
        {
            continue;
        }
        if group_rescued.contains(versionable.name()) || versionable.version().is_prerelease() {
            max_bumps.insert(versionable.name(), Some(Bump::Patch));
        }
    }
    Ok(())
}

pub struct StagedWrite {
    pub path: PathBuf,
    pub content: String,
}

impl StagedWrite {
    pub fn apply(&self) -> Result<()> {
        fs::write(&self.path, &self.content).with_context(|| self.path.display().to_string())
    }
}

pub fn stage_writes(
    workspace: &Workspace,
    releases: &[PlannedRelease],
) -> Result<Vec<StagedWrite>> {
    let mut writes = Vec::new();
    for release in releases {
        let Some(entry) = &release.changelog_entry else {
            continue;
        };
        let package = workspace.package(&release.name)?;
        let mut package_json = PackageJson::load(package.dir())?;
        package_json.set_version(&release.new_version)?;
        writes.push(StagedWrite {
            path: package_json.path().to_owned(),
            content: package_json.text(),
        });

        let changelog_path = package.dir().join("CHANGELOG.md");
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
