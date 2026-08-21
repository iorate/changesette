use anyhow::{Context, Result};
use serde::Serialize;

use crate::{output, package_json::PackageJson, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    private: bool,
    dir: String,
}

/// Prints the workspace members to stdout as a single-line JSON array of
/// `{name, version, private, dir}` objects in package name order, where
/// `version` is omitted when the member's package.json has no version field,
/// `private` is always a boolean, and `dir` is the member directory relative
/// to the workspace root, `/`-separated, or `.` for the root itself.
pub(crate) fn run() -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let packages = workspace
        .members()
        .iter()
        .map(|member| {
            let package_json = PackageJson::load(member.dir())?;
            let version = package_json.version().map(ToString::to_string);
            let private = package_json.private();
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
            Ok(Package {
                name: member.name(),
                version,
                private,
                dir,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    output::print_line(&serde_json::to_string(&packages)?)?;
    Ok(())
}
