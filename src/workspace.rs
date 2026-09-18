mod pattern;
mod walk;

use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fmt, fs, io,
    ops::Index,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use file_id::{FileId, get_file_id};
use nodejs_semver::Version;
use saphyr::{LoadableYamlNode, Yaml};
use serde_json::{Map, Value};
use tracing::{debug, warn};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
}

impl PackageManager {
    fn workspace_kind(self) -> &'static str {
        match self {
            PackageManager::Npm => "npm workspace",
            PackageManager::Yarn => "yarn workspace",
            PackageManager::Pnpm => "pnpm workspace",
        }
    }
}

pub struct Root {
    dir: PathBuf,
    pm: Option<PackageManager>,
    // The candidates `find` already enumerated to confirm an npm reroot,
    // kept so that `load` does not walk the workspace (and warn) a second
    // time.
    reroot: Option<Vec<Candidate>>,
}

impl Root {
    #[must_use]
    pub fn new(dir: PathBuf) -> Root {
        let pm = if probe_is_file(&dir.join("pnpm-workspace.yaml")) {
            Some(PackageManager::Pnpm)
        } else if probe_is_file(&dir.join("yarn.lock")) {
            Some(PackageManager::Yarn)
        } else if probe_is_file(&dir.join("package.json")) {
            Some(PackageManager::Npm)
        } else {
            None
        };
        Root {
            dir,
            pm,
            reroot: None,
        }
    }

    pub fn find(cwd: &Path) -> Result<Root> {
        for dir in cwd.ancestors() {
            if probe_is_file(&dir.join("pnpm-workspace.yaml")) {
                return Ok(Root {
                    dir: dir.to_path_buf(),
                    pm: Some(PackageManager::Pnpm),
                    reroot: None,
                });
            }
            if probe_is_file(&dir.join("yarn.lock")) {
                return Ok(Root {
                    dir: dir.to_path_buf(),
                    pm: Some(PackageManager::Yarn),
                    reroot: None,
                });
            }
        }

        let mut prefix = None;
        for dir in cwd.ancestors() {
            let path = dir.join("package.json");
            if !probe_is_file(&path) {
                continue;
            }
            let Some(prefix_dir) = &prefix else {
                prefix = Some(dir.to_path_buf());
                continue;
            };
            let value = match read_manifest(&path) {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(err) => {
                    warn!("{err:#}: passed over while looking for an npm workspace root");
                    continue;
                }
            };
            let Some(patterns) = workspaces_patterns(&value, &path) else {
                continue;
            };
            // The candidate prefix is looked for among every matched directory
            // holding a package.json, so the qualification must not run
            // first.
            let candidates = collect_candidates(dir, &path, &patterns, PackageManager::Npm)?;
            if lists_dir(&candidates, prefix_dir)? {
                return Ok(Root {
                    dir: dir.to_path_buf(),
                    pm: Some(PackageManager::Npm),
                    reroot: Some(candidates),
                });
            }
        }

        let (dir, pm) = match prefix {
            Some(dir) => (dir, Some(PackageManager::Npm)),
            None => (cwd.to_path_buf(), None),
        };
        Ok(Root {
            dir,
            pm,
            reroot: None,
        })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

// The root is always the physical path: `Root::find` and `Workspace::load`
// rely on it holding no `.` or `..` component, as they climb by `parent()`.
pub fn resolve_root(dir: &Path) -> Result<PathBuf> {
    let root = dunce::canonicalize(dir)?;
    if !fs::metadata(&root)?.is_dir() {
        bail!("not a directory")
    }
    Ok(root)
}

#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
    packages: BTreeMap<RelDir, Package>,
}

impl Workspace {
    pub fn load(root: Root, config: &Config, cli_ignore: &[String]) -> Result<Workspace> {
        let Root {
            dir: root,
            pm,
            reroot,
        } = root;
        if let Some(rel_dirs) = &config.packages {
            let mut candidates = Vec::new();
            for entry in rel_dirs {
                let (dir, rel_dir) = resolve_rel_dir(&root, entry)?;
                let manifest = dir.join("package.json");
                let Some(value) = read_manifest(&manifest)? else {
                    bail!(
                        "{}: not found (listed in \"changesette.packages\")",
                        manifest.display()
                    )
                };
                candidates.push(Candidate {
                    dir,
                    rel_dir,
                    manifest,
                    value,
                });
            }
            return Workspace::new(
                root,
                "workspace listed by changesette.packages",
                qualify_candidates(candidates)?,
                config,
                cli_ignore,
            );
        }
        let Some(pm) = pm else {
            warn!("{}: no workspace found", root.display());
            return Workspace::new(root, "no workspace", Vec::new(), config, cli_ignore);
        };
        let packages = if let Some(candidates) = reroot {
            qualify_candidates(candidates)?
        } else {
            let (manifest, patterns) = read_patterns(&root, pm)?;
            collect_packages(&root, &manifest, &patterns, pm)?
        };
        Workspace::new(root, pm.workspace_kind(), packages, config, cli_ignore)
    }

