use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use saphyr::{LoadableYamlNode, Yaml};
use serde_json::Value;
use wax::{
    Glob, Program,
    walk::{Entry, FileIterator},
};

const IGNORE_PATTERNS: [&str; 2] = ["**/node_modules/**", "**/bower_components/**"];

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
    version: Option<String>,
    private: bool,
}

impl Workspace {
    /// Discovers the workspace containing `cwd`: the nearest ancestor that
    /// is a pnpm or npm workspace root wins, and without one the nearest
    /// ancestor with a `package.json` becomes a single-package workspace.
    pub(crate) fn discover(cwd: &Path) -> Result<Workspace> {
        let mut fallback = None;
        for dir in cwd.ancestors() {
            if let Some(patterns) = read_pnpm_manifest(dir)? {
                let manifest = dir.join("pnpm-workspace.yaml");
                return Ok(Workspace {
                    root: dir.to_path_buf(),
                    members: collect_members(dir, &manifest, &patterns, true)?,
                });
            }
            let path = dir.join("package.json");
            let Some(value) = read_json(&path)? else {
                continue;
            };
            let Some(object) = value.as_object() else {
                bail!("{}: the root value must be an object", path.display())
            };
            if let Some(workspaces) = object.get("workspaces") {
                let Some(patterns) = string_array(workspaces) else {
                    bail!(
                        "{}: \"workspaces\" must be an array of strings (the Yarn 1 object form is not supported)",
                        path.display()
                    )
                };
                return Ok(Workspace {
                    root: dir.to_path_buf(),
                    members: collect_members(dir, &path, &patterns, false)?,
                });
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
        let Some(name_value) = value.get("name") else {
            bail!("{}: missing top-level \"name\"", path.display())
        };
        let name = member_name(name_value, &path)?;
        let member = Member::from_manifest(name, dir.clone(), ".".to_owned(), &value);
        Ok(Workspace {
            root: dir,
            members: vec![member],
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// In package-name order; empty only for a workspace whose globs match
    /// nothing.
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
    fn from_manifest(name: String, dir: PathBuf, rel_dir: String, value: &Value) -> Member {
        Member {
            name,
            dir,
            rel_dir,
            version: value
                .get("version")
                .and_then(Value::as_str)
                .filter(|version| !version.is_empty())
                .map(str::to_owned),
            private: value
                .get("private")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The member directory relative to the workspace root, `/`-separated
    /// with leading `..` segments for a directory outside the root; `.` for
    /// the root itself.
    pub(crate) fn rel_dir(&self) -> &str {
        &self.rel_dir
    }

    /// The manifest's top-level `version` when it is a nonempty string.
    pub(crate) fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Whether the manifest sets top-level `private` to boolean `true`.
    pub(crate) fn private(&self) -> bool {
        self.private
    }
}

fn read_pnpm_manifest(dir: &Path) -> Result<Option<Vec<String>>> {
    let path = dir.join("pnpm-workspace.yaml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let docs = match Yaml::load_from_str(&text) {
        Ok(docs) => docs,
        Err(err) => bail!("{}: invalid YAML: {err}", path.display()),
    };
    let Some(Yaml::Mapping(mapping)) = docs.into_iter().next() else {
        return Ok(None);
    };
    let packages = mapping
        .iter()
        .find_map(|(key, value)| (key.as_str() == Some("packages")).then_some(value));
    let Some(packages) = packages else {
        return Ok(None);
    };
    if packages.is_null() {
        return Ok(None);
    }
    let Yaml::Sequence(items) = packages else {
        bail!("{}: \"packages\" must be a list of strings", path.display())
    };
    let mut patterns = Vec::new();
    for item in items {
        let Some(pattern) = item.as_str() else {
            bail!("{}: \"packages\" must be a list of strings", path.display())
        };
        patterns.push(pattern.to_owned());
    }
    Ok(Some(patterns))
}

/// Reads `path` as JSON, treating a missing file as `None` and attaching the
/// path to any error.
pub(crate) fn read_json(path: &Path) -> Result<Option<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    let value = serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    Ok(Some(value))
}

fn string_array(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn member_name(value: &Value, path: &Path) -> Result<String> {
    let Some(name) = value.as_str() else {
        bail!("{}: top-level \"name\" must be a string", path.display())
    };
    if name.is_empty() {
        bail!("{}: top-level \"name\" must not be empty", path.display())
    }
    Ok(name.to_owned())
}

fn collect_members(
    root: &Path,
    manifest: &Path,
    patterns: &[String],
    include_root: bool,
) -> Result<Vec<Member>> {
    let mut positives = Vec::new();
    let mut negatives = Vec::new();
    for original in patterns {
        let (negative, body) = match original.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, original.as_str()),
        };
        if body.is_empty() {
            continue;
        }
        // pnpm 12 keeps the leading `/` of an absolute pattern, so relative
        // workspace paths never match it, whether positive or negated.
        if body.starts_with('/') {
            continue;
        }
        let mut normalized = body
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != ".")
            .collect::<Vec<_>>()
            .join("/");
        if normalized.is_empty() {
            normalized.push_str("package.json");
        } else {
            normalized.push_str("/package.json");
        }
        if negative {
            let compiled = Glob::new(&normalized)
                .map(Glob::into_owned)
                .with_context(|| {
                    format!(
                        "{}: invalid workspace pattern {original:?}",
                        manifest.display()
                    )
                })?;
            negatives.push(compiled);
            continue;
        }
        // A leading `../` segment cannot be expressed in a wax glob, so it
        // walks the ancestor directory it names instead, as pnpm 12 does; a
        // traversal climbing past the filesystem root matches nothing.
        // Negations are exempt: they match the path each entry has from the
        // workspace root, which keeps its `../` segments.
        let mut walk_root = root;
        let mut rest = normalized.as_str();
        let mut escaped = false;
        while let Some(tail) = rest.strip_prefix("../") {
            let Some(parent) = walk_root.parent() else {
                escaped = true;
                break;
            };
            walk_root = parent;
            rest = tail;
        }
        if escaped {
            continue;
        }
        let compiled = Glob::new(rest).map(Glob::into_owned).with_context(|| {
            format!(
                "{}: invalid workspace pattern {original:?}",
                manifest.display()
            )
        })?;
        positives.push((compiled, walk_root));
    }

    let ignore = wax::any(IGNORE_PATTERNS)?;
    let mut dirs = BTreeSet::new();
    let mut members = Vec::new();
    for (glob, walk_root) in &positives {
        let walk = glob.walk(walk_root).not(ignore.clone())?;
        'candidates: for entry in walk {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    // NotFound covers a pattern whose base directory does not
                    // exist and entries deleted mid-walk, and NotADirectory a
                    // pattern whose path prefix crosses a regular file; pnpm
                    // 12 matches nothing in both cases. Anything else, such
                    // as a permission error, is reported.
                    let err = io::Error::from(err);
                    if matches!(
                        err.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) {
                        continue;
                    }
                    return Err(err.into());
                }
            };
            let path = entry.into_path();
            // The nearest ancestor of the root containing the entry; the
            // walk root is one, so the loop always terminates.
            let mut base = root;
            let mut ups = 0;
            let rel = loop {
                if let Ok(rel) = path.strip_prefix(base) {
                    break rel;
                }
                let Some(parent) = base.parent() else {
                    continue 'candidates;
                };
                base = parent;
                ups += 1;
            };
            let mut rel_parts = Vec::new();
            for component in rel.components() {
                match component {
                    Component::Normal(part) => match part.to_str() {
                        Some(part) => rel_parts.push(part),
                        None => continue 'candidates,
                    },
                    _ => continue 'candidates,
                }
            }
            let rel_str = "../".repeat(ups) + &rel_parts.join("/");
            if negatives
                .iter()
                .any(|negative| negative.is_match(rel_str.as_str()))
            {
                continue;
            }
            let dir = if rel_parts.len() <= 1 {
                base.to_path_buf()
            } else {
                base.join(rel_parts[..rel_parts.len() - 1].join("/"))
            };
            if !dirs.insert(dir.clone()) {
                continue;
            }
            let Some(value) = read_json(&path)? else {
                continue;
            };
            let Some(name_value) = value.get("name") else {
                continue;
            };
            let name = member_name(name_value, &path)?;
            let mut rel_dir_parts = vec![".."; ups];
            rel_dir_parts.extend(&rel_parts[..rel_parts.len() - 1]);
            let rel_dir = if rel_dir_parts.is_empty() {
                ".".to_owned()
            } else {
                rel_dir_parts.join("/")
            };
            members.push(Member::from_manifest(name, dir, rel_dir, &value));
        }
    }

    // pnpm always makes the workspace root a project, whatever the patterns
    // say (pnpm/pnpm#1986).
    if include_root && !dirs.contains(root) {
        let path = root.join("package.json");
        if let Some(value) = read_json(&path)? {
            if let Some(name_value) = value.get("name") {
                let name = member_name(name_value, &path)?;
                members.push(Member::from_manifest(
                    name,
                    root.to_path_buf(),
                    ".".to_owned(),
                    &value,
                ));
            }
        }
    }

    members.sort_by(|a, b| (&a.name, &a.dir).cmp(&(&b.name, &b.dir)));
    for pair in members.windows(2) {
        if pair[0].name == pair[1].name {
            bail!(
                "duplicate package name `{}` declared in both {} and {}",
                pair[0].name,
                pair[0].dir.display(),
                pair[1].dir.display()
            );
        }
    }
    Ok(members)
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
    fn walks_nested_directories_and_dot_directories_skipping_node_modules() {
        insta::assert_debug_snapshot!(discover_ok("deep"));
    }

    #[test]
    fn rejects_the_yarn_1_workspaces_object() {
        insta::assert_snapshot!(discover_err("yarn1-object"));
    }

    #[test]
    fn rejects_non_list_pnpm_packages() {
        insta::assert_snapshot!(discover_err("pnpm-bad-packages"));
    }

    #[test]
    fn rejects_an_invalid_glob() {
        insta::assert_snapshot!(discover_err("bad-glob"));
    }

    #[test]
    fn rejects_duplicate_package_names() {
        insta::assert_snapshot!(discover_err("duplicate-names"));
    }

    #[test]
    fn rejects_a_non_string_member_name() {
        insta::assert_snapshot!(discover_err("bad-name"));
    }

    #[test]
    fn accepts_an_empty_workspaces_array() {
        assert_eq!(names_and_dirs(&discover_ok("npm-empty")), []);
    }

    #[test]
    fn accepts_an_empty_pnpm_packages_list() {
        assert_eq!(names_and_dirs(&discover_ok("pnpm-empty")), []);
    }

    #[test]
    fn does_not_follow_a_symlinked_member_directory() {
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
    fn pnpm_manifest_without_packages_does_not_suppress_npm_workspaces() {
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
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn pnpm_manifest_with_null_packages_is_not_a_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pnpm-workspace.yaml", "packages:\n");
        write(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n",
        );
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
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
    fn errors_when_the_nearest_package_json_has_no_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"version\": \"1.0.0\" }\n");
        let err = format!("{:#}", Workspace::discover(dir.path()).unwrap_err());
        assert!(err.contains("missing top-level \"name\""), "{err}");
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
    fn always_includes_the_pnpm_root_package() {
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
    fn excludes_a_nameless_pnpm_root_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n",
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
    fn reads_member_version_and_private_leniently() {
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
        let workspace = Workspace::discover(dir.path()).unwrap();
        let members = workspace.members();
        assert_eq!(members[0].version(), Some("1.0.0"));
        assert!(members[0].private());
        assert_eq!(members[1].version(), None);
        assert!(!members[1].private());
        assert_eq!(members[2].version(), None);
        assert!(!members[2].private());
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
    fn matches_dot_directories_with_a_wildcard() {
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
            [
                ("pkg-a", dir.path().join("a").as_path()),
                ("tool", dir.path().join(".tools").as_path()),
            ]
        );
    }

    #[test]
    fn matches_brace_alternatives() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/{a,b}\"\n",
        );
        for name in ["a", "b", "c"] {
            write(
                dir.path(),
                &format!("packages/{name}/package.json"),
                &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", dir.path().join("packages/a").as_path()),
                ("pkg-b", dir.path().join("packages/b").as_path()),
            ]
        );
    }

    #[test]
    fn follows_a_parent_directory_pattern() {
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
        let workspace = Workspace::discover(&dir.path().join("ws")).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("sibling/a").as_path())]
        );
    }

    #[test]
    fn a_negation_matches_a_parent_directory_member_by_its_dotted_path() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ws/pnpm-workspace.yaml",
            "packages:\n  - \"../sibling/*\"\n  - \"!../sibling/b\"\n",
        );
        for name in ["a", "b"] {
            write(
                dir.path(),
                &format!("sibling/{name}/package.json"),
                &format!("{{ \"name\": \"pkg-{name}\", \"version\": \"1.0.0\" }}\n"),
            );
        }
        let workspace = Workspace::discover(&dir.path().join("ws")).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("sibling/a").as_path())]
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
    fn reports_a_permission_error_inside_a_pattern() {
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
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("packages/denied"), "{err}");
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
    fn ignores_an_absolute_negative_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"packages/*\"\n  - \"!/packages/a\"\n",
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
    fn ignores_an_absolute_positive_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - \"/packages/*\"\n  - \"apps/*\"\n",
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
            [("app-b", dir.path().join("apps/b").as_path())]
        );
    }

    #[test]
    fn accepts_a_versionless_member_in_an_npm_workspace() {
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
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn accepts_a_versionless_member_in_a_pnpm_workspace() {
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
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-a", dir.path().join("packages/a").as_path())]
        );
    }

    #[test]
    fn accepts_a_versionless_single_package() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ \"name\": \"app\" }\n");
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(names_and_dirs(&workspace), [("app", dir.path())]);
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
            "{ \"name\": \"pkg-b\" }\n",
        );
        let workspace = Workspace::discover(dir.path()).unwrap();
        assert_eq!(
            names_and_dirs(&workspace),
            [("pkg-b", dir.path().join("packages/b").as_path())]
        );
    }
}
