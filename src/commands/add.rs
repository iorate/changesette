use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, IsTerminal, Write},
};

use anyhow::{Context, Result, bail, ensure};
use saphyr::{Mapping, Scalar, Yaml, YamlEmitter};

use crate::{bump::Bump, config, output, package_json::PackageJson, skip, workspace::Workspace};

/// Creates a changeset file under the workspace root's `.changeset/`,
/// creating the directory if needed, and reports the file to stderr.
/// With `empty`, the changeset names no packages and nothing is prompted.
/// Otherwise only the versionable packages — the ones `version` does not
/// skip — may be named: the releases come from the bump flags when any is
/// given, and the summary from `message`; inputs missing from the flags are
/// prompted for interactively when both stdin and stderr are terminals, and
/// reported as an error otherwise.
pub(crate) fn run(
    major: Vec<String>,
    minor: Vec<String>,
    patch: Vec<String>,
    message: Option<String>,
    empty: bool,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let workspace = Workspace::discover(&cwd)?;
    ensure!(
        !workspace.members().is_empty(),
        "no packages found in the workspace"
    );

    let changeset_dir = workspace.root().join(".changeset");
    let config = config::load(&changeset_dir)?;
    fs::create_dir_all(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;

    let (releases, summary) = if empty {
        (Vec::new(), message.unwrap_or_default())
    } else {
        let mut packages = Vec::new();
        for member in workspace.members() {
            let package_json = PackageJson::load(member.dir())?;
            if !skip::should_skip(&package_json, &config, &[]) {
                packages.push(package_json);
            }
        }
        ensure!(
            !packages.is_empty(),
            "no versionable packages found; ensure the packages are not private or ignored and have a version field in package.json"
        );

        let flags_given = !(major.is_empty() && minor.is_empty() && patch.is_empty());
        if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
            let mut missing = Vec::new();
            if !flags_given {
                missing.push("--major/--minor/--patch");
            }
            if message.is_none() {
                missing.push("--message");
            }
            if !missing.is_empty() {
                bail!(
                    "missing required flags in non-interactive mode: {}",
                    missing.join(", ")
                );
            }
        }
        let releases = if flags_given {
            releases_from_flags(&workspace, &packages, &major, &minor, &patch)?
        } else {
            prompt_releases(&packages)?
        };
        let summary = match message {
            Some(message) => message,
            None => prompt_summary()?,
        };
        (releases, summary)
    };

    let file_name = format!(
        "{}.md",
        petname::Petnames::small()
            .namer(3, "-")
            .iter(&mut rand::rng())
            .next()
            .context("failed to generate a changeset name")?
    );
    let path = changeset_dir.join(&file_name);
    let content = render(&releases, &summary)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .with_context(|| path.display().to_string())?;

    if !empty {
        let mut confirmation = String::from("Summary of changesets:");
        for bump in [Bump::Major, Bump::Minor, Bump::Patch] {
            let names: Vec<&str> = releases
                .iter()
                .filter(|(_, b)| *b == bump)
                .map(|(name, _)| name.as_str())
                .collect();
            if !names.is_empty() {
                confirmation.push_str(&format!("\n{}:  {}", bump.as_str(), names.join(", ")));
            }
        }
        output::eprint_line(&confirmation)?;
    }

    output::eprint_line(&format!(
        "Added {}",
        workspace.display_path(&cwd, &path).display()
    ))?;
    Ok(())
}

fn releases_from_flags(
    workspace: &Workspace,
    packages: &[PackageJson],
    major: &[String],
    minor: &[String],
    patch: &[String],
) -> Result<Vec<(String, Bump)>> {
    let flags = [
        ("--major", Bump::Major, major),
        ("--minor", Bump::Minor, minor),
        ("--patch", Bump::Patch, patch),
    ];

    let mut errors = Vec::new();
    for (flag, _, names) in &flags {
        for name in *names {
            if workspace.member(name).is_err() {
                errors.push(format!(
                    "the package `{name}` is passed to `{flag}` but is not a workspace member"
                ));
            } else if !packages
                .iter()
                .any(|package_json| package_json.name() == name)
            {
                errors.push(format!(
                    "the package `{name}` is passed to `{flag}` but is skipped (private, ignored, or without a version)"
                ));
            }
        }
    }
    let mut flags_by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (flag, _, names) in &flags {
        for name in *names {
            let entry = flags_by_name.entry(name).or_default();
            if !entry.contains(flag) {
                entry.push(flag);
            }
        }
    }
    for (name, name_flags) in &flags_by_name {
        if name_flags.len() > 1 {
            errors.push(format!(
                "the package `{name}` is passed to multiple bump type flags: {}",
                name_flags.join(", ")
            ));
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));

    let mut releases: Vec<(String, Bump)> = Vec::new();
    for (_, bump, names) in flags {
        for name in names {
            if !releases.iter().any(|(n, _)| n == name) {
                releases.push((name.clone(), bump));
            }
        }
    }
    Ok(releases)
}

