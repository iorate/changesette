use anyhow::{Context, Result};
use serde::Serialize;

use crate::{output, package_json::PackageJson, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    version: String,
    dir: String,
}

/// Prints the workspace members to stdout as a single-line JSON array of
/// `{name, version, dir}` objects in package name order, where `dir` is the
/// member directory relative to the workspace root, `/`-separated, or `.` for
/// the root itself.
pub(crate) fn run() -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let packages = workspace
        .members()
        .iter()
        .map(|member| {
            let version = PackageJson::load(member.dir())?.version().to_string();
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
                dir,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    output::print_line(&serde_json::to_string(&packages)?)?;
    Ok(())
}
