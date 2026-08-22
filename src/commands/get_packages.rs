use anyhow::Result;
use serde::Serialize;

use crate::{config, output, skip::SkipSet, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    private: bool,
    dir: &'a str,
}

/// Prints the packages managed by `version` — every workspace member with
/// `all` — to stdout as a single-line JSON array of
/// `{name, version, private, dir}` objects, `dir` being relative to the
/// workspace root.
pub(crate) fn run(all: bool) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let config = config::load(&workspace.root().join(".changeset"))?;
    let skip = SkipSet::load(&workspace, &config, &[])?;
    let mut packages = Vec::new();
    for member in workspace.members() {
        if !all && skip.contains(member.name()) {
            continue;
        }
        packages.push(Package {
            name: member.name(),
            version: member.version().map(str::to_owned),
            private: member.private(),
            dir: member.rel_dir(),
        });
    }
    output::print_line(&serde_json::to_string(&packages)?)?;
    Ok(())
}
