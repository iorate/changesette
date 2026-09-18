use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
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
    config::{Config, ResolvedGroups, UpdateInternalDependencies},
    dependency::{self, DependentsGraph, effective_range},
    package_json::PackageJson,
    pre::{PreJson, PreMode},
    range::{self, Target, Update},
    snapshot::{Snapshot, SnapshotVersions},
    workspace::{DependencyField, Package, PackageNotFound, RelDir, Versioned, Workspace},
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
    pub graph: DependentsGraph,
    pub dependency_updates: Vec<DependencyUpdate>,
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
    if in_pre.is_some() && snapshot.is_some() {
        bail!("snapshot releases are not allowed in pre mode; run `changesette pre exit` first");
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
    let graph = if config.manage_internal_dependencies {
        DependentsGraph::build(dependency::internal_dependencies(
            &workspace,
            config.bump_versions_with_workspace_protocol_only,
        ))
    } else {
        DependentsGraph::default()
    };
    let mut releases = plan_releases(
        &workspace,
        &config_path,
        &consumed_changes,
        pre.as_ref(),
        snapshot_versions.as_ref(),
        &groups,
        &graph,
    )?;
    let dependency_updates = plan_dependency_updates(
        &graph,
        &releases,
        snapshot_versions.is_some(),
        config.update_internal_dependencies,
    );
    render_changelog_entries(&consumed_changes, &dependency_updates, &mut releases);

    Ok(PlannedVersion {
        workspace,
        changeset_dir,
        pre,
        changes,
        consumed_changes,
        releases,
        graph,
        dependency_updates,
    })
}

pub struct DependencyUpdate {
    pub dependent: RelDir,
    pub dependency: RelDir,
    pub dependency_name: String,
    pub field: DependencyField,
    pub old: String,
    pub new: Option<String>,
}

fn plan_dependency_updates(
    graph: &DependentsGraph,
    releases: &[PlannedRelease],
    snapshot: bool,
    min: UpdateInternalDependencies,
) -> Vec<DependencyUpdate> {
    let mut targets = BTreeMap::new();
    for release in releases {
        let Some(bump) = release.bump else {
            continue;
        };
        let new_version = release.new_version.clone();
        let target = if snapshot {
            Target::Snapshot(new_version)
        } else {
            Target::Release(new_version)
        };
        targets.insert(&release.dir, (release.name.as_str(), bump, target));
    }
    let mut updates = Vec::new();
    for edge in graph.iter() {
        let Some((name, bump, target)) = targets.get(&edge.dependency) else {
            continue;
        };
        let Some(update) = range::update(&edge.spec, target, *bump, min) else {
            continue;
        };
        updates.push(DependencyUpdate {
            dependent: edge.dependent.clone(),
            dependency: edge.dependency.clone(),
            dependency_name: (*name).to_owned(),
            field: edge.field,
            old: edge.spec_text.clone(),
            new: match update {
                Update::Explicit(new) => Some(new),
                Update::Implicit => None,
            },
        });
    }
    updates.sort_by(|a, b| {
        (&a.dependent, &a.dependency, a.field).cmp(&(&b.dependent, &b.dependency, b.field))
    });
    updates
}

fn filter_changes(
    workspace: &Workspace,
    changeset_dir: &Path,
    changes: &[LoadedChange],
) -> Result<Vec<LoadedChange>> {
    // Membership is checked before the skip judgment so that a changeset
    // naming an unknown package always reports that rather than a
    // mixed-changeset error.
    let mut resolved = Vec::new();
    for change in changes {
        let path = changeset_dir.join(change.rel_path());
        let mut packages = Vec::new();
        for (name, _) in &change.releases {
            let package = workspace
                .package(name)
                .ok_or_else(|| PackageNotFound::new(name, workspace))
                .with_context(|| path.display().to_string())?;
            if package.version().is_none() {
                bail!(
                    "{}: package `{name}` has no version in package.json",
                    path.display()
                );
            }
            packages.push((name.as_str(), package));
        }
        resolved.push(packages);
    }

    let mut consumed = Vec::new();
    for (change, packages) in changes.iter().zip(resolved) {
        let mut skipped = Vec::new();
        let mut not_skipped = Vec::new();
        for (name, package) in packages {
            match package.skip_reason() {
                Some(reason) => skipped.push(format!("`{name}`: {reason}")),
                None => not_skipped.push(format!("`{name}`")),
            }
        }
        if skipped.is_empty() {
            consumed.push(change.clone());
        } else if !not_skipped.is_empty() {
            bail!(
                "{}: cannot mix skipped packages ({}) and not skipped packages ({})",
                changeset_dir.join(change.rel_path()).display(),
                skipped.join(", "),
                not_skipped.join(", ")
            );
        }
    }
    Ok(consumed)
}

