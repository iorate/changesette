use anyhow::Result;
use serde::Serialize;

use crate::{output, workspace::Workspace};

#[derive(Serialize)]
struct Row<'a> {
    dir: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    private: bool,
}

pub fn run(workspace: &Workspace, all: bool) -> Result<()> {
    let rows: Vec<Row> = workspace
        .packages()
        .filter(|package| all || package.versioned().is_some())
        .map(|package| Row {
            dir: package.rel_dir().as_str(),
            name: package.name(),
            version: package.version().map(ToString::to_string),
            private: package.private(),
        })
        .collect();
    output::print_json(&rows)?;
    Ok(())
}
