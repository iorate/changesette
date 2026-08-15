use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use saphyr::{LoadableYamlNode, Yaml};

/// A package discovered in the workspace: a directory whose `package.json`
/// has both a `name` and a `version` key, or the single-package fallback,
/// whose `package.json` only needs a `name`.
#[derive(Debug)]
pub(crate) struct Member {
    name: String,
    dir: PathBuf,
}

impl Member {
    /// The package name declared in the member's `package.json`.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The directory holding the member's `package.json`.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

/// The set of packages the commands operate on, resolved from the working
/// directory.
#[derive(Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
    members: Vec<Member>,
}

impl Workspace {
    /// Discovers the workspace by walking up from `cwd`. The first ancestor
    /// that is a workspace root wins: either its `pnpm-workspace.yaml` has a
    /// `packages` list (which may be empty; `workspaces` in `package.json` is
    /// then ignored, as pnpm ignores it), or, with no `pnpm-workspace.yaml`,
    /// its `package.json` has a `workspaces` array of strings (which may be
    /// empty; the Yarn 1 object form is an error). The members are the
    /// directories under the root matching the listed globs (`!` negates;
    /// `node_modules` and dot-directories are never entered; symlinks are
    /// followed) whose `package.json` has both `name` and `version` keys,
    /// sorted by package name; the root itself is not a member, and two
    /// members declaring the same name are an error. Without a workspace
    /// root, the directory nearest to `cwd` with a `package.json` becomes a
    /// single-member workspace, erroring if that `package.json` has no
    /// `name`; without even that, discovery fails.
    pub(crate) fn discover(cwd: &Path) -> Result<Self> {
        let mut nearest_package = None;
        for dir in cwd.ancestors() {
            let pnpm_path = dir.join("pnpm-workspace.yaml");
            let pnpm_text = read_optional(&pnpm_path)?;
            let package_path = dir.join("package.json");
            let package_text = read_optional(&package_path)?;
            if package_text.is_some() && nearest_package.is_none() {
                nearest_package = Some(dir);
            }
            if let Some(text) = pnpm_text {
                if let Some(patterns) =
                    pnpm_packages(&text).with_context(|| pnpm_path.display().to_string())?
                {
                    return Self::from_patterns(dir, &patterns);
                }
                continue;
            }
            if let Some(text) = package_text {
                if let Some(patterns) =
                    npm_workspaces(&text).with_context(|| package_path.display().to_string())?
                {
                    return Self::from_patterns(dir, &patterns);
                }
            }
        }

        let Some(dir) = nearest_package else {
            bail!(
                "no package.json found in {} or any parent directory",
                cwd.display()
            );
        };
        let path = dir.join("package.json");
        let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
        let name = package_name(&text)
            .with_context(|| path.display().to_string())?
            .with_context(|| format!("{}: missing top-level \"name\"", path.display()))?;
        Ok(Self {
            root: dir.to_owned(),
            members: vec![Member {
                name,
                dir: dir.to_owned(),
            }],
        })
    }

    /// The workspace root directory, holding the `.changeset` directory.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The members in package name order. Empty only for a workspace whose
    /// globs match nothing.
    pub(crate) fn members(&self) -> &[Member] {
        &self.members
    }