    // The one construction point, so that every loading path reports the
    // final package list — the shortest answer to "why is my package not
    // found".
    fn new(
        root: PathBuf,
        kind: &'static str,
        packages: Vec<Package>,
        config: &Config,
        cli_ignore: &[String],
    ) -> Result<Workspace> {
        let mut workspace = Workspace {
            root,
            packages: packages
                .into_iter()
                .map(|package| (package.rel_dir.clone(), package))
                .collect(),
        };
        if workspace.packages.is_empty() {
            debug!("{}: {kind}, no packages", workspace.root.display());
        } else {
            // The list is built inside the macro so that the event macro's
            // enabled check makes it free at the default level.
            debug!(
                "{}: {kind}, packages: {}",
                workspace.root.display(),
                workspace
                    .packages
                    .values()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        workspace.mark_skipped(config, cli_ignore)?;
        Ok(workspace)
    }

    fn mark_skipped(&mut self, config: &Config, cli_ignore: &[String]) -> Result<()> {
        let ignore = if config.has_ignore() {
            ensure!(
                cli_ignore.is_empty(),
                "the --ignore option cannot be used while ignore is defined in .changeset/config.json; use only one of them"
            );
            config.resolve_ignore(self.packages.values().filter_map(Package::name))
        } else {
            for name in cli_ignore {
                self.package(name)
                    .ok_or_else(|| PackageNotFound::new(name, self))
                    .context("invalid `--ignore` value")?;
            }
            cli_ignore.to_vec()
        };
        let mut namesakes: BTreeMap<&str, Vec<&RelDir>> = BTreeMap::new();
        for package in self.packages.values() {
            if let Some(name) = package.name() {
                namesakes.entry(name).or_default().push(&package.rel_dir);
            }
        }
        let mut resolved = BTreeMap::new();
        for (name, rel_dirs) in namesakes {
            let winner = rel_dirs
                .last()
                .expect("a name is recorded with the package using it");
            if rel_dirs.len() > 1 {
                warn!(
                    "the name `{name}` is used by {}; `{name}` resolves to {winner}",
                    rel_dirs
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            resolved.insert(name.to_owned(), (*winner).clone());
        }
        for package in self.packages.values_mut() {
            let reason = match (&package.name, &package.version) {
                (None, _) => SkipReason::NoName,
                (Some(name), _) if resolved[name.as_str()] != package.rel_dir => {
                    SkipReason::Shadowed(resolved[name.as_str()].clone())
                }
                (Some(name), _) if ignore.contains(name) => SkipReason::Ignored,
                (_, None) => SkipReason::NoVersion,
                _ if package.private && !config.private_packages_version => SkipReason::Private,
                _ => continue,
            };
            debug!("{package}: skipped: {reason}");
            package.skip_reason = Some(reason);
        }
        Ok(())
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn changeset_dir(&self) -> PathBuf {
        self.root.join(".changeset")
    }

    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.values()
    }

    pub fn versioned(&self) -> impl Iterator<Item = Versioned<'_>> {
        self.packages.values().filter_map(Package::versioned)
    }

    #[must_use]
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages
            .values()
            .rev()
            .find(|package| package.name.as_deref() == Some(name))
    }
}

#[derive(Debug)]
pub struct PackageNotFound {
    name: String,
    known: Vec<String>,
}

impl PackageNotFound {
    #[must_use]
    pub fn new(name: &str, workspace: &Workspace) -> PackageNotFound {
        let known: BTreeSet<&str> = workspace
            .packages
            .values()
            .filter_map(Package::name)
            .collect();
        PackageNotFound {
            name: name.to_owned(),
            known: known.into_iter().map(str::to_owned).collect(),
        }
    }
}

impl fmt::Display for PackageNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.known.is_empty() {
            write!(
                f,
                "package `{}` not found: the workspace has no named packages",
                self.name
            )
        } else {
            write!(
                f,
                "package `{}` not found; known packages: {}",
                self.name,
                self.known.join(", ")
            )
        }
    }
}

impl std::error::Error for PackageNotFound {}

impl Index<&RelDir> for Workspace {
    type Output = Package;

