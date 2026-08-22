use anyhow::{Context, Result};
use serde::Serialize;

use crate::{output, skip::SkipSet, workspace::Workspace};

#[derive(Serialize)]
struct Package<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    private: bool,
    dir: String,
}

/// Prints the packages managed by `version` — every workspace member with
/// `all` — to stdout as a single-line JSON array of
/// `{name, version, private, dir}` objects, `dir` being relative to the
/// workspace root.
pub(crate) fn run(all: bool) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let skip = SkipSet::load(&workspace, &workspace.root().join(".changeset"), &[])?;
    let mut packages = Vec::new();
    for member in workspace.members() {
        if !all && skip.contains(member.name()) {
            continue;
        }
        let mut base = workspace.root();
        let mut ups = 0;
        let rel = loop {
            if let Ok(rel) = member.dir().strip_prefix(base) {
                break rel;
            }
            base = base
                .parent()
                .with_context(|| member.dir().display().to_string())?;
            ups += 1;
        };
        let mut parts: Vec<String> = vec!["..".to_owned(); ups];
        parts.extend(
            rel.components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned()),
        );
        let dir = if parts.is_empty() {
            ".".to_owned()
        } else {
            parts.join("/")
        };
        packages.push(Package {
            name: member.name(),
            version: member.version().map(str::to_owned),
            private: member.private(),
            dir,
        });
    }
    output::print_line(&serde_json::to_string(&packages)?)?;
    Ok(())
}
