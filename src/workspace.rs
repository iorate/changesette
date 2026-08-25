mod pattern;
mod walk;

use std::{
    fs, io,
    path::{Path, PathBuf},
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

impl Workspace {
    /// Discovers the workspace containing `cwd`: the nearest ancestor that
    /// is a pnpm or npm workspace root wins, and without one the nearest
    /// ancestor with a `package.json` becomes a single-package workspace.
    pub(crate) fn discover(cwd: &Path) -> Result<Workspace> {
        let mut fallback = None;
        for dir in cwd.ancestors() {
            // The presence of a pnpm-workspace.yaml alone marks a pnpm root,
            // a settings-only file included; when both markers share a
            // directory the yaml wins, matching every tool's behavior on the
            // repositories that migrated to pnpm and kept the old
            // `workspaces` field.
            let manifest = dir.join("pnpm-workspace.yaml");
            if probe_is_file(&manifest) {
                let patterns = pnpm_patterns(&manifest)?;
                return Ok(Workspace::new(
                    dir.to_path_buf(),
                    collect_members(dir, &manifest, &patterns, true)?,
                ));
            }
            let path = dir.join("package.json");
            if !probe_is_file(&path) {
                continue;
            }
            // A parse failure is always an error: walking past a broken npm
            // root would silently pick another root (an outer marker or the
            // single-package fallback), and merge conflict markers left in a
            // root package.json are a realistic way to get here.
            let Some(value) = read_manifest(&path)? else {
                continue;
            };
            // An array or object `workspaces` marks an npm-family root and
            // a null one reads as an absent field, as npm and pnpm both
            // tolerate a null pattern container; any other type is an error
            // even mid-walk — npm fails the same way wherever the walk
            // reads a truthy invalid type, and a false or 0, which npm
            // passes over, is too clearly a mistake to ignore. A stray
            // non-object manifest still passes by.
            if let Some(workspaces) = value.get("workspaces")
                && !workspaces.is_null()
            {
                if !workspaces.is_array() && !workspaces.is_object() {
                    bail!(
                        "{}: \"workspaces\" must be an array or an object",
                        path.display()
                    )
                }
                let patterns = npm_patterns(workspaces, &path)?;
                return Ok(Workspace::new(
                    dir.to_path_buf(),
                    collect_members(dir, &path, &patterns, false)?,
                ));
            }
            if fallback.is_none() {
                fallback = Some((dir.to_path_buf(), path, value));
            }
        }

        let Some((dir, path, value)) = fallback else {
            bail!(
                "no package.json found in {} or any parent directory",
                cwd.display()
            )
        };
        // The fallback manifest takes the same qualification as any member;
        // failing it leaves a workspace with zero members rather than an
        // error.
        let members = qualify(&value, dir.clone(), ".".to_owned(), &path)
            .into_iter()
            .collect();
        Ok(Workspace::new(dir, members))
    }

    // The one construction point, so that every discovery path reports the
    // final member list — the shortest answer to "why is my package not
    // found".
    fn new(root: PathBuf, members: Vec<Member>) -> Workspace {
        if members.is_empty() {
            debug!("workspace {}: no members", root.display());
        } else {
            // The list is built inside the macro so that the event macro's
            // enabled check makes it free at the default level.
            debug!(
                "workspace {}: members: {}",
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

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// In package-name order; empty when the globs match nothing or every
    /// candidate fails member qualification.
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

    /// Renders `path` relative to `cwd` for display; returns `path`
    /// unchanged when either path is not under the workspace root.
    pub(crate) fn display_path(&self, cwd: &Path, path: &Path) -> PathBuf {
        match (cwd.strip_prefix(&self.root), path.strip_prefix(&self.root)) {
            (Ok(cwd_rel), Ok(path_rel)) => {
                let mut display = PathBuf::new();
                for _ in cwd_rel.components() {
                    display.push("..");
                }
                display.push(path_rel);
                display
            }
            _ => path.to_path_buf(),
        }
    }
}

impl Member {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The member directory relative to the workspace root, `/`-separated;
    /// `.` for the root itself.
    pub(crate) fn rel_dir(&self) -> &str {
        &self.rel_dir
    }

    /// The manifest's top-level `version`, parsed at discovery; every member
    /// has one, as versionless candidates are excluded.
    pub(crate) fn version(&self) -> &Version {
        &self.version
    }

    /// Whether the manifest sets top-level `private` to boolean `true`.
    pub(crate) fn private(&self) -> bool {
        self.private
    }
}

/// Whether `path` is an existing file, following symlinks; a probe failure
/// is never fatal, but [`report_fs_error`] decides whether it warns.
pub(crate) fn probe_is_file(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        Err(err) => {
            report_fs_error(path, &err);
            false
        }
    }
}

/// Reports a filesystem error swallowed by discovery as a warning.
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

// Reads a package.json, treating a missing file as `None` and a parse
// failure as an error. A UTF-8 BOM is stripped before parsing: BOM'd
// manifests exist in the wild and pnpm accepts them.
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

/// Reads `path` as JSON, treating a missing file as `None` and attaching the
/// path to any error; unlike the workspace manifests, a BOM is not accepted,
/// matching the upstream's plain `JSON.parse` of config.json.
pub(crate) fn read_json(path: &Path) -> Result<Option<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let value = serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    Ok(Some(value))
}

// Reads the `packages` patterns from a pnpm-workspace.yaml. A null
// `packages` is the plain YAML spelling of an empty list and reads as
// absent, matching pnpm; any other type mismatch is an error, as it is in
// pnpm itself.
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

// Reads the patterns from an npm-family `workspaces` field, already known to
// be an array or an object; the object form must carry a `packages` list —
// npm errors on any object without one, a missing or null key included —
// and a non-string entry is an error too.
fn npm_patterns(workspaces: &Value, path: &Path) -> Result<Vec<String>> {
    let items = match workspaces {
        Value::Array(items) => items,
        // The Yarn 1 object form `{packages: [...], nohoist: [...]}`; every
        // key but `packages` is ignored.
        Value::Object(object) => match object.get("packages") {
            Some(Value::Array(items)) => items,
            _ => bail!(
                "{}: \"packages\" in \"workspaces\" must be a list of strings",
                path.display()
            ),
        },
        _ => unreachable!(),
    };
    let mut patterns = Vec::new();
    for item in items {
        let Some(pattern) = item.as_str() else {
            bail!(
                "{}: \"workspaces\" patterns must be strings",
                path.display()
            )
        };
        patterns.push(pattern.to_owned());
    }
    Ok(patterns)
}

fn collect_members(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    pnpm: bool,
) -> Result<Vec<Member>> {
    let mut positives = Vec::new();
    let mut negations = Vec::new();
    for original in patterns {
        let (negated, compiled) = pattern::compile(original).with_context(|| {
            format!(
                "{}: invalid workspace pattern {original:?}",
                manifest.display()
            )
        })?;
        if negated {
            negations.push(compiled);
        } else {
            positives.push(compiled);
        }
    }

    let mut candidates = walk::collect(root, &positives, &negations);
    // The pnpm root is always a member candidate, pattern or no pattern, and
    // not even a negation excludes it (pnpm #1986); the npm family includes
    // the root only when a pattern matches it.
    if pnpm && probe_is_file(&root.join("package.json")) {
        candidates.insert(".".to_owned(), root.to_path_buf());
    }

    let mut members = Vec::new();
    for (rel_dir, dir) in candidates {
        let path = dir.join("package.json");
        let Some(value) = read_manifest(&path)? else {
            continue;
        };
        if let Some(member) = qualify(&value, dir, rel_dir, &path) {
            members.push(member);
        }
    }
    members.sort_by(|a, b| (&a.name, &a.dir).cmp(&(&b.name, &b.dir)));
    exclude_duplicate_names(&mut members);
    Ok(members)
}

// Applies the member qualification: a candidate without a nonempty string
// `name` and a valid semver `version` is excluded rather than erroring, so a
// repository with fixture or junk manifests still works; changesette cannot
// address a nameless package anyway, and cannot bump a versionless one. A
// missing `name` or `version` key is only reported at debug level — fixture,
// private-root, and docs-site manifests omit them legitimately — while a key
// carrying an invalid value can only be a mistake and warns.
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
        Some(Value::String(version)) => match version.parse::<Version>() {
            Ok(version) => version,
            Err(_) => {
                warn!(
                    "{}: not a workspace member: \"version\" {version:?} is not a valid semver",
                    path.display()
                );
                return None;
            }
        },
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

// Excludes every member of a duplicated name: changesette can only address
// packages by name, so a duplicated one cannot be referred to at all, and an
// error here would make repositories with duplicated fixture names unusable.
// Qualification runs first, so a disqualified candidate sharing a real
// package's name does not evict it. Candidates that are the same physical
// directory under different paths (a symlink alias, a case variant) are one
// package, not a duplication, and collapse into the first of the (name, dir)
// order; an `is_same_file` error counts as a distinct directory, falling back
// to the exclusion. Expects `members` sorted by (name, dir), so name groups
// are contiguous and the kept alias is deterministic.
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

    fn discover_ok(case: &str) -> Workspace {
        Workspace::discover(&fixture(case)).unwrap()
    }

    fn discover_err(case: &str) -> String {
        format!("{:#}", Workspace::discover(&fixture(case)).unwrap_err())
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
        insta::assert_debug_snapshot!(discover_ok("npm"));
    }

    #[test]
    fn discovers_pnpm_workspace_members_with_exclusions() {
        insta::assert_debug_snapshot!(discover_ok("pnpm"));
    }

    #[test]
    fn walks_nested_directories_skipping_node_modules_and_dot_directories() {
        insta::assert_debug_snapshot!(discover_ok("deep"));
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
    fn excludes_a_wildcard_matched_symlinked_member_directory() {
        let workspace = discover_ok("symlink");
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", fixture("symlink/packages/a").as_path())]
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
    }

    #[test]
    fn rejects_a_non_mapping_pnpm_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "- packages/*\n");
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(&inner).unwrap();
        assert_eq!(workspace.root(), inner);
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
        let workspace = Workspace::discover(&dir.path().join("packages/a")).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(&dir.path().join("app/src")).unwrap();
        assert_eq!(workspace.root(), dir.path().join("app"));
        assert_eq!(
            names_and_dirs(&workspace),
            [("app", dir.path().join("app").as_path())]
        );
    }

    #[test]
    fn errors_without_any_package_json() {
        let dir = tempfile::tempdir().unwrap();
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
        assert!(err.contains("no package.json found"), "{err}");
    }

    #[test]
    fn a_nameless_fallback_package_yields_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"version\": \"1.0.0\" }\n");
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("deep", dir.path().join("packages/nested/deep").as_path()),
                ("direct", dir.path().join("packages").as_path()),
                (
                    "old",
                    dir.path().join("packages/bower_components/old").as_path()
                ),
            ]
        );
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn an_inner_workspaces_field_shadows_an_outer_workspace() {
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
        let inner = dir.path().join("packages/a");
        let workspace = Workspace::discover(&inner).unwrap();
        assert_eq!(workspace.root(), inner);
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-x", inner.join("nested/x").as_path())]
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
            "packages:\n  - \"packages/{a,b}\"\n",
        );
        write(
            dir.path(),
            "packages/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        write(
            dir.path(),
            "packages/{a,b}/package.json",
            "{ \"name\": \"pkg-braced\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn rejects_a_parent_directory_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"../sibling/*\"\n",
        );
        write(
            dir.path(),
            "sibling/a/package.json",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
        );
        let err = format!(
            "{:#}",
            Workspace::discover(&dir.path().join("ws")).unwrap_err()
        );
        assert!(
            err.contains("invalid workspace pattern \"../sibling/*\""),
            "{err}"
        );
    }

    #[test]
    fn rejects_a_parent_directory_negation() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - \"!../sibling/b\"\n",
        );
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
        assert!(
            err.contains("invalid workspace pattern \"!../sibling/b\""),
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let result = Workspace::discover(dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn negation_is_order_independent() {
        for patterns in [
            "packages:\n  - \"!packages/b\"\n  - \"packages/a\"\n  - \"packages/b\"\n",
            "packages:\n  - \"packages/a\"\n  - \"packages/b\"\n  - \"!packages/b\"\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), "pnpm-workspace.yaml", patterns);
            for name in ["a", "b"] {
                write(
                    dir.path(),
                    &format!("packages/{name}/package.json"),
                    &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
                );
            }
            let workspace = Workspace::discover(dir.path()).unwrap();
            assert_eq!(
                names_and_dirs(&workspace),
                [("pkg-a", dir.path().join("packages/a").as_path())],
                "{patterns}"
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
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), []);
    }

    #[test]
    fn a_versionless_single_package_yields_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"name\": \"app\" }\n");
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("root", dir.path()),
            ]
        );
    }

    #[test]
    fn passes_over_a_null_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": null }\n",
        );
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), [("app", dir.path())]);
    }

    #[test]
    fn rejects_an_invalid_workspaces_type() {
        for workspaces in ["\"packages/*\"", "42", "true", "false", "0", "\"\""] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                &format!(
                    "{{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
                ),
            );
            let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let err = format!(
            "{:#}",
            Workspace::discover(&dir.path().join("pkg")).unwrap_err()
        );
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
            let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
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
        let err = format!(
            "{:#}",
            Workspace::discover(&dir.path().join("app")).unwrap_err()
        );
        assert!(
            err.contains(&dir.path().join("app/package.json").display().to_string()),
            "{err}"
        );
    }

    #[test]
    fn passes_over_a_type_invalid_stray_manifest() {
        for stray in ["[1, 2]", "\"hello\""] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                "package.json",
                "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [] }\n",
            );
            write(dir.path(), "sub/package.json", stray);
            let workspace = Workspace::discover(&dir.path().join("sub")).unwrap();
            assert_eq!(workspace.root(), dir.path(), "{stray}");
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
        let workspace = Workspace::discover(&dir.path().join("sub")).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let workspace = Workspace::discover(dir.path()).unwrap();
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(workspace.root(), dir.path());
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
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
        assert!(
            err.contains(
                &dir.path()
                    .join("packages/a/package.json")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
    }
}
