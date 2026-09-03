mod pattern;
mod walk;

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use same_file::is_same_file;
use saphyr::{LoadableYamlNode, Yaml};
use semver::Version;
use serde_json::Value;
use tracing::{debug, warn};

#[derive(Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
    members: Vec<Member>,
}

#[derive(Debug)]
pub(crate) struct Member {
    name: String,
    dir: PathBuf,
    rel_dir: String,
    version: Version,
    private: bool,
}

// The package manager whose workspace rules a root is enumerated with,
// chosen from the marker that made it a root; not the package manager the
// repository actually uses, as a `workspaces` field alone reads as npm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
}

impl PackageManager {
    fn name(self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Yarn => "yarn",
            PackageManager::Pnpm => "pnpm",
        }
    }

    fn excluded_names(self) -> &'static [&'static str] {
        match self {
            PackageManager::Npm => &["node_modules"],
            PackageManager::Yarn => &["node_modules", ".git", ".yarn"],
            PackageManager::Pnpm => &["node_modules", "bower_components"],
        }
    }

    fn root_always(self) -> bool {
        self != PackageManager::Npm
    }
}

pub(crate) enum RootMarker {
    PnpmWorkspaceYaml,
    YarnLock,
    NpmReroot(Vec<Package>),
    PackageJson,
}

