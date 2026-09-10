mod pattern;
mod walk;

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use file_id::{FileId, get_file_id};
use saphyr::{LoadableYamlNode, Yaml};
use semver::Version;
use serde_json::Value;
use tracing::{debug, warn};

#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
    members: Vec<Member>,
}

#[derive(Debug)]
pub struct Member {
    name: String,
    dir: PathBuf,
    rel_dir: String,
    version: Version,
    private: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
}

impl PackageManager {
    fn as_str(self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Yarn => "yarn",
            PackageManager::Pnpm => "pnpm",
        }
    }
}

pub struct Root {
    dir: PathBuf,
    pm: PackageManager,
    // The packages `find` already enumerated to confirm an npm reroot, kept
    // so that `load` does not walk the workspace (and warn) a second time.
    reroot: Option<Vec<Package>>,
}

impl Root {
    #[must_use]
    pub fn new(dir: PathBuf) -> Root {
        let pm = if probe_is_file(&dir.join("pnpm-workspace.yaml")) {
            PackageManager::Pnpm
        } else if probe_is_file(&dir.join("yarn.lock")) {
            PackageManager::Yarn
        } else {
            PackageManager::Npm
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
                    pm: PackageManager::Pnpm,
                    reroot: None,
                });
            }
            if probe_is_file(&dir.join("yarn.lock")) {
                return Ok(Root {
                    dir: dir.to_path_buf(),
                    pm: PackageManager::Yarn,
                    reroot: None,
                });
            }
        }

        let pm = PackageManager::Npm;
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
            // holding a package.json, so the member qualification (and the
            // duplicate-name exclusion) must not run first.
            let packages = collect_packages(dir, &path, &patterns, pm)?;
            if lists_dir(&packages, prefix_dir)? {
                return Ok(Root {
                    dir: dir.to_path_buf(),
                    pm,
                    reroot: Some(packages),
                });
            }
        }

        Ok(Root {
            dir: prefix.unwrap_or_else(|| cwd.to_path_buf()),
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

impl Workspace {
    pub fn load(root: Root, rel_dirs: Option<&[String]>) -> Result<Workspace> {
        let Root {
            dir: root,
            pm,
            reroot,
        } = root;
        if let Some(rel_dirs) = rel_dirs {
            let mut packages = Vec::new();
            for entry in rel_dirs {
                let (dir, rel_dir) = resolve_rel_dir(&root, entry)?;
                let manifest = dir.join("package.json");
                let Some(value) = read_manifest(&manifest)? else {
                    bail!(
                        "{}: not found (listed in \"changesette.packages\")",
                        manifest.display()
                    )
                };
                packages.push(Package {
                    dir,
                    rel_dir,
                    manifest,
                    value,
                });
            }
            return Ok(Workspace::new(
                root,
                "packages from config",
                qualify_packages(packages)?,
            ));
        }
        let members = if let Some(packages) = reroot {
            qualify_packages(packages)?
        } else {
            let (manifest, patterns) = read_patterns(&root, pm)?;
            collect_members(&root, &manifest, &patterns, pm)?
        };
        Ok(Workspace::new(root, pm.as_str(), members))
    }

    // The one construction point, so that every loading path reports the
    // final member list — the shortest answer to "why is my package not
    // found".
    fn new(root: PathBuf, source: &'static str, members: Vec<Member>) -> Workspace {
        if members.is_empty() {
            debug!("workspace {} ({source}): no members", root.display());
        } else {
            // The list is built inside the macro so that the event macro's
            // enabled check makes it free at the default level.
            debug!(
                "workspace {} ({source}): members: {}",
                root.display(),
                members
                    .iter()
                    .map(|member| format!("{} ({})", member.name, member.rel_dir))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Workspace { root, members }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn changeset_dir(&self) -> PathBuf {
        self.root.join(".changeset")
    }

    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn member(&self, name: &str) -> Result<&Member> {
        if let Some(member) = self.members.iter().find(|member| member.name == name) {
            return Ok(member);
        }
        if self.members.is_empty() {
            bail!("package `{name}` not found: the workspace has no members")
        }
        let known = self
            .members
            .iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("package `{name}` not found; known packages: {known}")
    }
}

impl Member {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn rel_dir(&self) -> &str {
        &self.rel_dir
    }

    #[must_use]
    pub fn version(&self) -> &Version {
        &self.version
    }

    #[must_use]
    pub fn private(&self) -> bool {
        self.private
    }
}

// A segment is pushed only when it parses as exactly one `Normal` component:
// `PathBuf::push` re-parses the segment, and a prefix in it (`C:` or `C:x` on
// Windows) would silently replace the directory built so far.
fn resolve_rel_dir(root: &Path, entry: &str) -> Result<(PathBuf, String)> {
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
pub(crate) fn rel_dir_between(root: &Path, dir: &Path) -> String {
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
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
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
pub(crate) fn read_json(path: &Path) -> Result<Option<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let value = serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    Ok(Some(value))
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

fn collect_members(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
) -> Result<Vec<Member>> {
    qualify_packages(collect_packages(root, manifest, patterns, pm)?)
}

pub(crate) struct Package {
    dir: PathBuf,
    rel_dir: String,
    manifest: PathBuf,
    value: Value,
}

fn collect_packages(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    // Yarn expands every member's own `workspaces` field in turn (its
    // worktrees), a declaration's negations reaching only its own directory.
    let mut queue = VecDeque::from([(
        root.to_path_buf(),
        manifest.to_path_buf(),
        patterns.to_vec(),
    )]);
    // Keyed by the file id, so that a member declaring a symlink to itself is
    // not requeued forever; the root goes in first, as a `..` pattern lists it
    // again. Only the queue is guarded, leaving the alias spelling to
    // `exclude_duplicate_names`.
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
                && rel_dir != "."
                && let Some(declared) = workspaces_patterns(&value, &path)
                && visited.insert(dir_id(&child_dir)?)
            {
                queue.push_back((child_dir.clone(), path.clone(), declared));
            }
            packages.push(Package {
                dir: child_dir,
                rel_dir,
                manifest: path,
                value,
            });
        }
    }
    Ok(packages)
}

fn enumerate(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
) -> Result<BTreeMap<String, PathBuf>> {
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
        candidates.insert(".".to_owned(), root.to_path_buf());
    }
    Ok(candidates)
}

fn qualify_packages(packages: Vec<Package>) -> Result<Vec<Member>> {
    let mut members: Vec<Member> = packages
        .into_iter()
        .filter_map(|package| {
            qualify(
                &package.value,
                package.dir,
                package.rel_dir,
                &package.manifest,
            )
        })
        .collect();
    members.sort_by(|a, b| (&a.name, &a.dir).cmp(&(&b.name, &b.dir)));
    exclude_duplicate_names(&mut members)?;
    Ok(members)
}

// A missing `name` or `version` key is only reported at debug level —
// fixture, private-root, and docs-site manifests omit them legitimately —
// while a key carrying an invalid value can only be a mistake and warns.
fn qualify(value: &Value, dir: PathBuf, rel_dir: String, path: &Path) -> Option<Member> {
    let Some(object) = value.as_object() else {
        warn!(
            "{}: not a workspace member: the manifest is not a JSON object",
            path.display()
        );
        return None;
    };
    let name = match object.get("name") {
        None => {
            debug!(
                "{}: not a workspace member: \"name\" is missing",
                path.display()
            );
            return None;
        }
        Some(Value::String(name)) if name.is_empty() => {
            warn!(
                "{}: not a workspace member: \"name\" is an empty string",
                path.display()
            );
            return None;
        }
        Some(Value::String(name)) => name.clone(),
        Some(_) => {
            warn!(
                "{}: not a workspace member: \"name\" is not a string",
                path.display()
            );
            return None;
        }
    };
    let version = match object.get("version") {
        None => {
            debug!(
                "{}: not a workspace member: \"version\" is missing",
                path.display()
            );
            return None;
        }
        Some(Value::String(version)) => {
            let Ok(version) = version.parse::<Version>() else {
                warn!(
                    "{}: not a workspace member: \"version\" {version:?} is not a valid semver",
                    path.display()
                );
                return None;
            };
            version
        }
        Some(_) => {
            warn!(
                "{}: not a workspace member: \"version\" is not a string",
                path.display()
            );
            return None;
        }
    };
    Some(Member {
        name,
        dir,
        rel_dir,
        version,
        private: object
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

// Qualification runs first, so a disqualified candidate sharing a real
// package's name does not evict it. Aliases of one physical directory are
// one package and collapse into the first. Expects `members` sorted by
// (name, dir) with a stable sort: aliases collapse into the smallest dir, and
// one directory spelled twice keeps its first listing.
fn exclude_duplicate_names(members: &mut Vec<Member>) -> Result<()> {
    let mut iter = std::mem::take(members).into_iter().peekable();
    while let Some(first) = iter.next() {
        if iter.peek().is_none_or(|next| next.name != first.name) {
            members.push(first);
            continue;
        }
        let mut group = vec![(dir_id(&first.dir)?, first)];
        while let Some(member) = iter.next_if(|next| next.name == group[0].1.name) {
            let id = dir_id(&member.dir)?;
            if !group.iter().any(|(kept, _)| *kept == id) {
                group.push((id, member));
            }
        }
        if group.len() > 1 {
            for (_, member) in group {
                warn!(
                    "{}: not a workspace member: the name `{}` is used by more than one package",
                    member.dir.join("package.json").display(),
                    member.name
                );
            }
        } else {
            members.extend(group.into_iter().map(|(_, member)| member));
        }
    }
    Ok(())
}

fn lists_dir(packages: &[Package], dir: &Path) -> Result<bool> {
    let id = dir_id(dir)?;
    for package in packages {
        if dir_id(&package.dir)? == id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dir_id(dir: &Path) -> Result<FileId> {
    get_file_id(dir).with_context(|| dir.display().to_string())
}

pub(crate) fn probe_is_file(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        Err(err) => {
            report_fs_error(path, &err);
            false
        }
    }
}

pub(crate) fn report_fs_error(path: &Path, err: &io::Error) {
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