    fn index(&self, rel_dir: &RelDir) -> &Package {
        &self.packages[rel_dir]
    }
}

#[derive(Debug)]
pub struct Package {
    name: Option<String>,
    version: Option<Version>,
    dir: PathBuf,
    rel_dir: RelDir,
    private: bool,
    dependencies: Vec<Dependency>,
    skip_reason: Option<SkipReason>,
}

impl Package {
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn version(&self) -> Option<&Version> {
        self.version.as_ref()
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn rel_dir(&self) -> &RelDir {
        &self.rel_dir
    }

    #[must_use]
    pub fn private(&self) -> bool {
        self.private
    }

    #[must_use]
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    #[must_use]
    pub fn skip_reason(&self) -> Option<&SkipReason> {
        self.skip_reason.as_ref()
    }

    #[must_use]
    pub fn versioned(&self) -> Option<Versioned<'_>> {
        if self.skip_reason.is_some() {
            return None;
        }
        Some(Versioned {
            package: self,
            name: self.name.as_deref()?,
            version: self.version.as_ref()?,
        })
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name.as_deref().unwrap_or("<unnamed>"),
            self.rel_dir
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    NoName,
    Shadowed(RelDir),
    Ignored,
    NoVersion,
    Private,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::NoName => f.write_str("no name"),
            SkipReason::Shadowed(rel_dir) => write!(f, "shadowed by {rel_dir}"),
            SkipReason::Ignored => f.write_str("ignored"),
            SkipReason::NoVersion => f.write_str("no version"),
            SkipReason::Private => f.write_str("private"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Versioned<'a> {
    package: &'a Package,
    name: &'a str,
    version: &'a Version,
}

impl<'a> Versioned<'a> {
    #[must_use]
    pub fn package(&self) -> &'a Package {
        self.package
    }

    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    #[must_use]
    pub fn version(&self) -> &'a Version {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyField {
    Dependencies,
    DevDependencies,
    PeerDependencies,
    OptionalDependencies,
}

impl DependencyField {
    pub const ALL: [DependencyField; 4] = [
        DependencyField::Dependencies,
        DependencyField::DevDependencies,
        DependencyField::PeerDependencies,
        DependencyField::OptionalDependencies,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DependencyField::Dependencies => "dependencies",
            DependencyField::DevDependencies => "devDependencies",
            DependencyField::PeerDependencies => "peerDependencies",
            DependencyField::OptionalDependencies => "optionalDependencies",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub field: DependencyField,
    pub name: String,
    pub spec: String,
}

// `/`-separated, `.` for the root itself, and climbing only by leading `..`
// segments; `rel_dir_between` and `join` are its only sources.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelDir(String);

impl RelDir {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn join(&self, rel: &str) -> RelDir {
        let mut parts: Vec<&str> = if self.0 == "." {
            Vec::new()
        } else {
            self.0.split('/').collect()
        };
        for seg in rel.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.last().is_none_or(|last| *last == "..") {
                        parts.push("..");
                    } else {
                        parts.pop();
                    }
                }
                _ => parts.push(seg),
            }
        }
        RelDir(if parts.is_empty() {
            ".".to_owned()
        } else {
            parts.join("/")
        })
    }
}

