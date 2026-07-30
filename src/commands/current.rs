use std::path::Path;

use anyhow::Result;

use crate::package_json::PackageJson;

/// Prints the current version from the working tree's package.json to stdout
/// as a bare semver string.
pub(crate) fn run() -> Result<()> {
    println!("{}", PackageJson::load(Path::new("."))?.version());
    Ok(())
}