pub(crate) fn find_root(cwd: &Path) -> Result<(PathBuf, RootMarker)> {
    for dir in cwd.ancestors() {
        if probe_is_file(&dir.join("pnpm-workspace.yaml")) {
            return Ok((dir.to_path_buf(), RootMarker::PnpmWorkspaceYaml));
        }
        if probe_is_file(&dir.join("yarn.lock")) {
            return Ok((dir.to_path_buf(), RootMarker::YarnLock));
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
        let pm = PackageManager::Npm;
        let Some(patterns) = workspaces_patterns(&value, &path, pm)? else {
            continue;
        };
        // The candidate prefix is looked for among every matched directory
        // holding a package.json, so the member qualification (and the
        // duplicate-name exclusion) must not run first.
        let packages = collect_packages(dir, &path, &patterns, pm)?;
        if packages
            .iter()
            .any(|package| is_same_file(&package.dir, prefix_dir).unwrap_or(false))
        {
            return Ok((dir.to_path_buf(), RootMarker::NpmReroot(packages)));
        }
    }

    let Some(dir) = prefix else {
        bail!(
            "no package.json found in {} or any parent directory",
            cwd.display()
        )
    };
    Ok((dir, RootMarker::PackageJson))
}

// The root is always the physical path: `find_root` and `Workspace::load`
// rely on it holding no `.` or `..` component, as they climb by `parent()`.
pub(crate) fn resolve_root(dir: &Path) -> Result<PathBuf> {
    let root = dunce::canonicalize(dir)?;
    if !fs::metadata(&root)?.is_dir() {
        bail!("not a directory")
    }
    Ok(root)
}

impl Workspace {
    pub(crate) fn load(
        root: &Path,
        rel_dirs: Option<&[String]>,
        marker: Option<RootMarker>,
    ) -> Result<Workspace> {
        if let Some(rel_dirs) = rel_dirs {
            let mut packages = Vec::new();
            for entry in rel_dirs {
                let (dir, rel_dir) = resolve_rel_dir(root, entry)?;
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
                root.to_path_buf(),
                "packages from config",
                qualify_packages(packages),
            ));
        }
        let marker = match marker {
            Some(marker) => marker,
            None if probe_is_file(&root.join("pnpm-workspace.yaml")) => {
                RootMarker::PnpmWorkspaceYaml
            }
            None if probe_is_file(&root.join("yarn.lock")) => RootMarker::YarnLock,
            None => RootMarker::PackageJson,
        };
        let path = root.join("package.json");
        match marker {
            RootMarker::NpmReroot(packages) => Ok(Workspace::new(
                root.to_path_buf(),
                PackageManager::Npm.name(),
                qualify_packages(packages),
            )),
            RootMarker::PnpmWorkspaceYaml => {
                let manifest = root.join("pnpm-workspace.yaml");
                let patterns = pnpm_patterns(&manifest)?;
                let pm = PackageManager::Pnpm;
                Ok(Workspace::new(
                    root.to_path_buf(),
                    pm.name(),
                    collect_members(root, &manifest, &patterns, pm)?,
                ))
            }
            RootMarker::YarnLock => {
                let pm = PackageManager::Yarn;
                let mut patterns = Vec::new();
                if probe_is_file(&path)
                    && let Some(value) = read_manifest(&path)?
                    && let Some(declared) = workspaces_patterns(&value, &path, pm)?
                {
                    patterns = declared;
                }
                Ok(Workspace::new(
                    root.to_path_buf(),
                    pm.name(),
                    collect_members(root, &path, &patterns, pm)?,
                ))
            }
            RootMarker::PackageJson => {
                let Some(value) = read_manifest(&path)? else {
                    bail!(
                        "no package.json in {}; --root must name a workspace root or a package",
                        root.display()
                    )
                };
                let pm = PackageManager::Npm;
                if let Some(patterns) = workspaces_patterns(&value, &path, pm)? {
                    return Ok(Workspace::new(
                        root.to_path_buf(),
                        pm.name(),
                        collect_members(root, &path, &patterns, pm)?,
                    ));
                }
                // A single package takes the same qualification as any member;
                // failing it leaves a workspace with zero members rather than an
                // error.
                let packages = vec![Package {
                    dir: root.to_path_buf(),
                    rel_dir: ".".to_owned(),
                    manifest: path,
                    value,
                }];
                Ok(Workspace::new(
                    root.to_path_buf(),
                    "single package",
                    qualify_packages(packages),
                ))
            }
        }
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

    pub(crate) fn changeset_dir(&self) -> PathBuf {
        self.root.join(".changeset")
    }

    pub(crate) fn members(&self) -> &[Member] {
        &self.members
    }

    pub(crate) fn member(&self, name: &str) -> Result<&Member> {
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
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn rel_dir(&self) -> &str {
        &self.rel_dir
    }

    pub(crate) fn version(&self) -> &Version {
        &self.version
    }

    pub(crate) fn private(&self) -> bool {
        self.private
    }
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

// A null `packages` is the plain YAML spelling of an empty list and reads as
// absent.
fn pnpm_patterns(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let docs = match Yaml::load_from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
        Ok(docs) => docs,
        Err(err) => bail!("{}: invalid YAML: {err}", path.display()),
    };
    // A missing or null document keeps an empty or comment-only file a
    // valid settings-only root.
    let Some(doc) = docs.into_iter().next() else {
        return Ok(Vec::new());
    };
    if doc.is_null() {
        return Ok(Vec::new());
    }
    let Yaml::Mapping(mapping) = doc else {
        bail!("{}: not a YAML mapping", path.display())
    };
    let packages = mapping
        .iter()
        .find_map(|(key, value)| (key.as_str() == Some("packages")).then_some(value));
    let Some(packages) = packages else {
        return Ok(Vec::new());
    };
    let items = match packages {
        Yaml::Sequence(items) => items,
        _ if packages.is_null() => return Ok(Vec::new()),
        _ => bail!("{}: \"packages\" must be a list of strings", path.display()),
    };
    let mut patterns = Vec::new();
    for item in items {
        let Some(pattern) = item.as_str() else {
            bail!("{}: \"packages\" must be a list of strings", path.display())
        };
        patterns.push(pattern.to_owned());
    }
    Ok(patterns)
}

fn workspaces_patterns(
    value: &Value,
    path: &Path,
    pm: PackageManager,
) -> Result<Option<Vec<String>>> {
    let Some(workspaces) = value.get("workspaces") else {
        return Ok(None);
    };
    let items = match workspaces {
        Value::Array(items) => Some(items),
        // The Yarn 1 object form `{packages: [...], nohoist: [...]}`; every
        // key but `packages` is ignored.
        Value::Object(object) => match object.get("packages") {
            Some(Value::Array(items)) => Some(items),
            _ => None,
        },
        Value::Null | Value::Bool(false) => return Ok(None),
        Value::Number(number) if number.as_f64() == Some(0.0) => return Ok(None),
        Value::String(text) if text.is_empty() => return Ok(None),
        _ => None,
    };
    let Some(items) = items else {
        let what = if workspaces.is_object() {
            "\"packages\" in \"workspaces\" must be a list of strings"
        } else {
            "\"workspaces\" must be an array or an object"
        };
        if pm == PackageManager::Yarn {
            warn!("{}: {what}: ignored, as Yarn ignores it", path.display());
            return Ok(None);
        }
        bail!("{}: {what}", path.display())
    };
    let mut patterns = Vec::new();
    for item in items {
        let Some(pattern) = item.as_str() else {
            if pm == PackageManager::Yarn {
                warn!(
                    "{}: a non-string \"workspaces\" pattern is skipped, as Yarn skips it",
                    path.display()
                );
                continue;
            }
            bail!(
                "{}: \"workspaces\" patterns must be strings",
                path.display()
            )
        };
        patterns.push(pattern.to_owned());
    }
    Ok(Some(patterns))
}

fn collect_members(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
) -> Result<Vec<Member>> {
    Ok(qualify_packages(collect_packages(
        root, manifest, patterns, pm,
    )?))
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
    // Keyed by the physical directory, so that a member declaring a symlink
    // to itself is not requeued forever; the root goes in first, as a `..`
    // pattern lists it again. Only the queue is guarded, leaving the alias
    // spelling to `exclude_duplicate_names`.
    let mut visited = HashSet::new();
    if pm == PackageManager::Yarn {
        visited.insert(physical_key(root));
    }
    while let Some((dir, manifest, patterns)) = queue.pop_front() {
        for child_dir in enumerate(&dir, &manifest, &patterns, pm)?.into_values() {
            let rel_dir = rel_dir_between(root, &child_dir);
            let path = child_dir.join("package.json");
            let Some(value) = read_manifest(&path)? else {
                continue;
            };
            if pm == PackageManager::Yarn
                && rel_dir != "."
                && let Some(declared) = workspaces_patterns(&value, &path, pm)?
                && visited.insert(physical_key(&child_dir))
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

fn physical_key(dir: &Path) -> PathBuf {
    match fs::canonicalize(dir) {
        Ok(key) => key,
        Err(err) => {
            report_fs_error(dir, &err);
            dir.to_path_buf()
        }
    }
}

fn qualify_packages(packages: Vec<Package>) -> Vec<Member> {
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
    exclude_duplicate_names(&mut members);
    members
}

fn enumerate(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pm: PackageManager,
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

    let reject_pnpm_manifests = pm == PackageManager::Pnpm;
    let mut candidates = walk::collect(
        root,
        &positives,
        &negations,
        pm.excluded_names(),
        reject_pnpm_manifests,
    )?;
    if pm.root_always() && walk::has_manifest(root, reject_pnpm_manifests)? {
        candidates.insert(".".to_owned(), root.to_path_buf());
    }
    Ok(candidates)
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
// one package and collapse into the first; an `is_same_file` error counts as
// a distinct directory. Expects `members` sorted by (name, dir) with a stable
// sort, so one directory spelled twice keeps its first listing.
fn exclude_duplicate_names(members: &mut Vec<Member>) {
    let mut iter = std::mem::take(members).into_iter().peekable();
    while let Some(first) = iter.next() {
        let mut group = vec![first];
        while iter.peek().is_some_and(|next| next.name == group[0].name) {
            let member = iter.next().unwrap();
            if !group
                .iter()
                .any(|kept| is_same_file(&kept.dir, &member.dir).unwrap_or(false))
            {
                group.push(member);
            }
        }
        if group.len() > 1 {
            for member in group {
                warn!(
                    "{}: not a workspace member: the name `{}` is used by more than one package",
                    member.dir.join("package.json").display(),
                    member.name
                );
            }
        } else {
            members.extend(group);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> PathBuf {
        Path::new("tests/fixtures/workspace").join(case)
    }

    fn discover(cwd: &Path) -> Result<Workspace> {
        let (root, marker) = find_root(cwd)?;
        Workspace::load(&root, None, Some(marker))
    }

    fn discover_ok(case: &str) -> Workspace {
        discover(&fixture(case)).unwrap()
    }

    fn discover_debug(case: &str) -> String {
        format!("{:#?}", discover_ok(case)).replace(r"\\", "/")
    }

    fn discover_err(case: &str) -> String {
        let text = format!("{:#}", discover(&fixture(case)).unwrap_err());
        match text.split_once(": ") {
            Some((path, rest)) => format!("{}: {rest}", path.replace('\\', "/")),
            None => text.replace('\\', "/"),
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn names_and_dirs(workspace: &Workspace) -> Vec<(&str, &Path)> {
        workspace
            .members
            .iter()
            .map(|member| (member.name.as_str(), member.dir.as_path()))
            .collect()
    }

    #[test]
    fn discovers_npm_workspace_members_sorted_by_name() {
        insta::assert_snapshot!(discover_debug("npm"));
    }

    #[test]
    fn discovers_pnpm_workspace_members_with_exclusions() {
        insta::assert_snapshot!(discover_debug("pnpm"));
    }

    #[test]
    fn walks_nested_directories_skipping_node_modules_and_dot_directories() {
        insta::assert_snapshot!(discover_debug("deep"));
    }

    #[test]
    fn accepts_the_yarn_1_workspaces_object() {
        let workspace = discover_ok("yarn1-object");
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", fixture("yarn1-object/packages/a").as_path())]
        );
    }

    #[test]
    fn rejects_a_non_list_pnpm_packages() {
        let err = discover_err("pnpm-bad-packages");
        assert!(
            err.contains("\"packages\" must be a list of strings"),
            "{err}"
        );
    }

    #[test]
    fn rejects_an_invalid_glob() {
        insta::assert_snapshot!(discover_err("bad-glob"));
    }

    #[test]
    fn excludes_duplicate_package_names() {
        let workspace = discover_ok("duplicate-names");
        assert_eq!(
            names_and_dirs(&workspace),
            [("unique", fixture("duplicate-names/packages/c").as_path())]
        );
        assert!(workspace.member("unique").is_ok());
        let err = format!("{:#}", workspace.member("dup").unwrap_err());
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn excludes_a_non_string_member_name() {
        assert_eq!(names_and_dirs(&discover_ok("bad-name")), []);
    }

    #[test]
    fn accepts_an_empty_workspaces_array() {
        assert_eq!(names_and_dirs(&discover_ok("npm-empty")), []);
    }

    #[test]
    fn an_empty_pnpm_packages_list_keeps_the_root_as_the_only_member() {
        let workspace = discover_ok("pnpm-empty");
        assert_eq!(
            names_and_dirs(&workspace),
            [("root", fixture("pnpm-empty").as_path())]
        );
    }

    #[test]
    fn includes_a_wildcard_matched_symlinked_member_directory() {
        let workspace = discover_ok("symlink");
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", fixture("symlink/packages/a").as_path()),
                ("pkg-b", fixture("symlink/packages/b").as_path()),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn follows_a_symlink_in_a_literal_pattern_prefix() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"link/*\"\n",
        );
        write(
            dir.path(),
            "real/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("link/a").as_path())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn collapses_candidates_aliasing_the_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"real/*\"\n  - \"link/*\"\n",
        );
        write(
            dir.path(),
            "real/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("link/a").as_path())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_true_duplicate_is_excluded_even_beside_an_aliased_pair() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"real/*\"\n  - \"link/*\"\n  - \"other/*\"\n",
        );
        write(
            dir.path(),
            "real/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "other/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn resolves_a_member_by_name() {
        let workspace = discover_ok("npm");
        let member = workspace.member("alpha").unwrap();
        assert_eq!(member.dir(), fixture("npm/packages/two"));
    }

    #[test]
    fn rejects_an_unknown_member_name() {
        let workspace = discover_ok("npm");
        insta::assert_snapshot!(format!("{:#}", workspace.member("missing").unwrap_err()));
    }

    #[test]
    fn rejects_any_name_in_an_empty_workspace() {
        let workspace = discover_ok("npm-empty");
        insta::assert_snapshot!(format!("{:#}", workspace.member("missing").unwrap_err()));
    }

    #[test]
    fn a_settings_only_pnpm_manifest_wins_over_npm_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "onlyBuiltDependencies: []\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn pnpm_manifest_with_null_packages_is_a_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "packages:\n");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn an_empty_pnpm_manifest_is_a_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn rejects_a_non_mapping_pnpm_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "- packages/*\n");
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(err.contains("not a YAML mapping"), "{err}");
    }

    #[test]
    fn accepts_a_bom_prefixed_pnpm_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "\u{feff}packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn an_inner_settings_only_pnpm_manifest_shadows_an_outer_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/inner/package.json",
            "{ \"name\": \"inner\", \"version\": \"0.1.0\" }\n",
        );
        write(
            dir.path(),
            "packages/inner/pnpm-workspace.yaml",
            "onlyBuiltDependencies:\n  - esbuild\n",
        );
        let inner = dir.path().join("packages/inner");
        let workspace = discover(&inner).unwrap();
        assert_eq!(workspace.root, inner);
        assert_eq!(names_and_dirs(&workspace), [("inner", inner.as_path())]);
    }

    #[test]
    fn resolves_the_workspace_root_from_a_member_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("pkg-b", dir.path().join("packages/b").as_path()),
            ]
        );
    }

    #[test]
    fn falls_back_to_the_nearest_package_json() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\" }\n",
        );
        fs::create_dir_all(dir.path().join("app/src")).unwrap();
        let workspace = discover(&dir.path().join("app/src")).unwrap();
        assert_eq!(workspace.root, dir.path().join("app"));
        assert_eq!(
            names_and_dirs(&workspace),
            [("app", dir.path().join("app").as_path())]
        );
    }

    #[test]
    fn errors_without_any_package_json() {
        let dir = tempfile::tempdir().unwrap();
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(err.contains("no package.json found"), "{err}");
    }

    #[test]
    fn a_nameless_fallback_package_yields_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"version\": \"1.0.0\" }\n");
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn accepts_a_leading_dot_slash_in_patterns() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"./packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn accepts_a_trailing_slash_in_patterns() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*/\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn double_star_matches_zero_and_more_components() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/**\"\n",
        );
        write(
            dir.path(),
            "packages/package.json",
            "{ \"name\": \"direct\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/nested/deep/package.json",
            "{ \"name\": \"deep\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/node_modules/evil/package.json",
            "{ \"name\": \"evil\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/bower_components/old/package.json",
            "{ \"name\": \"old\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("deep", dir.path().join("packages/nested/deep").as_path()),
                ("direct", dir.path().join("packages").as_path()),
            ]
        );
    }

    #[test]
    fn the_excluded_directory_names_follow_the_package_manager() {
        let literals = "\"bower_components/a\", \".yarn/b\", \".git/c\"";
        let yaml = format!("packages: [{literals}]\n");
        let json = format!("{{ \"workspaces\": [{literals}] }}\n");
        for (files, expected) in [
            (vec![("pnpm-workspace.yaml", &yaml)], vec!["pkg-b", "pkg-c"]),
            (
                vec![("package.json", &json)],
                vec!["pkg-a", "pkg-b", "pkg-c"],
            ),
            (
                vec![("yarn.lock", &String::new()), ("package.json", &json)],
                vec!["pkg-a"],
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            for (rel, text) in &files {
                write(dir.path(), rel, text);
            }
            for (rel, name) in [
                ("bower_components/a", "pkg-a"),
                (".yarn/b", "pkg-b"),
                (".git/c", "pkg-c"),
            ] {
                write(
                    dir.path(),
                    &format!("{rel}/package.json"),
                    &format!("{{ \"name\": \"{name}\", \"version\": \"1.0.0\" }}\n"),
                );
            }
            let workspace = discover(dir.path()).unwrap();
            let names: Vec<_> = workspace.members().iter().map(Member::name).collect();
            assert_eq!(names, expected, "{files:?}");
        }
    }

    #[test]
    fn a_dot_pattern_makes_the_root_a_member_in_pnpm() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "packages:\n  - \".\"\n");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn a_dot_pattern_makes_the_root_a_member_in_npm() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\".\"] }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn includes_the_pnpm_root_package_without_a_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn a_negation_cannot_exclude_the_pnpm_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - \"!.\"\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn the_pnpm_root_without_a_manifest_is_not_a_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn excludes_a_nameless_pnpm_root_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \".\"\n  - \"packages/*\"\n",
        );
        write(dir.path(), "package.json", "{ \"version\": \"1.0.0\" }\n");
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn does_not_include_the_npm_root_package_without_a_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn workspaces_without_a_lockfile_is_a_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn an_outer_pnpm_manifest_wins_over_an_inner_workspaces_field() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a/nested/x")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn an_outer_yarn_lock_wins_over_an_inner_workspaces_field() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a/nested/x")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [
                ("pkg-a", "packages/a"),
                ("pkg-x", "packages/a/nested/x"),
                ("root", ".")
            ]
        );
    }

    #[test]
    fn a_yarn_1_nohoist_leftover_in_a_pnpm_member_is_never_read_as_a_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": { \"nohoist\": [\"**/foo\"] } }\n",
        );
        let workspace = discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn a_member_of_a_nested_npm_workspace_re_roots_only_to_the_inner_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"outer\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let inner = dir.path().join("packages/a");
        let workspace = discover(&inner.join("nested/x")).unwrap();
        assert_eq!(workspace.root, inner);
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-x", inner.join("nested/x").as_path())]
        );
    }

    #[test]
    fn re_roots_to_the_npm_root_listing_the_nearest_package_as_a_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        fs::create_dir_all(dir.path().join("packages/a/src")).unwrap();
        let workspace = discover(&dir.path().join("packages/a/src")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("pkg-b", dir.path().join("packages/b").as_path()),
            ]
        );
    }

    #[test]
    fn re_roots_from_a_versionless_member_listed_by_the_npm_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"apps/*\", \"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "apps/web/package.json",
            "{ \"name\": \"web\", \"private\": true }\n",
        );
        write(
            dir.path(),
            "packages/lib/package.json",
            "{ \"name\": \"lib\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("apps/web")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("lib", dir.path().join("packages/lib").as_path())]
        );
    }

    #[test]
    fn re_roots_from_a_member_whose_name_is_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\", \"fixtures/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "fixtures/a/package.json",
            "{ \"name\": \"dup\", \"version\": \"0.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("packages/b").as_path())]
        );
    }

    #[test]
    fn a_package_not_listed_by_the_npm_root_above_is_a_single_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "examples/x/package.json",
            "{ \"name\": \"example-x\", \"version\": \"1.0.0\" }\n",
        );
        let example = dir.path().join("examples/x");
        let workspace = discover(&example).unwrap();
        assert_eq!(workspace.root, example);
        assert_eq!(
            names_and_dirs(&workspace),
            [("example-x", example.as_path())]
        );
    }

    #[test]
    fn a_stray_manifest_below_an_npm_member_is_a_single_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/src/package.json",
            "{ \"name\": \"stray\", \"version\": \"1.0.0\" }\n",
        );
        let stray = dir.path().join("packages/a/src");
        let workspace = discover(&stray).unwrap();
        assert_eq!(workspace.root, stray);
        assert_eq!(names_and_dirs(&workspace), [("stray", stray.as_path())]);
    }

    #[test]
    fn a_member_with_a_leftover_workspaces_field_re_roots_to_the_npm_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn an_npm_workspace_between_a_member_and_its_root_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"a/b\"] }\n",
        );
        write(
            dir.path(),
            "a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"c\"] }\n",
        );
        write(
            dir.path(),
            "a/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "a/c/package.json",
            "{ \"name\": \"pkg-c\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("a/b")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("a/b").as_path())]
        );
    }

    #[test]
    fn the_yarn_1_object_form_is_a_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": { \"packages\": [\"packages/*\"] } }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn reads_private_leniently_and_excludes_invalid_versions() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"private\": true }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": 1, \"private\": \"true\" }\n",
        );
        write(
            dir.path(),
            "packages/c/package.json",
            "{ \"name\": \"pkg-c\", \"version\": \"\" }\n",
        );
        write(
            dir.path(),
            "packages/d/package.json",
            "{ \"name\": \"pkg-d\", \"version\": \"2.0.0\", \"private\": \"yes\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        let members = workspace.members();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("pkg-d", dir.path().join("packages/d").as_path()),
            ]
        );
        assert_eq!(members[0].version(), &Version::new(1, 0, 0));
        assert!(members[0].private());
        assert!(!members[1].private());
    }

    #[test]
    fn excludes_a_non_semver_version() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn matches_a_literal_dot_directory_component() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \".tools/*\"\n",
        );
        write(
            dir.path(),
            ".tools/a/package.json",
            "{ \"name\": \"tool-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("tool-a", dir.path().join(".tools/a").as_path())]
        );
    }

    #[test]
    fn does_not_match_dot_directories_with_a_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "packages:\n  - \"*\"\n");
        write(
            dir.path(),
            ".tools/package.json",
            "{ \"name\": \"tool\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("a").as_path())]
        );
    }

    #[test]
    fn expands_brace_alternatives() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/{a,b/c}\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/c/package.json",
            "{ \"name\": \"pkg-bc\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/{a,b/c}/package.json",
            "{ \"name\": \"pkg-braced\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("pkg-bc", dir.path().join("packages/b/c").as_path())
            ]
        );
    }

    #[test]
    fn a_leading_parent_pattern_reaches_outside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"app\"\n  - \"../shared\"\n",
        );
        write(
            dir.path(),
            "ws/package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "ws/app/package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "shared/package.json",
            "{ \"name\": \"shared\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("ws")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("app", "app"), ("root", "."), ("shared", "../shared")]
        );
        assert_eq!(
            workspace.member("shared").unwrap().dir(),
            dir.path().join("shared")
        );
    }

    #[test]
    fn a_parent_run_climbing_past_the_filesystem_root_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            &format!("packages:\n  - \"{}*\"\n", "../".repeat(64)),
        );
        write(
            dir.path(),
            "ws/package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("ws")).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("root", ".")]);
    }

    #[test]
    fn the_root_listed_by_a_parent_pattern_keeps_its_dot_spelling() {
        for pm in [
            PackageManager::Pnpm,
            PackageManager::Yarn,
            PackageManager::Npm,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root_manifest = match pm {
                PackageManager::Pnpm => {
                    write(
                        dir.path(),
                        "ws/pnpm-workspace.yaml",
                        "packages:\n  - \"../*\"\n",
                    );
                    "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n"
                }
                PackageManager::Yarn => {
                    write(dir.path(), "ws/yarn.lock", "");
                    "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"../*\"] }\n"
                }
                PackageManager::Npm => {
                    "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"../*\", \".\"] }\n"
                }
            };
            write(dir.path(), "ws/package.json", root_manifest);
            write(
                dir.path(),
                "sib/package.json",
                "{ \"name\": \"sib\", \"version\": \"1.0.0\" }\n",
            );
            let workspace = discover(&dir.path().join("ws")).unwrap();
            assert_eq!(
                names_and_rel_dirs(&workspace),
                [("root", "."), ("sib", "../sib")],
                "{}",
                pm.name()
            );
        }
    }

    #[test]
    fn a_parent_detour_to_a_member_collapses_into_the_direct_spelling() {
        for patterns in [["../*/*", "*"], ["*", "../*/*"]] {
            for yarn in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                if yarn {
                    write(dir.path(), "ws/yarn.lock", "");
                    write(
                        dir.path(),
                        "ws/package.json",
                        &format!(
                            "{{ \"workspaces\": [\"{}\", \"{}\"] }}\n",
                            patterns[0], patterns[1]
                        ),
                    );
                } else {
                    write(
                        dir.path(),
                        "ws/pnpm-workspace.yaml",
                        &format!(
                            "packages:\n  - \"{}\"\n  - \"{}\"\n",
                            patterns[0], patterns[1]
                        ),
                    );
                }
                write(
                    dir.path(),
                    "ws/a/package.json",
                    "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
                );
                let workspace = discover(&dir.path().join("ws")).unwrap();
                assert_eq!(
                    names_and_rel_dirs(&workspace),
                    [("pkg-a", "a")],
                    "{patterns:?} yarn={yarn}"
                );
            }
        }
    }

    #[test]
    fn a_parent_negation_excludes_a_parent_candidate() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["o", "p"] {
            write(
                dir.path(),
                &format!("ext/{name}/package.json"),
                &format!("{{ \"name\": \"{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"../ext/*\"\n  - \"!../ext/o\"\n",
        );
        let workspace = discover(&dir.path().join("ws")).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("p", "../ext/p")]);
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"../ext/*\"\n  - \"!**/o\"\n",
        );
        let workspace = discover(&dir.path().join("ws")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("o", "../ext/o"), ("p", "../ext/p")]
        );
    }

    #[test]
    fn a_bare_parent_pattern_makes_the_parent_a_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "parent/package.json",
            "{ \"name\": \"parent\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "parent/ws/pnpm-workspace.yaml",
            "packages:\n  - \"..\"\n",
        );
        let workspace = discover(&dir.path().join("parent/ws")).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("parent", "..")]);
    }

    #[test]
    fn mixes_patterns_of_different_ascents() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "g/p/ws/pnpm-workspace.yaml",
            "packages:\n  - \"a\"\n  - \"../x\"\n  - \"../../y\"\n",
        );
        for (rel, name) in [("g/p/ws/a", "a"), ("g/p/x", "x"), ("g/y", "y")] {
            write(
                dir.path(),
                &format!("{rel}/package.json"),
                &format!("{{ \"name\": \"{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        let workspace = discover(&dir.path().join("g/p/ws")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("a", "a"), ("x", "../x"), ("y", "../../y")]
        );
    }

    #[test]
    fn rejects_a_parent_segment_after_a_name() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/../other\"\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("invalid workspace pattern \"packages/../other\""),
            "{err}"
        );
        assert!(err.contains("only supported at the start"), "{err}");
    }

    #[test]
    fn re_roots_to_an_npm_root_with_a_parent_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "root/package.json",
            "{ \"workspaces\": [\"../ext/*\", \"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "root/packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "ext/o/package.json",
            "{ \"name\": \"pkg-o\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("root/packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path().join("root"));
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("pkg-o", "../ext/o")]
        );
    }

    #[test]
    fn rejects_a_pnpm_only_manifest_outside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"../shared\"\n",
        );
        write(
            dir.path(),
            "shared/package.yaml",
            "name: shared\nversion: 1.0.0\n",
        );
        let err = format!("{:#}", discover(&dir.path().join("ws")).unwrap_err());
        let manifest = dir.path().join("shared").join("package.yaml");
        assert!(
            err.contains(&format!(
                "{}: only package.json manifests are supported",
                manifest.display()
            )),
            "{err}"
        );
    }

    #[test]
    fn tolerates_a_pattern_whose_base_directory_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - \"apps/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn tolerates_a_pattern_whose_path_crosses_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"docs/pkg\"\n  - \"packages/*\"\n",
        );
        write(dir.path(), "docs", "not a directory\n");
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignores_a_permission_error_inside_a_pattern() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/**\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        fs::create_dir_all(dir.path().join("packages/denied")).unwrap();
        fs::set_permissions(
            dir.path().join("packages/denied"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let result = discover(dir.path());
        fs::set_permissions(
            dir.path().join("packages/denied"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(
            names_and_dirs(&result.unwrap()),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignores_junk_outside_the_patterns() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        fs::create_dir_all(dir.path().join("junk/denied")).unwrap();
        std::os::unix::fs::symlink("loop", dir.path().join("junk/loop")).unwrap();
        fs::set_permissions(
            dir.path().join("junk/denied"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let workspace = discover(dir.path()).unwrap();
        fs::set_permissions(
            dir.path().join("junk/denied"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn excludes_with_a_double_star_negative_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/**\"\n  - \"!packages/**/fixtures/**\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/fixtures/x/package.json",
            "{ \"name\": \"fx-x\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/fixtures/y/package.json",
            "{ \"name\": \"fx-y\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn negation_is_order_independent() {
        let yaml_head = "packages:\n  - \"!packages/b\"\n  - \"packages/a\"\n  - \"packages/b\"\n";
        let yaml_tail = "packages:\n  - \"packages/a\"\n  - \"packages/b\"\n  - \"!packages/b\"\n";
        let json_head = "{ \"workspaces\": [\"!packages/b\", \"packages/a\", \"packages/b\"] }\n";
        let json_tail = "{ \"workspaces\": [\"packages/a\", \"packages/b\", \"!packages/b\"] }\n";
        for (marker, text, lock) in [
            ("pnpm-workspace.yaml", yaml_head, false),
            ("pnpm-workspace.yaml", yaml_tail, false),
            ("package.json", json_head, true),
            ("package.json", json_tail, true),
            ("package.json", json_head, false),
            ("package.json", json_tail, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), marker, text);
            if lock {
                write(dir.path(), "yarn.lock", "");
            }
            for name in ["a", "b"] {
                write(
                    dir.path(),
                    &format!("packages/{name}/package.json"),
                    &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
                );
            }
            let workspace = discover(dir.path()).unwrap();
            assert_eq!(
                names_and_dirs(&workspace),
                [("pkg-a", dir.path().join("packages/a").as_path())],
                "{marker} (yarn.lock: {lock}): {text}"
            );
        }
    }

    #[test]
    fn rejects_an_absolute_negative_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - \"!/packages/a\"\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("invalid workspace pattern \"!/packages/a\""),
            "{err}"
        );
    }

    #[test]
    fn rejects_an_absolute_positive_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"/packages/*\"\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("invalid workspace pattern \"/packages/*\""),
            "{err}"
        );
    }

    #[test]
    fn excludes_a_versionless_member_in_an_npm_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn excludes_a_versionless_member_in_a_pnpm_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn a_versionless_single_package_yields_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"name\": \"app\" }\n");
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn excludes_a_non_object_member_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(dir.path(), "packages/a/package.json", "[1, 2]\n");
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("packages/b").as_path())]
        );
    }

    #[test]
    fn excludes_a_nameless_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("packages/b").as_path())]
        );
    }

    #[test]
    fn a_disqualified_duplicate_does_not_evict_the_real_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"dup\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("dup", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn prefers_pnpm_when_both_markers_share_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"apps/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "apps/b/package.json",
            "{ \"name\": \"app-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn passes_over_a_falsy_workspaces() {
        for workspaces in ["null", "false", "0", "0.0", "\"\""] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                &format!(
                    "{{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
                ),
            );
            let workspace = discover(dir.path()).unwrap();
            assert_eq!(
                names_and_dirs(&workspace),
                [("app", dir.path())],
                "{workspaces}"
            );
        }
    }

    #[test]
    fn rejects_an_invalid_workspaces_type() {
        for workspaces in ["\"packages/*\"", "42", "true", "0.5"] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                &format!(
                    "{{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
                ),
            );
            let err = format!("{:#}", discover(dir.path()).unwrap_err());
            assert!(
                err.contains("\"workspaces\" must be an array or an object"),
                "{workspaces}: {err}"
            );
        }
    }

    #[test]
    fn rejects_an_invalid_workspaces_type_during_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"workspaces\": 42 }\n");
        write(
            dir.path(),
            "pkg/package.json",
            "{ \"name\": \"leaf\", \"version\": \"1.0.0\" }\n",
        );
        let err = format!("{:#}", discover(&dir.path().join("pkg")).unwrap_err());
        assert!(
            err.contains("\"workspaces\" must be an array or an object"),
            "{err}"
        );
    }

    #[test]
    fn rejects_a_workspaces_object_without_a_packages_list() {
        for workspaces in [
            "{}",
            "{ \"packages\": null }",
            "{ \"packages\": \"packages/*\" }",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                &format!(
                    "{{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
                ),
            );
            let err = format!("{:#}", discover(dir.path()).unwrap_err());
            assert!(
                err.contains("\"packages\" in \"workspaces\" must be a list of strings"),
                "{workspaces}: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_string_pattern_entries() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - 42\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("\"packages\" must be a list of strings"),
            "{err}"
        );

        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [42, \"packages/*\"] }\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("\"workspaces\" patterns must be strings"),
            "{err}"
        );
    }

    #[test]
    fn errors_on_a_broken_manifest_during_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"app\"] }\n",
        );
        write(
            dir.path(),
            "app/package.json",
            "{ \"name\": \"app\",\n<<<<<<< HEAD\n  \"version\": \"1.0.0\"\n}\n",
        );
        let err = format!("{:#}", discover(&dir.path().join("app")).unwrap_err());
        assert!(
            err.contains(
                &dir.path()
                    .join("app")
                    .join("package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }

    #[test]
    fn passes_over_a_broken_ancestor_manifest_in_npm_mode() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\",\n<<<<<<< HEAD\n  \"workspaces\": [\"app\"]\n}\n",
        );
        write(
            dir.path(),
            "app/package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("app")).unwrap();
        assert_eq!(workspace.root, dir.path().join("app"));
        assert_eq!(
            names_and_dirs(&workspace),
            [("app", dir.path().join("app").as_path())]
        );
    }

    #[test]
    fn a_broken_manifest_between_a_member_and_the_npm_root_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"a/b\"] }\n",
        );
        write(dir.path(), "a/package.json", "{ broken");
        write(
            dir.path(),
            "a/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("a/b")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("a/b").as_path())]
        );
    }

    #[test]
    fn a_broken_nearest_manifest_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a/package.json", "{ broken");
        let err = format!("{:#}", discover(&dir.path().join("a")).unwrap_err());
        let manifest = dir.path().join("a").join("package.json");
        assert!(err.starts_with(&manifest.display().to_string()), "{err}");
    }

    #[test]
    fn a_broken_manifest_below_a_pnpm_root_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"a/b\"\n",
        );
        write(dir.path(), "a/package.json", "{ broken");
        write(
            dir.path(),
            "a/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("a/b")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("a/b").as_path())]
        );
    }

    #[test]
    fn a_type_invalid_stray_manifest_is_a_memberless_single_package() {
        for stray in ["[1, 2]", "\"hello\""] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [] }\n",
            );
            write(dir.path(), "sub/package.json", stray);
            let workspace = discover(&dir.path().join("sub")).unwrap();
            assert_eq!(workspace.root, dir.path().join("sub"), "{stray}");
            assert_eq!(names_and_dirs(&workspace), [], "{stray}");
        }
    }

    #[test]
    fn passes_over_a_directory_named_package_json() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [] }\n",
        );
        fs::create_dir_all(dir.path().join("sub/package.json")).unwrap();
        let workspace = discover(&dir.path().join("sub")).unwrap();
        assert_eq!(workspace.root, dir.path());
    }

    #[test]
    fn parses_a_bom_member_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "\u{feff}{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn reads_workspaces_through_a_bom_during_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "\u{feff}{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn errors_on_a_broken_member_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(dir.path(), "packages/a/package.json", "{ broken");
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains(
                &dir.path()
                    .join("packages")
                    .join("a")
                    .join("package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }

    fn names_and_rel_dirs(workspace: &Workspace) -> Vec<(&str, &str)> {
        workspace
            .members
            .iter()
            .map(|member| (member.name.as_str(), member.rel_dir.as_str()))
            .collect()
    }

    #[test]
    fn a_yarn_lock_alone_is_a_workspace_root_without_members() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn a_yarn_lock_beside_a_manifest_makes_the_root_the_only_member() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root, dir.path());
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn a_yarn_lock_beside_workspaces_is_a_yarn_root() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn a_yarn_root_reads_the_workspaces_object_form() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": { \"packages\": [\"packages/*\"] } }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn a_yarn_root_ignores_an_invalid_workspaces_type() {
        for workspaces in [
            "\"packages/*\"",
            "42",
            "true",
            "{}",
            "{ \"packages\": \"packages/*\" }",
            "{ \"nohoist\": [\"**/foo\"] }",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), "yarn.lock", "");
            write(
                dir.path(),
                "package.json",
                &format!(
                    "{{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
                ),
            );
            write(
                dir.path(),
                "packages/a/package.json",
                "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
            );
            let workspace = discover(dir.path()).unwrap();
            assert_eq!(workspace.root, dir.path(), "{workspaces}");
            assert_eq!(
                names_and_dirs(&workspace),
                [("root", dir.path())],
                "{workspaces}"
            );
        }
    }

    #[test]
    fn a_yarn_root_skips_a_non_string_workspaces_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\", 42, null] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn a_yarn_member_with_an_invalid_workspaces_type_declares_no_worktree() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": \"nested/*\" }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn a_pnpm_manifest_wins_over_a_yarn_lock_in_the_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"apps/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "apps/b/package.json",
            "{ \"name\": \"app-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn the_nearest_yarn_lock_wins() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\", \"examples/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(dir.path(), "examples/e/yarn.lock", "");
        write(
            dir.path(),
            "examples/e/package.json",
            "{ \"name\": \"example\", \"version\": \"1.0.0\" }\n",
        );
        let inner = dir.path().join("examples/e");
        let workspace = discover(&inner.join("src")).unwrap();
        assert_eq!(workspace.root, inner);
        assert_eq!(names_and_dirs(&workspace), [("example", inner.as_path())]);
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("example", inner.as_path()),
                ("pkg-a", dir.path().join("packages/a").as_path()),
            ]
        );
    }

    #[test]
    fn a_yarn_member_expands_its_own_workspaces_field() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/x\", \"nested/y\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\", \"workspaces\": [\"deep/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/deep/z/package.json",
            "{ \"name\": \"pkg-z\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/y/package.json",
            "{ \"name\": \"pkg-y\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("packages/b")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [
                ("pkg-a", "packages/a"),
                ("pkg-b", "packages/b"),
                ("pkg-x", "packages/a/nested/x"),
                ("pkg-y", "packages/a/nested/y"),
                ("pkg-z", "packages/a/nested/x/deep/z"),
                ("root", "."),
            ]
        );
        assert_eq!(
            workspace.member("pkg-z").unwrap().dir(),
            dir.path().join("packages/a/nested/x/deep/z")
        );
    }

    #[test]
    fn a_yarn_member_reads_the_object_form_and_a_null_field() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": { \"packages\": [\"packages/*\"] } }\n",
        );
        write(
            dir.path(),
            "packages/desktop/package.json",
            "{ \"name\": \"desktop\", \"version\": \"1.0.0\", \"workspaces\": { \"packages\": [\"app\"] } }\n",
        );
        write(
            dir.path(),
            "packages/desktop/app/package.json",
            "{ \"name\": \"desktop-app\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/mobile/package.json",
            "{ \"name\": \"mobile\", \"version\": \"1.0.0\", \"workspaces\": null }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [
                ("desktop", "packages/desktop"),
                ("desktop-app", "packages/desktop/app"),
                ("mobile", "packages/mobile"),
            ]
        );
    }

    #[test]
    fn a_root_negation_does_not_reach_a_yarn_member_declaration() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\", \"!packages/a/nested/x\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\", \"!nested/y\"] }\n",
        );
        for name in ["x", "y"] {
            write(
                dir.path(),
                &format!("packages/a/nested/{name}/package.json"),
                &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("pkg-x", "packages/a/nested/x")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_yarn_member_symlinked_to_itself_is_expanded_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\", \"workspaces\": [\"loop\", \"*\"] }\n",
        );
        std::os::unix::fs::symlink(".", dir.path().join("packages/x/loop")).unwrap();
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-x", "packages/x")]);
    }

    #[test]
    fn a_yarn_member_declaration_reaches_outside_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "root/yarn.lock", "");
        write(
            dir.path(),
            "root/package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "root/packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../b\", \"../../../ext/o\"] }\n",
        );
        write(
            dir.path(),
            "root/packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "ext/o/package.json",
            "{ \"name\": \"pkg-o\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("root")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [
                ("pkg-a", "packages/a"),
                ("pkg-b", "packages/b"),
                ("pkg-o", "../ext/o"),
            ]
        );
        write(
            dir.path(),
            "root/package.json",
            "{ \"workspaces\": [\"packages/a\"] }\n",
        );
        let workspace = discover(&dir.path().join("root")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [
                ("pkg-a", "packages/a"),
                ("pkg-b", "packages/b"),
                ("pkg-o", "../ext/o"),
            ]
        );
        assert_eq!(
            workspace.member("pkg-b").unwrap().dir(),
            dir.path().join("root/packages/b")
        );
        assert_eq!(
            workspace.member("pkg-o").unwrap().dir(),
            dir.path().join("ext/o")
        );
    }

    #[test]
    fn a_yarn_member_declaration_climbing_back_into_the_root_is_spelled_from_the_root() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "root/yarn.lock", "");
        write(
            dir.path(),
            "root/package.json",
            "{ \"workspaces\": [\"packages/a\"] }\n",
        );
        write(
            dir.path(),
            "root/packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../../other/c\"] }\n",
        );
        write(
            dir.path(),
            "root/other/c/package.json",
            "{ \"name\": \"pkg-c\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(&dir.path().join("root")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("pkg-c", "other/c")]
        );
        assert_eq!(
            workspace.member("pkg-c").unwrap().dir(),
            dir.path().join("root/other/c")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_yarn_member_declaration_climbs_the_lexical_parent() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "root/yarn.lock", "");
        write(
            dir.path(),
            "root/package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "real/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../b\"] }\n",
        );
        write(
            dir.path(),
            "real/b/package.json",
            "{ \"name\": \"physical-b\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "root/packages/b/package.json",
            "{ \"name\": \"lexical-b\", \"version\": \"1.0.0\" }\n",
        );
        std::os::unix::fs::symlink("../../real/a", dir.path().join("root/packages/link")).unwrap();
        let workspace = discover(&dir.path().join("root")).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("lexical-b", "packages/b"), ("pkg-a", "packages/link")]
        );
    }

    #[test]
    fn a_yarn_member_declaration_takes_the_pattern_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"x/../b\"] }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let err = format!("{:#}", discover(dir.path()).unwrap_err());
        assert!(
            err.contains("invalid workspace pattern \"x/../b\""),
            "{err}"
        );
        assert!(
            err.contains(
                &dir.path()
                    .join("packages")
                    .join("a")
                    .join("package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }

    #[test]
    fn an_npm_member_does_not_expand_its_workspaces_field() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/nested/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    }

    #[test]
    fn rejects_a_pnpm_only_manifest() {
        for rel in ["packages/a/package.yaml", "package.json5"] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "pnpm-workspace.yaml",
                "packages:\n  - \"packages/*\"\n",
            );
            write(dir.path(), rel, "name: pkg\nversion: 1.0.0\n");
            let err = format!("{:#}", discover(dir.path()).unwrap_err());
            let mut manifest = dir.path().to_path_buf();
            manifest.extend(rel.split('/'));
            assert!(
                err.contains(&format!(
                    "{}: only package.json manifests are supported",
                    manifest.display()
                )),
                "{rel}: {err}"
            );
        }
    }

    #[test]
    fn a_package_json_beside_a_pnpm_only_manifest_is_read_as_usual() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(dir.path(), "package.yaml", "name: root\n");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json5",
            "{ name: 'pkg-a' }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("root", ".")]
        );
    }

    #[test]
    fn a_pnpm_only_manifest_is_ignored_outside_pnpm() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write(dir.path(), "packages/a/package.yaml", "name: pkg-a\n");
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-b", "packages/b")]);
    }

    #[test]
    fn an_empty_pattern_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"\"\n  - \"!\"\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    }

    #[test]
    fn a_slash_inside_a_character_class_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/[a/b]\"\n",
        );
        for name in ["a", "b", "c"] {
            write(
                dir.path(),
                &format!("packages/{name}/package.json"),
                &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        let workspace = discover(dir.path()).unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("pkg-b", "packages/b")]
        );
    }

    fn load_listed(root: &Path, packages: &[&str]) -> Result<Workspace> {
        let packages: Vec<String> = packages.iter().map(|dir| (*dir).to_owned()).collect();
        Workspace::load(root, Some(&packages), None)
    }

    fn load_listed_err(root: &Path, packages: &[&str]) -> String {
        format!("{:#}", load_listed(root, packages).unwrap_err())
    }

    #[test]
    fn a_forced_root_below_a_pnpm_root_is_a_single_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let root = dir.path().join("packages/a");
        let workspace = Workspace::load(&root, None, None).unwrap();
        assert_eq!(workspace.root, root);
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", ".")]);
    }

    #[test]
    fn a_forced_root_reads_only_its_own_markers() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "packages/inner/package.json",
            "{ \"name\": \"inner\", \"version\": \"1.0.0\", \"workspaces\": [\"libs/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/inner/libs/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let root = dir.path().join("packages/inner");
        let workspace = Workspace::load(&root, None, None).unwrap();
        assert_eq!(workspace.root, root);
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-x", "libs/x")]);
    }

    #[test]
    fn a_forced_root_without_a_manifest_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = format!("{:#}", Workspace::load(dir.path(), None, None).unwrap_err());
        assert!(err.contains("no package.json in"), "{err}");
        assert!(err.contains("--root"), "{err}");
    }

    #[test]
    fn a_forced_root_with_a_yarn_lock_alone_has_no_members() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "yarn.lock", "");
        let workspace = Workspace::load(dir.path(), None, None).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn listed_packages_replace_the_enumeration() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
        );
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "other/c/package.json",
            "{ \"name\": \"pkg-c\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = load_listed(dir.path(), &["other/c", "."]).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-c", dir.path().join("other/c").as_path()),
                ("root", dir.path()),
            ]
        );
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-c", "other/c"), ("root", ".")]
        );
        let workspace = load_listed(dir.path(), &[]).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn listed_packages_are_read_without_the_markers() {
        for case in ["bad-glob", "pnpm-bad-packages"] {
            let dir = tempfile::tempdir().unwrap();
            for entry in fs::read_dir(fixture(case)).unwrap() {
                let entry = entry.unwrap();
                fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
            }
            write(
                dir.path(),
                "packages/a/package.json",
                "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
            );
            assert!(discover(dir.path()).is_err(), "{case}");
            let workspace = load_listed(dir.path(), &["packages/a"]).unwrap();
            assert_eq!(
                names_and_dirs(&workspace),
                [("pkg-a", dir.path().join("packages/a").as_path())],
                "{case}"
            );
        }
    }

    #[test]
    fn listed_packages_do_not_need_a_root_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = load_listed(dir.path(), &["packages/a"]).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    }

    #[test]
    fn a_listed_package_without_a_manifest_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let err = load_listed_err(dir.path(), &["packages/a", "packages/missing"]);
        assert!(
            err.contains(&format!(
                "{}: not found (listed in \"changesette.packages\")",
                dir.path()
                    .join("packages")
                    .join("missing")
                    .join("package.json")
                    .display()
            )),
            "{err}"
        );
    }

    #[test]
    fn a_listed_package_with_a_broken_manifest_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "packages/a/package.json", "{\n");
        let err = load_listed_err(dir.path(), &["packages/a"]);
        assert!(
            err.starts_with(
                &dir.path()
                    .join("packages")
                    .join("a")
                    .join("package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }

    #[test]
    fn a_listed_package_that_is_a_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "packages/a", "not a directory\n");
        let err = load_listed_err(dir.path(), &["packages/a"]);
        assert!(
            err.starts_with(
                &dir.path()
                    .join("packages")
                    .join("a")
                    .join("package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }

    #[test]
    fn listed_packages_take_the_member_qualification() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"private\": true }\n",
        );
        write(
            dir.path(),
            "packages/c/package.json",
            "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/d/package.json",
            "{ \"name\": \"dup\", \"version\": \"2.0.0\" }\n",
        );
        let workspace = load_listed(
            dir.path(),
            &["packages/d", "packages/c", "packages/b", "packages/a"],
        )
        .unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    }

    #[test]
    fn listed_packages_win_over_the_reroot_enumeration() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        let (root, marker) = find_root(&dir.path().join("packages/a")).unwrap();
        assert_eq!(root, dir.path());
        assert!(matches!(marker, RootMarker::NpmReroot(_)));
        let packages = vec!["packages/b".to_owned()];
        let workspace = Workspace::load(&root, Some(&packages), Some(marker)).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-b", "packages/b")]);
    }

    #[cfg(unix)]
    #[test]
    fn listed_aliases_of_one_directory_collapse_into_one_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "real/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        let workspace = load_listed(dir.path(), &["real/a", "link/a"]).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("link/a").as_path())]
        );
    }

    #[cfg(windows)]
    #[test]
    fn listed_packages_keep_their_spelling_on_a_case_insensitive_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = load_listed(dir.path(), &["Packages/A"]).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "Packages/A")]);
        let workspace = load_listed(dir.path(), &["packages/a", "Packages/A"]).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "Packages/A")]);
    }

    #[test]
    fn listed_packages_are_spelled_in_the_normal_form() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "root/packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "root/b/package.json",
            "{ \"name\": \"pkg-b\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "root/package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let root = dir.path().join("root");
        let workspace = load_listed(
            &root,
            &["./../root/packages/./a", "a/../b", "./", "packages//a/"],
        )
        .unwrap();
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a"), ("pkg-b", "b"), ("root", ".")]
        );
        assert_eq!(
            workspace.member("pkg-a").unwrap().dir(),
            root.join("packages/a")
        );
        assert_eq!(workspace.member("pkg-b").unwrap().dir(), root.join("b"));
        assert_eq!(workspace.member("root").unwrap().dir(), root);
    }

    #[test]
    fn a_listed_package_climbing_past_the_filesystem_root_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let entry = format!("{}x", "../".repeat(64));
        let err = load_listed_err(dir.path(), &[&entry]);
        assert_eq!(
            err,
            format!(
                "invalid \"changesette.packages\" entry {entry:?}: escapes the filesystem root"
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_listed_package_with_a_drive_prefixed_segment_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for entry in ["../C:/x", "C:/x", "C:x", "packages/C:x"] {
            let err = load_listed_err(dir.path(), &[entry]);
            assert!(
                err.contains(&format!("entry {entry:?}: "))
                    && err.ends_with("is not a directory name"),
                "{err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_listed_drive_like_segment_is_an_ordinary_name_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "C:/x/package.json",
            "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = load_listed(&dir.path().join("root"), &["../C:/x"]).unwrap();
        assert_eq!(names_and_rel_dirs(&workspace), [("pkg-x", "../C:/x")]);
    }

    #[test]
    fn rejects_a_root_that_is_missing_or_a_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "file", "");
        let err = resolve_root(&dir.path().join("missing")).unwrap_err();
        assert_eq!(
            err.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::NotFound),
            "{err:#}"
        );
        let err = format!("{:#}", resolve_root(&dir.path().join("file")).unwrap_err());
        assert_eq!(err, "not a directory");
    }

    #[cfg(unix)]
    #[test]
    fn resolves_a_symlinked_root_to_its_physical_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        assert_eq!(
            resolve_root(&dir.path().join("link")).unwrap(),
            dir.path().canonicalize().unwrap().join("real")
        );
    }
}
