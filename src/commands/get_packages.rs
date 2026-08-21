use anyhow::{Context, Result};
use serde::Serialize;

use crate::{config, output, package_json::PackageJson, skip, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    private: bool,
    dir: String,
}

/// Prints the packages managed by `version` — the workspace members it does
/// not skip, so `version` is always present — to stdout as a single-line JSON
/// array of `{name, version, private, dir}` objects in package name order,
/// where `private` is always a boolean and `dir` is the member directory
/// relative to the workspace root, `/`-separated, or `.` for the root
/// itself. With `all`, prints every workspace member instead; only then may
/// `version` be omitted, when the member's package.json has no version field.
pub(crate) fn run(all: bool) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let config = config::load(&workspace.root().join(".changeset"))?;
    let mut packages = Vec::new();
    for member in workspace.members() {
        let package_json = PackageJson::load(member.dir())?;
        if !all && skip::should_skip(&package_json, &config, &[]) {
            continue;
        }
        let rel = member
            .dir()
            .strip_prefix(workspace.root())
            .with_context(|| member.dir().display().to_string())?;
        let dir = if rel.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            rel.components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        };
        packages.push(Package {
            name: member.name(),
            version: package_json.version().map(ToString::to_string),
            private: package_json.private(),
            dir,
        });
    }
    output::print_line(&serde_json::to_string(&packages)?)?;
    Ok(())
}
