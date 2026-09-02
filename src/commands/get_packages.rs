use anyhow::Result;
use serde::Serialize;

use crate::{config::Config, output, skip::SkipSet, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    version: String,
    private: bool,
    dir: &'a str,
}

pub(crate) fn run(workspace: &Workspace, config: &Config, all: bool) -> Result<()> {
    let skip = SkipSet::load(workspace, config, &[])?;
    let mut packages = Vec::new();
    for member in workspace.members() {
        if !all && skip.contains(member.name()) {
            continue;
        }
        packages.push(Package {
            name: member.name(),
            version: member.version().to_string(),
            private: member.private(),
            dir: member.rel_dir(),
        });
    }
    output::print_json(&packages)?;
    Ok(())
}
