use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use glob::{MatchOptions, Pattern};
use saphyr::{LoadableYamlNode, Yaml};
use serde_json::Value;

const MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: true,
};

#[derive(Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
    members: Vec<Member>,
}

#[derive(Debug)]
pub(crate) struct Member {
    name: String,
    dir: PathBuf,
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
                    members: collect_members(dir, &manifest, &patterns)?,
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
                    members: collect_members(dir, &path, &patterns)?,
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
        Ok(Workspace {
            root: dir.clone(),
            members: vec![Member { name, dir }],
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
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
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

fn read_json(path: &Path) -> Result<Option<Value>> {
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

fn collect_members(root: &Path, manifest: &Path, patterns: &[String]) -> Result<Vec<Member>> {
    let Some(root_str) = root.to_str() else {
        bail!(
            "the workspace root path {} is not valid UTF-8",
            root.display()
        )
    };

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
        let compiled = Pattern::new(&normalized).with_context(|| {
            format!(
                "{}: invalid workspace pattern {original:?}",
                manifest.display()
            )
        })?;
        if negative {
            negatives.push(compiled);
        } else {
            positives.push((original, normalized));
        }
    }

    let escaped_root = Pattern::escape(root_str);
    let mut dirs = BTreeSet::new();
    let mut members = Vec::new();
    for (original, normalized) in &positives {
        let full = format!("{escaped_root}/{normalized}");
        let paths = glob::glob_with(&full, MATCH_OPTIONS).with_context(|| {
            format!(
                "{}: invalid workspace pattern {original:?}",
                manifest.display()
            )
        })?;
        'candidates: for entry in paths {
            let Ok(path) = entry else {
                continue;
            };
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let mut rel_parts = Vec::new();
            for component in rel.components() {
                match component {
                    Component::Normal(part) => match part.to_str() {
                        Some("node_modules") | None => continue 'candidates,
                        Some(part) => rel_parts.push(part),
                    },
                    Component::ParentDir => rel_parts.push(".."),
                    Component::CurDir => {}
                    _ => continue 'candidates,
                }
            }
            let rel_str = rel_parts.join("/");
            if negatives
                .iter()
                .any(|negative| negative.matches_with(&rel_str, MATCH_OPTIONS))
            {
                continue;
            }
            let dir = if rel_parts.len() <= 1 {
                root.to_path_buf()
            } else {
                root.join(rel_parts[..rel_parts.len() - 1].join("/"))
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
            members.push(Member { name, dir });
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
    fn walks_nested_directories_skipping_node_modules_and_dot_directories() {
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
    fn follows_a_symlinked_member_directory() {
        let workspace = discover_ok("symlink");
        assert_eq!(
            names_and_dirs(&workspace),
            [
                ("pkg-a", fixture("symlink/packages/a").as_path()),
                ("pkg-b", fixture("symlink/packages/b").as_path()),
            ]
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