impl fmt::Display for RelDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Borrow<str> for RelDir {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// A segment is pushed only when it parses as exactly one `Normal` component:
// `PathBuf::push` re-parses the segment, and a prefix in it (`C:` or `C:x` on
// Windows) would silently replace the directory built so far.
fn resolve_rel_dir(root: &Path, entry: &str) -> Result<(PathBuf, RelDir)> {
    let mut dir = root.to_path_buf();
    for seg in entry.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                let Some(parent) = dir.parent() else {
                    bail!(
                        "invalid \"changesette.packages\" entry {entry:?}: escapes the filesystem root"
                    )
                };
                dir = parent.to_path_buf();
            }
            _ => {
                let mut components = Path::new(seg).components();
                if !matches!(
                    (components.next(), components.next()),
                    (Some(Component::Normal(_)), None)
                ) {
                    bail!(
                        "invalid \"changesette.packages\" entry {entry:?}: {seg:?} is not a directory name"
                    )
                }
                dir.push(seg);
            }
        }
    }
    let rel_dir = rel_dir_between(root, &dir);
    Ok((dir, rel_dir))
}

// Purely lexical: `dir` is built from `root` by `parent()` and `push` only,
// so the two share every component up to where `dir` climbed away.
#[must_use]
pub fn rel_dir_between(root: &Path, dir: &Path) -> RelDir {
    let mut root_components = root.components().peekable();
    let mut dir_components = dir.components().peekable();
    while let (Some(a), Some(b)) = (root_components.peek(), dir_components.peek()) {
        if a != b {
            break;
        }
        root_components.next();
        dir_components.next();
    }
    let mut parts: Vec<String> = root_components.map(|_| "..".to_owned()).collect();
    parts.extend(
        dir_components.map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );
    RelDir(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

fn read_patterns(root: &Path, pm: PackageManager) -> Result<(PathBuf, Vec<String>)> {
    match pm {
        PackageManager::Pnpm => {
            let manifest = root.join("pnpm-workspace.yaml");
            let patterns = read_pnpm_manifest(&manifest)?
                .and_then(|doc| pnpm_patterns(&doc, &manifest))
                .unwrap_or_default();
            Ok((manifest, patterns))
        }
        PackageManager::Npm | PackageManager::Yarn => {
            let manifest = root.join("package.json");
            let patterns = read_manifest(&manifest)?
                .and_then(|value| workspaces_patterns(&value, &manifest))
                .unwrap_or_default();
            Ok((manifest, patterns))
        }
    }
}

// An empty or comment-only file holds no document, making it a settings-only
// root.
fn read_pnpm_manifest(path: &Path) -> Result<Option<Yaml<'static>>> {
    let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let docs = match Yaml::load_from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
        Ok(docs) => docs,
        Err(err) => bail!("{}: invalid YAML: {err}", path.display()),
    };
    Ok(docs.into_iter().next())
}

fn pnpm_patterns(doc: &Yaml, path: &Path) -> Option<Vec<String>> {
    let packages = doc.as_mapping_get("packages");
    if packages.is_none() && doc.is_mapping() {
        return None;
    }
    packages
        .and_then(Yaml::as_vec)
        .and_then(|items| all_strings(items.iter().map(Yaml::as_str)))
        .or_else(|| {
            warn!(
                "{}: must be a mapping whose \"packages\" is a list of strings: ignored",
                path.display()
            );
            None
        })
}

fn workspaces_patterns(value: &Value, path: &Path) -> Option<Vec<String>> {
    let workspaces = value.get("workspaces")?;
    let items = match workspaces {
        Value::Object(object) => object.get("packages").and_then(Value::as_array),
        _ => workspaces.as_array(),
    };
    items
        .and_then(|items| all_strings(items.iter().map(Value::as_str)))
        .or_else(|| {
            warn!(
                "{}: \"workspaces\" must be an array of strings or an object whose \"packages\" is an array of strings: ignored",
                path.display()
            );
            None
        })
}