pub struct PlannedRelease {
    pub dir: RelDir,
    pub name: String,
    pub bump: Option<Bump>,
    pub old_version: Version,
    pub new_version: Version,
    pub changeset_ids: Vec<String>,
    pub updated_dependencies: Vec<(String, Version)>,
    pub changelog_entry: Option<String>,
}

fn plan_releases(
    workspace: &Workspace,
    config_path: &Path,
    changes: &[LoadedChange],
    pre: Option<&PreJson>,
    snapshot: Option<&SnapshotVersions>,
    groups: &ResolvedGroups,
    graph: &DependentsGraph,
) -> Result<Vec<PlannedRelease>> {
    let pre_tag = pre_state(pre)
        .map(|pre| {
            Prerelease::new(pre.tag()).with_context(|| format!("invalid pre tag {:?}", pre.tag()))
        })
        .transpose()?;
    let pre_counters = match &pre_tag {
        Some(tag) => group_pre_counters(workspace, groups, tag)
            .with_context(|| config_path.display().to_string())?,
        None => BTreeMap::new(),
    };

    let mut drafts = initial_drafts(workspace, changes);
    // A package the fixed pass adds can leave a dependent's range, and that
    // dependent can raise its linked group, so the passes repeat until such
    // chains die out.
    loop {
        let dependents = add_dependents(
            workspace,
            graph,
            pre_tag.as_ref(),
            &pre_counters,
            &mut drafts,
        );
        let fixed = apply_fixed(workspace, &groups.fixed, &mut drafts)
            .with_context(|| config_path.display().to_string())?;
        let linked = apply_linked(workspace, &groups.linked, &mut drafts)
            .with_context(|| config_path.display().to_string())?;
        if !(dependents || fixed || linked) {
            break;
        }
    }
    // The group passes run before the pre exit rescue so that a rescued
    // package does not pull its group along.
    if matches!(pre, Some(pre) if pre.mode() == PreMode::Exit) {
        rescue_prereleases(workspace, groups, &mut drafts)
            .with_context(|| config_path.display().to_string())?;
    }

    let mut releases = Vec::new();
    for (rel_dir, draft) in &drafts {
        let name = draft.versioned.name();
        let changeset_ids = changes
            .iter()
            .filter(|change| change.releases.iter().any(|(n, _)| n == name))
            .map(LoadedChange::id)
            .collect();
        let new_version = match draft.bump {
            Some(bump) => next_version_of(
                &draft.old_version,
                bump,
                pre_tag.as_ref(),
                pre_counters.get(rel_dir).copied(),
                snapshot,
            ),
            None => draft.old_version.clone(),
        };
        releases.push(PlannedRelease {
            dir: rel_dir.clone(),
            name: name.to_owned(),
            bump: draft.bump,
            old_version: draft.old_version.clone(),
            new_version,
            changeset_ids,
            updated_dependencies: Vec::new(),
            changelog_entry: None,
        });
    }
    Ok(releases)
}

