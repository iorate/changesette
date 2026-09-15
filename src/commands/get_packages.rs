use anyhow::Result;
use serde::Serialize;

use crate::{output, workspace::Workspace};

#[derive(Serialize)]
struct Row<'a> {
    name: Option<&'a str>,
    version: Option<String>,
    private: bool,
    dir: &'a str,
}

pub fn run(workspace: &Workspace, all: bool) -> Result<()> {
    let rows: Vec<Row> = workspace
        .packages()
        .filter(|package| all || package.versionable().is_some())
        .map(|package| Row {
            name: package.name(),
            version: package.version().map(ToString::to_string),
            private: package.private(),
            dir: package.rel_dir().as_str(),
        })
        .collect();
    output::print_json(&rows)?;
    Ok(())
}
