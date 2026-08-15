use std::{env, fs, io, path::PathBuf};

use anyhow::{Context, Result};

use crate::{
    bump::{self, Bump},
    changelog::{self, render_entry, render_section},
    changeset::{self, LoadedChange},
    package_json::PackageJson,
    release_plan::{ChangesetEntry, Release, ReleasePlan, ReleaseRef},
    workspace::Workspace,
};

/// Consumes every changeset in the workspace: bumps each named package's
/// package.json, inserts the new section into its CHANGELOG.md, deletes the
/// consumed files (`none`-only and empty changesets included), and prints the
/// release plan to stdout as single-line JSON, each bumped release carrying
/// its new changelog entry. Packages named only with
/// `none` keep their version and changelog. With zero changesets, prints an
/// empty plan and touches nothing. With `dry_run`, prints the same JSON but
/// touches no file.
pub(crate) fn run(dry_run: bool) -> Result<()> {
    let workspace = Workspace::discover(&env::current_dir()?)?;
    let changeset_dir = workspace.root().join(".changeset");
    let changes = changeset::load(&changeset_dir)?;

    for change in &changes {
        for (name, _) in &change.releases {
            workspace
                .member(name)
                .with_context(|| changeset_dir.join(&change.file_name).display().to_string())?;
        }
    }

    let mut releases = Vec::new();
    let mut writes: Vec<(PackageJson, PathBuf, String)> = Vec::new();
    for (name, max_bump) in changeset::max_bumps(&changes) {
        let member = workspace.member(name)?;
        let mut package_json = PackageJson::load(member.dir())?;
        let old_version = package_json.version().clone();
        let ids = changes
            .iter()
            .filter(|change| change.releases.iter().any(|(n, _)| n == name))
            .map(id)
            .collect();

        let (new_version, changelog_entry) = match max_bump {
            Some(max_bump) => {
                let next = bump::next_version(&old_version, max_bump);
                let entries: Vec<(Bump, &str)> = changes
                    .iter()
                    .filter_map(|change| {
                        change
                            .releases
                            .iter()
                            .find(|(n, _)| n == name)
                            .and_then(|(_, bump)| *bump)
                            .map(|bump| (bump, change.summary.as_str()))
                    })
                    .collect();
                let entry = render_entry(&entries);
                let section = render_section(&next, &entry);

                package_json.set_version(&next)?;
                let changelog_path = member.dir().join("CHANGELOG.md");
                let changelog_text = match fs::read_to_string(&changelog_path) {
                    Ok(text) => text,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
                    Err(err) => return Err(err).context(changelog_path.display().to_string()),
                };
                let new_changelog_text =
                    changelog::upsert_section(&changelog_text, name, &next.to_string(), &section);
                writes.push((package_json, changelog_path, new_changelog_text));
                (next, Some(entry))
            }
            None => (old_version.clone(), None),
        };
        releases.push(Release {
            name: name.to_owned(),
            bump: max_bump.map_or("none", Bump::as_str),
            old_version: old_version.to_string(),
            new_version: new_version.to_string(),
            changesets: ids,
            changelog_entry,
        });
    }

    let plan = ReleasePlan {
        changesets: changes
            .iter()
            .map(|change| ChangesetEntry {
                id: id(change),
                summary: change.summary.clone(),
                releases: change
                    .releases
                    .iter()
                    .map(|(name, bump)| ReleaseRef {
                        name: name.clone(),
                        bump: bump.map_or("none", Bump::as_str),
                    })
                    .collect(),
            })
            .collect(),
        releases,
    };

    if changes.is_empty() {
        eprintln!("note: no changesets found; nothing to do");
    }
    if dry_run {
        eprintln!("dry run: no files will be modified");
    } else {
        for (package_json, changelog_path, new_changelog_text) in writes {
            package_json.save()?;
            fs::write(&changelog_path, new_changelog_text)
                .with_context(|| changelog_path.display().to_string())?;
        }
        for change in &changes {
            let path = changeset_dir.join(&change.file_name);
            fs::remove_file(&path).with_context(|| path.display().to_string())?;
        }
    }

    println!("{}", serde_json::to_string(&plan)?);
    Ok(())
}

fn id(change: &LoadedChange) -> String {
    change
        .file_name
        .strip_suffix(".md")
        .unwrap_or(&change.file_name)
        .to_owned()
}