fn render_changelog_entries(
    changes: &[LoadedChange],
    dependency_updates: &[DependencyUpdate],
    releases: &mut [PlannedRelease],
) {
    let mut new_versions = BTreeMap::new();
    for release in releases.iter() {
        if release.bump.is_some() {
            new_versions.insert(release.dir.clone(), release.new_version.clone());
        }
    }
    for release in releases {
        if release.bump.is_none() {
            continue;
        }
        let name = release.name.as_str();
        let rel_dir = &release.dir;
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
        let updated_dependencies: Vec<(&str, &Version)> = dependency_updates
            .iter()
            .filter(|update| {
                update.dependent == *rel_dir
                    && matches!(
                        update.field,
                        DependencyField::Dependencies | DependencyField::PeerDependencies
                    )
            })
            .map(|update| {
                (
                    &update.dependency,
                    (
                        update.dependency_name.as_str(),
                        &new_versions[&update.dependency],
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        release.changelog_entry = Some(render_entry(&summaries, &updated_dependencies));
        release.updated_dependencies = updated_dependencies
            .into_iter()
            .map(|(name, version)| (name.to_owned(), version.clone()))
            .collect();
    }
}

struct Draft<'a> {
    versioned: Versioned<'a>,
    bump: Option<Bump>,
    // Fixed and linked groups plan every member against the group's highest
    // version rather than its own.
    old_version: Version,
}

impl Draft<'_> {
    fn new(versioned: Versioned<'_>, bump: Option<Bump>) -> Draft<'_> {
        Draft {
            versioned,
            bump,
            old_version: versioned.version().clone(),
        }
    }
}

type Drafts<'a> = BTreeMap<RelDir, Draft<'a>>;

fn initial_drafts<'a>(workspace: &'a Workspace, changes: &[LoadedChange]) -> Drafts<'a> {
    let mut drafts = BTreeMap::new();
    for (name, bump) in changeset::max_bumps(changes) {
        let versioned = workspace
            .package(name)
            .and_then(Package::versioned)
            .expect("a consumed changeset names only versioned packages");
        drafts.insert(
            versioned.package().rel_dir().clone(),
            Draft::new(versioned, bump),
        );
    }
    drafts
}

fn member<'a>(workspace: &'a Workspace, name: &str) -> &'a Package {
    workspace
        .package(name)
        .expect("a group member is a workspace package")
}

fn next_version_of(
    old_version: &Version,
    bump: Bump,
    pre_tag: Option<&Prerelease>,
    pre_counter: Option<u64>,
    snapshot: Option<&SnapshotVersions>,
) -> Version {
    match snapshot {
        Some(snapshot) => snapshot.apply(old_version, bump),
        None => match pre_tag {
            Some(tag) => match pre_counter {
                Some(counter) => bump::next_pre_version_with(old_version, bump, tag, counter),
                None => bump::next_pre_version(old_version, bump, tag),
            },
            None => bump::next_version(old_version, bump),
        },
    }
}

// The skipped packages of a group count too: their versions bound the group
// even though the bump application excludes them.
fn group_version<'a>(workspace: &'a Workspace, kind: &str, name: &str) -> Result<&'a Version> {
    member(workspace, name)
        .version()
        .with_context(|| format!("package `{name}` in a {kind:?} group has no version"))
}

// The judgment uses the plain next version even for a snapshot release, so a
// dependent joins the release on the same condition either way.
fn add_dependents<'a>(
    workspace: &'a Workspace,
    graph: &DependentsGraph,
    pre_tag: Option<&Prerelease>,
    pre_counters: &BTreeMap<RelDir, u64>,
    drafts: &mut Drafts<'a>,
) -> bool {
    let mut changed = false;
    let nexts: Vec<(RelDir, &str, Version, Version)> = drafts
        .iter()
        .filter_map(|(rel_dir, draft)| {
            let bump = draft.bump?;
            let next = next_version_of(
                &draft.old_version,
                bump,
                pre_tag,
                pre_counters.get(rel_dir).copied(),
                None,
            );
            Some((
                rel_dir.clone(),
                draft.versioned.name(),
                draft.old_version.clone(),
                next,
            ))
        })
        .collect();
    for (rel_dir, name, old_version, next) in nexts {
        for edge in graph.dependents(&rel_dir) {
            if edge.field == DependencyField::DevDependencies {
                continue;
            }
            let Some(dependent) = workspace[&edge.dependent].versioned() else {
                continue;
            };
            if drafts
                .get(&edge.dependent)
                .is_some_and(|draft| draft.bump.is_some())
            {
                continue;
            }
            let Some(range) = effective_range(&edge.spec, &old_version) else {
                continue;
            };
            if range.satisfies(&next) {
                continue;
            }
            debug!(
                "{}: bumped as a dependent of `{name}` ({range} does not include {next})",
                dependent.package()
            );
            drafts.insert(
                edge.dependent.clone(),
                Draft::new(dependent, Some(Bump::Patch)),
            );
            changed = true;
        }
    }
    changed
}