fn all_strings<'a>(items: impl IntoIterator<Item = Option<&'a str>>) -> Option<Vec<String>> {
    items
        .into_iter()
        .map(|item| item.map(str::to_owned))
        .collect()
}

// BOM'd manifests exist in the wild, so the BOM is stripped before parsing.
fn read_manifest(path: &Path) -> Result<Option<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
        .with_context(|| path.display().to_string())?;
    Ok(Some(value))
}

// Unlike `read_manifest`, a BOM is deliberately not accepted.
pub fn read_json(path: &Path) -> Result<Option<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let value = serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    Ok(Some(value))
}

pub struct Candidate {
    dir: PathBuf,
    rel_dir: RelDir,
    manifest: PathBuf,
    value: Value,
}

fn collect_packages(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
) -> Result<Vec<Package>> {
    qualify_candidates(collect_candidates(root, manifest, patterns, pm)?)
}

fn collect_candidates(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    // Yarn expands every package's own `workspaces` field in turn (its
    // worktrees), a declaration's negations reaching only its own directory.
    let mut queue = VecDeque::from([(
        root.to_path_buf(),
        manifest.to_path_buf(),
        patterns.to_vec(),
    )]);
    // Keyed by the file id, so that a package declaring a symlink to itself is
    // not requeued forever; the root goes in first, as a `..` pattern lists it
    // again. Only the queue is guarded, leaving the alias spelling to
    // `collapse_aliases`.
    let mut visited = HashSet::new();
    if pm == PackageManager::Yarn {
        visited.insert(dir_id(root)?);
    }
    while let Some((dir, manifest, patterns)) = queue.pop_front() {
        for child_dir in enumerate(&dir, &manifest, &patterns)?.into_values() {
            let rel_dir = rel_dir_between(root, &child_dir);
            let path = child_dir.join("package.json");
            let Some(value) = read_manifest(&path)? else {
                continue;
            };
            if pm == PackageManager::Yarn
                && rel_dir.as_str() != "."
                && let Some(declared) = workspaces_patterns(&value, &path)
                && visited.insert(dir_id(&child_dir)?)
            {
                queue.push_back((child_dir.clone(), path.clone(), declared));
            }
            candidates.push(Candidate {
                dir: child_dir,
                rel_dir,
                manifest: path,
                value,
            });
        }
    }
    Ok(candidates)
}

fn enumerate(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
) -> Result<BTreeMap<RelDir, PathBuf>> {
    let mut positives = Vec::new();
    let mut negations = Vec::new();
    for original in patterns {
        let (negated, compiled) = pattern::compile(original).with_context(|| {
            format!(
                "{}: invalid workspace pattern {original:?}",
                manifest.display()
            )
        })?;
        if compiled.is_empty() {
            debug!(
                "{}: the workspace pattern {original:?} matches nothing",
                manifest.display()
            );
            continue;
        }
        if negated {
            negations.extend(compiled);
        } else {
            positives.extend(compiled);
        }
    }

    let mut candidates = walk::collect(root, &positives, &negations);
    if probe_is_file(&root.join("package.json")) {
        candidates.insert(rel_dir_between(root, root), root.to_path_buf());
    }
    Ok(candidates)
}

fn qualify_candidates(candidates: Vec<Candidate>) -> Result<Vec<Package>> {
    let mut packages: Vec<Package> = candidates
        .into_iter()
        .filter_map(|candidate| {
            qualify(
                &candidate.value,
                candidate.dir,
                candidate.rel_dir,
                &candidate.manifest,
            )
        })
        .collect();
    packages.sort_by(|a, b| (&a.name, &a.rel_dir).cmp(&(&b.name, &b.rel_dir)));
    collapse_aliases(&mut packages)?;
    Ok(packages)
}

