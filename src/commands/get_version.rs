use anyhow::Result;

use crate::{package_json::PackageJson, workspace::Workspace};

/// Prints the named package's version from its package.json to stdout as a
/// bare semver string.
pub(crate) fn run(package: &str) -> Result<()> {
    let workspace = Workspace::discover(&std::env::current_dir()?)?;
    let member = workspace.member(package)?;
    println!("{}", PackageJson::load(member.dir())?.version());
    Ok(())
}