fn apply_fixed<'a>(
    workspace: &'a Workspace,
    groups: &[Vec<String>],
    drafts: &mut Drafts<'a>,
) -> Result<bool> {
    let mut changed = false;
    for group in groups {
        let Some(max_bump) = group_max_bump(workspace, group, drafts) else {
            continue;
        };
        let highest = group_highest_version(workspace, "fixed", group)?;
        for name in group {
            let Some(versioned) = member(workspace, name).versioned() else {
                continue;
            };
            let rel_dir = versioned.package().rel_dir();
            let previous = drafts.get(rel_dir);
            if previous
                .is_some_and(|draft| draft.bump == Some(max_bump) && draft.old_version == highest)
            {
                continue;
            }
            if previous.and_then(|draft| draft.bump) != Some(max_bump) {
                debug!(
                    "`{name}`: the \"fixed\" group raises the bump to {} (planning against {highest})",
                    max_bump.as_str()
                );
            }
            drafts.insert(
                rel_dir.clone(),
                Draft {
                    versioned,
                    bump: Some(max_bump),
                    old_version: highest.clone(),
                },
            );
            changed = true;
        }
    }
    Ok(changed)
}

fn apply_linked(
    workspace: &Workspace,
    groups: &[Vec<String>],
    drafts: &mut Drafts<'_>,
) -> Result<bool> {
    let mut changed = false;
    for group in groups {
        let Some(max_bump) = group_max_bump(workspace, group, drafts) else {
            continue;
        };
        let highest = group_highest_version(workspace, "linked", group)?;
        for name in group {
            let Some(draft) = drafts.get_mut(member(workspace, name).rel_dir()) else {
                continue;
            };
            if draft.bump.is_none()
                || (draft.bump == Some(max_bump) && draft.old_version == highest)
            {
                continue;
            }
            if draft.bump != Some(max_bump) {
                debug!(
                    "`{name}`: the \"linked\" group raises the bump to {} (planning against {highest})",
                    max_bump.as_str()
                );
            }
            draft.bump = Some(max_bump);
            draft.old_version = highest.clone();
            changed = true;
        }
    }
    Ok(changed)
}

fn group_max_bump(workspace: &Workspace, group: &[String], drafts: &Drafts<'_>) -> Option<Bump> {
    let mut max_bump = None;
    for name in group {
        let bump = drafts
            .get(member(workspace, name).rel_dir())
            .and_then(|draft| draft.bump);
        max_bump = max_bump.max(bump);
    }
    max_bump
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

// The old_version override alone would miss a package whose version is low
// but whose counter is high, so the counter is aligned separately.
fn group_pre_counters(
    workspace: &Workspace,
    groups: &ResolvedGroups,
    tag: &Prerelease,
) -> Result<BTreeMap<RelDir, u64>> {
    let mut pre_counters = BTreeMap::new();
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
                pre_counters.insert(member(workspace, name).rel_dir().clone(), counter);
            }
        }
    }
    Ok(pre_counters)
}

// Each rescued package is planned at its own version: `next_version` then
// merely drops the pre-release, and the empty summary list renders a
// heading-only changelog section.
fn rescue_prereleases<'a>(
    workspace: &'a Workspace,
    groups: &ResolvedGroups,
    drafts: &mut Drafts<'a>,
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
                for name in group {
                    group_rescued.insert(member(workspace, name).rel_dir());
                }
            }
        }
    }
    for versioned in workspace.versioned() {
        let rel_dir = versioned.package().rel_dir();
        if drafts
            .get(rel_dir)
            .is_some_and(|draft| draft.bump.is_some())
        {
            continue;
        }
        if group_rescued.contains(rel_dir) || versioned.version().is_prerelease() {
            drafts.insert(rel_dir.clone(), Draft::new(versioned, Some(Bump::Patch)));
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
    dependency_updates: &[DependencyUpdate],
) -> Result<Vec<StagedWrite>> {
    let mut manifests: BTreeMap<RelDir, PackageJson> = BTreeMap::new();
    let mut writes = Vec::new();
    for release in releases {
        let Some(entry) = &release.changelog_entry else {
            continue;
        };
        let package = &workspace[&release.dir];
        let mut package_json = PackageJson::load(package.dir())?;
        package_json.set_version(&release.new_version)?;
        manifests.insert(package.rel_dir().clone(), package_json);

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
    for update in dependency_updates {
        let Some(new) = &update.new else {
            continue;
        };
        let package_json = match manifests.entry(update.dependent.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                entry.insert(PackageJson::load(workspace[&update.dependent].dir())?)
            }
        };
        package_json.set_dependency(update.field, &update.dependency_name, new)?;
    }
    writes.extend(manifests.into_values().map(|package_json| StagedWrite {
        path: package_json.path().to_owned(),
        content: package_json.text(),
    }));
    Ok(writes)
}
