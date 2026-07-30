use std::{fs, io, path::Path};

use anyhow::{Context, Result};

use crate::{
    bump,
    changelog::{
        document,
        entries::{Entry, render_section},
    },
    changeset, config,
    github::GithubClient,
    package_json::PackageJson,
    package_lock::PackageLock,
};

/// Consumes every changeset: resolves PR links (when the GitHub integration
/// is on), bumps package.json (and package-lock.json if present), inserts the
/// new section into CHANGELOG.md, deletes the consumed files, and prints the
/// next version to stdout. With zero changesets, does nothing and prints
/// nothing. With `dry_run`, computes everything (network included) but prints
/// the plan to stderr instead of touching any file.
pub(crate) fn run(dry_run: bool) -> Result<()> {
    let dir = Path::new(".");
    let changeset_dir = Path::new(".changeset");

    let mut package_json = PackageJson::load(dir)?;
    let mut package_lock = PackageLock::load(dir)?;
    let changes = changeset::load(changeset_dir, package_json.name())?;
    if changes.is_empty() {
        eprintln!("note: no changesets found; nothing to do");
        return Ok(());
    }

    let config = config::load(dir)?;
    let entries: Vec<(bump::Bump, Entry)> = match &config.github_repo {
        Some(repository) => {
            let mut client = GithubClient::new(repository);
            changes
                .iter()
                .map(|change| {
                    let prs = match client.merged_prs_for_changeset(&change.file_name)? {
                        Some(prs) => prs,
                        None => {
                            eprintln!(
                                "warning: no commits found for .changeset/{}; generating the \
                                 entry without PR links",
                                change.file_name
                            );
                            Vec::new()
                        }
                    };
                    Ok((
                        change.bump,
                        Entry {
                            prs,
                            body: change.summary.clone(),
                        },
                    ))
                })
                .collect::<Result<_>>()?
        }
        None => changes
            .iter()
            .map(|change| {
                (
                    change.bump,
                    Entry {
                        prs: Vec::new(),
                        body: change.summary.clone(),
                    },
                )
            })
            .collect(),
    };

    let current = package_json.version().clone();
    // max_bump is None only for zero changesets, which returned early above.
    let next = bump::next_version(&current, changeset::max_bump(&changes).unwrap());
    let section = render_section(&next, &entries, config.github_repo.as_deref());

    package_json.set_version(&next)?;
    if let Some(package_lock) = &mut package_lock {
        package_lock.set_version(&next)?;
    }
    let changelog_path = Path::new("CHANGELOG.md");
    let changelog_text = match fs::read_to_string(changelog_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).context(changelog_path.display().to_string()),
    };
    let new_changelog_text = document::upsert_section(
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
        eprintln!(
            "would update {}: {current} -> {next}",
            if package_lock.is_some() {
                "package.json, package-lock.json"
            } else {
                "package.json"
            }
        );
        eprintln!("would insert into CHANGELOG.md:\n\n{section}");
    } else {
        package_json.save()?;
        if let Some(package_lock) = &package_lock {
            package_lock.save()?;
        }
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
