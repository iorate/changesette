use std::{fs, io, path::Path};

use anyhow::{Context, Result};

use crate::{
    bump,
    changelog::{self, render_section},
    changeset,
    package_json::PackageJson,
};

/// Consumes every changeset: bumps package.json, inserts the new section into
/// CHANGELOG.md, deletes the consumed files, and prints the next version to
/// stdout. With zero changesets, does nothing and prints nothing. With
/// `dry_run`, computes everything but prints the plan to stderr instead of
/// touching any file.
pub(crate) fn run(dry_run: bool) -> Result<()> {
    let dir = Path::new(".");
    let changeset_dir = Path::new(".changeset");

    let mut package_json = PackageJson::load(dir)?;
    let changes = changeset::load(changeset_dir, package_json.name())?;
    if changes.is_empty() {
        eprintln!("note: no changesets found; nothing to do");
        return Ok(());
    }

    let entries: Vec<(bump::Bump, &str)> = changes
        .iter()
        .map(|change| (change.bump, change.summary.as_str()))
        .collect();

    let current = package_json.version().clone();
    // max_bump is None only for zero changesets, which returned early above.
    let next = bump::next_version(&current, changeset::max_bump(&changes).unwrap());
    let section = render_section(&next, &entries);

    package_json.set_version(&next)?;
    let changelog_path = Path::new("CHANGELOG.md");
    let changelog_text = match fs::read_to_string(changelog_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).context(changelog_path.display().to_string()),
    };
    let new_changelog_text = changelog::upsert_section(
        &changelog_text,
        package_json.name(),
        &next.to_string(),
        &section,
    );

    if dry_run {
        eprintln!("dry run: no files will be modified");
        eprintln!(
            "would consume {} {}:",
            changes.len(),
            if changes.len() == 1 {
                "changeset"
            } else {
                "changesets"
            }
        );
        for change in &changes {
            eprintln!(
                "  .changeset/{} ({})",
                change.file_name,
                change.bump.as_str()
            );
        }
        eprintln!("would update package.json: {current} -> {next}");
        eprintln!("would insert into CHANGELOG.md:\n\n{section}");
    } else {
        package_json.save()?;
        fs::write(changelog_path, new_changelog_text)
            .with_context(|| changelog_path.display().to_string())?;
        for change in &changes {
            let path = changeset_dir.join(&change.file_name);
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
    }

    println!("{next}");
    Ok(())
}