    /// Resolves the member declaring the given package name, erroring with
    /// the known names when there is no such member.
    pub(crate) fn member(&self, name: &str) -> Result<&Member> {
        self.members
            .iter()
            .find(|member| member.name == name)
            .with_context(|| {
                if self.members.is_empty() {
                    format!("package `{name}` not found: the workspace has no members")
                } else {
                    format!(
                        "package `{name}` not found; known packages: {}",
                        self.members
                            .iter()
                            .map(|member| member.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            })
    }

    fn from_patterns(root: &Path, patterns: &[String]) -> Result<Self> {
        let mut includes = GlobSetBuilder::new();
        let mut excludes = GlobSetBuilder::new();
        for pattern in patterns {
            let (builder, glob) = match pattern.strip_prefix('!') {
                Some(negated) => (&mut excludes, negated),
                None => (&mut includes, pattern.as_str()),
            };
            builder.add(
                GlobBuilder::new(glob)
                    .literal_separator(true)
                    .build()
                    .with_context(|| format!("invalid workspace pattern {pattern:?}"))?,
            );
        }
        let includes = includes.build()?;
        let excludes = excludes.build()?;

        let mut members = Vec::new();
        collect_members(root, "", &includes, &excludes, &mut members)?;
        members.sort_by(|a, b| (&a.name, &a.dir).cmp(&(&b.name, &b.dir)));
        for pair in members.windows(2) {
            ensure!(
                pair[0].name != pair[1].name,
                "duplicate package name `{}` declared in both {} and {}",
                pair[0].name,
                pair[0].dir.display(),
                pair[1].dir.display()
            );
        }
        Ok(Self {
            root: root.to_owned(),
            members,
        })
    }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(path.display().to_string()),
    }
}

fn pnpm_packages(text: &str) -> Result<Option<Vec<String>>> {
    let docs = match Yaml::load_from_str(text) {
        Ok(docs) => docs,
        Err(err) => bail!("invalid YAML: {err}"),
    };
    let Some(doc) = docs.into_iter().next() else {
        return Ok(None);
    };
    let Some(packages) = doc.as_mapping_get("packages") else {
        return Ok(None);
    };
    if packages.is_null() {
        return Ok(None);
    }
    let Yaml::Sequence(entries) = packages else {
        bail!("\"packages\" must be a list of strings")
    };
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .context("\"packages\" must be a list of strings")
        })
        .collect::<Result<_>>()
        .map(Some)
}

fn npm_workspaces(text: &str) -> Result<Option<Vec<String>>> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let object = value
        .as_object()
        .context("the root value must be an object")?;
    let Some(workspaces) = object.get("workspaces") else {
        return Ok(None);
    };
    let entries = workspaces.as_array().context(
        "\"workspaces\" must be an array of strings (the Yarn 1 object form is not supported)",
    )?;
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .context("\"workspaces\" must be an array of strings")
        })
        .collect::<Result<_>>()
        .map(Some)
}

fn package_name(text: &str) -> Result<Option<String>> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let object = value
        .as_object()
        .context("the root value must be an object")?;
    let Some(name) = object.get("name") else {
        return Ok(None);
    };
    valid_name(name).map(Some)
}

fn valid_name(name: &serde_json::Value) -> Result<String> {
    let name = name
        .as_str()
        .context("top-level \"name\" must be a string")?;
    ensure!(!name.is_empty(), "top-level \"name\" must not be empty");
    Ok(name.to_owned())
}

fn collect_members(
    dir: &Path,
    rel: &str,
    includes: &GlobSet,
    excludes: &GlobSet,
    members: &mut Vec<Member>,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| dir.display().to_string())?;
    for entry in entries {
        let entry = entry.with_context(|| dir.display().to_string())?;
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if file_name.starts_with('.') || file_name == "node_modules" {
            continue;
        }
        let path = entry.path();
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).context(path.display().to_string()),
        }
        let child_rel = if rel.is_empty() {
            file_name
        } else {
            format!("{rel}/{file_name}")
        };
        if includes.is_match(&child_rel) && !excludes.is_match(&child_rel) {
            if let Some(member) = read_member(&path)? {
                members.push(member);
            }
        }
        collect_members(&path, &child_rel, includes, excludes, members)?;
    }
    Ok(())
}

fn read_member(dir: &Path) -> Result<Option<Member>> {
    let path = dir.join("package.json");
    let Some(text) = read_optional(&path)? else {
        return Ok(None);
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    let object = value
        .as_object()
        .with_context(|| format!("{}: the root value must be an object", path.display()))?;
    let (Some(name), Some(_version)) = (object.get("name"), object.get("version")) else {
        return Ok(None);
    };
    let name = valid_name(name).with_context(|| path.display().to_string())?;
    Ok(Some(Member {
        name,
        dir: dir.to_owned(),
    }))
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
    fn pnpm_manifest_without_packages_is_not_a_workspace_root() {
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
        assert_eq!(names_and_dirs(&workspace), [("root", dir.path())]);
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
}