// A missing `name` or `version` is only reported at debug level — fixture,
// private-root, and docs-site manifests omit them legitimately — while a key
// carrying an invalid value can only be a mistake and warns.
fn qualify(value: &Value, dir: PathBuf, rel_dir: RelDir, path: &Path) -> Option<Package> {
    let Some(object) = value.as_object() else {
        warn!(
            "{}: not a workspace package: the manifest is not a JSON object",
            path.display()
        );
        return None;
    };
    Some(Package {
        name: qualify_name(object.get("name"), path),
        version: qualify_version(object.get("version"), path),
        dir,
        rel_dir,
        private: object
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        dependencies: qualify_dependencies(object, path),
        skip_reason: None,
    })
}

fn qualify_dependencies(object: &Map<String, Value>, path: &Path) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    for field in DependencyField::ALL {
        let Some(value) = object.get(field.as_str()) else {
            continue;
        };
        let Some(entries) = value.as_object() else {
            warn!(
                "{}: \"{}\" is not an object: ignored",
                path.display(),
                field.as_str()
            );
            continue;
        };
        for (name, spec) in entries {
            let Some(spec) = spec.as_str() else {
                warn!(
                    "{}: {name:?} in \"{}\" is not a string: ignored",
                    path.display(),
                    field.as_str()
                );
                continue;
            };
            dependencies.push(Dependency {
                field,
                name: name.clone(),
                spec: spec.to_owned(),
            });
        }
    }
    dependencies
}

fn qualify_name(value: Option<&Value>, path: &Path) -> Option<String> {
    match value {
        None => {
            debug!("{}: has no \"name\"", path.display());
            None
        }
        Some(Value::String(name)) if name.is_empty() => {
            warn!("{}: \"name\" is an empty string: ignored", path.display());
            None
        }
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => {
            warn!("{}: \"name\" is not a string: ignored", path.display());
            None
        }
    }
}

fn qualify_version(value: Option<&Value>, path: &Path) -> Option<Version> {
    match value {
        None => {
            debug!("{}: has no \"version\"", path.display());
            None
        }
        Some(Value::String(version)) => {
            let Ok(version) = version.parse::<Version>() else {
                warn!(
                    "{}: \"version\" {version:?} is not a valid semver: ignored",
                    path.display()
                );
                return None;
            };
            Some(version)
        }
        Some(_) => {
            warn!("{}: \"version\" is not a string: ignored", path.display());
            None
        }
    }
}

// Aliases of one physical directory are one package and collapse into the
// first. Expects `packages` sorted by (name, rel_dir) with a stable sort:
// aliases collapse into the smallest rel_dir, and one directory spelled twice
// keeps its first listing.
fn collapse_aliases(packages: &mut Vec<Package>) -> Result<()> {
    let mut iter = std::mem::take(packages).into_iter().peekable();
    while let Some(first) = iter.next() {
        if iter.peek().is_none_or(|next| next.name != first.name) {
            packages.push(first);
            continue;
        }
        let mut group = vec![(dir_id(&first.dir)?, first)];
        while let Some(package) = iter.next_if(|next| next.name == group[0].1.name) {
            let id = dir_id(&package.dir)?;
            if !group.iter().any(|(kept, _)| *kept == id) {
                group.push((id, package));
            }
        }
        packages.extend(group.into_iter().map(|(_, package)| package));
    }
    Ok(())
}

fn lists_dir(candidates: &[Candidate], dir: &Path) -> Result<bool> {
    let id = dir_id(dir)?;
    for candidate in candidates {
        if dir_id(&candidate.dir)? == id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dir_id(dir: &Path) -> Result<FileId> {
    get_file_id(dir).with_context(|| dir.display().to_string())
}

#[must_use]
pub fn probe_is_file(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        Err(err) => {
            report_fs_error(path, &err);
            false
        }
    }
}

pub fn report_fs_error(path: &Path, err: &io::Error) {
    // Plain absence — NotFound from a missing or dangling path, NotADirectory
    // from a path crossing a regular file — is an ordinary no-match for every
    // caller; any other error (permissions, a symlink loop) can silently drop
    // a package and is worth a warning, though never worth aborting over.
    if !matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    ) {
        warn!("{}: {err}", path.display());
    }
}