fn prompt_releases(packages: &[PackageJson]) -> Result<Vec<(String, Bump)>> {
    if let [package_json] = packages {
        const ITEMS: [Bump; 3] = [Bump::Patch, Bump::Minor, Bump::Major];
        let prompt = match package_json.version() {
            Some(version) => format!(
                "What kind of change is this for {}? (current version is {version})",
                package_json.name()
            ),
            None => format!("What kind of change is this for {}?", package_json.name()),
        };
        let index = dialoguer::Select::new()
            .with_prompt(prompt)
            .items(ITEMS.map(Bump::as_str))
            .default(0)
            .interact()?;
        return Ok(vec![(package_json.name().to_owned(), ITEMS[index])]);
    }

    let names: Vec<&str> = packages.iter().map(PackageJson::name).collect();
    let affected: Vec<&PackageJson> = loop {
        let indexes = dialoguer::MultiSelect::new()
            .with_prompt("Which packages were affected by the changes you made?")
            .items(&names)
            .interact()?;
        if indexes.is_empty() {
            eprintln!("You must select at least one package");
            continue;
        }
        break indexes.into_iter().map(|index| &packages[index]).collect();
    };

    let labels: Vec<String> = affected
        .iter()
        .map(|package_json| match package_json.version() {
            Some(version) => format!("{}@{version}", package_json.name()),
            None => package_json.name().to_owned(),
        })
        .collect();

    let mut releases = Vec::new();
    let mut remaining: Vec<usize> = (0..affected.len()).collect();
    for (bump, prompt) in [
        (Bump::Major, "Which packages should have a major bump?"),
        (Bump::Minor, "Which packages should have a minor bump?"),
    ] {
        if remaining.is_empty() {
            break;
        }
        let items: Vec<&str> = remaining.iter().map(|&i| labels[i].as_str()).collect();
        let selected = dialoguer::MultiSelect::new()
            .with_prompt(prompt)
            .items(&items)
            .interact()?;
        let bumped: Vec<usize> = selected.iter().map(|&s| remaining[s]).collect();
        remaining.retain(|i| !bumped.contains(i));
        releases.extend(
            bumped
                .into_iter()
                .map(|i| (affected[i].name().to_owned(), bump)),
        );
    }
    if !remaining.is_empty() {
        eprintln!(
            "The following packages will be patch bumped:\n{}",
            remaining
                .iter()
                .map(|&i| labels[i].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        releases.extend(
            remaining
                .into_iter()
                .map(|i| (affected[i].name().to_owned(), Bump::Patch)),
        );
    }
    Ok(releases)
}

fn prompt_summary() -> Result<String> {
    let input: String = dialoguer::Input::new()
        .with_prompt("Please enter a summary for this change (leave empty to open your editor)")
        .allow_empty(true)
        .interact_text()?;
    if !input.trim().is_empty() {
        return Ok(input);
    }
    let edited = dialoguer::Editor::new()
        .edit(
            "\n\n# Please enter a summary for your changes.\n# An empty message aborts the editor.",
        )
        .context("failed to edit the summary")?;
    if let Some(text) = edited {
        let text = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let text = text.trim();
        if !text.is_empty() {
            return Ok(text.to_owned());
        }
    }
    loop {
        let input: String = dialoguer::Input::new()
            .with_prompt("Did not find a summary in the edited file. Please enter one")
            .allow_empty(true)
            .interact_text()?;
        if !input.trim().is_empty() {
            return Ok(input);
        }
    }
}

fn render(releases: &[(String, Bump)], summary: &str) -> Result<String> {
    let summary = summary.trim();
    let mut content = if releases.is_empty() {
        String::from("---\n---\n")
    } else {
        let mut mapping = Mapping::new();
        for (name, bump) in releases {
            mapping.insert(
                Yaml::Value(Scalar::String(name.as_str().into())),
                Yaml::Value(Scalar::String(bump.as_str().into())),
            );
        }
        let mut frontmatter = String::new();
        YamlEmitter::new(&mut frontmatter).dump(&Yaml::Mapping(mapping))?;
        format!("{frontmatter}\n---\n")
    };
    if !summary.is_empty() {
        content.push('\n');
        content.push_str(summary);
        content.push('\n');
    }
    Ok(content)
}
