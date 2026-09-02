use std::{
    collections::BTreeMap,
    env,
    fmt::Write as _,
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    process,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use inquire::{InquireError, MultiSelect, Select, Text, validator::MinLengthValidator};
use tracing::info;

use crate::{
    AddArgs,
    bump::Bump,
    changeset,
    config::Config,
    skip::SkipSet,
    workspace::{Member, Workspace},
};

pub(crate) fn run(workspace: &Workspace, config: &Config, args: AddArgs) -> Result<()> {
    ensure!(
        !args.open || (io::stdin().is_terminal() && io::stderr().is_terminal()),
        "cannot use --open in non-interactive mode"
    );

    ensure!(
        !workspace.members().is_empty(),
        "no packages found in the workspace"
    );

    let changeset_dir = workspace.changeset_dir();
    let skip = SkipSet::load(workspace, config, &[])?;
    let packages: Vec<&Member> = workspace
        .members()
        .iter()
        .filter(|member| !skip.contains(member.name()))
        .collect();
    ensure!(
        !packages.is_empty(),
        "no versionable packages found; ensure the packages are not private or ignored and have a version field in package.json"
    );
    fs::create_dir_all(&changeset_dir).with_context(|| changeset_dir.display().to_string())?;

    let (releases, summary) = if args.empty {
        (Vec::new(), args.message.unwrap_or_default())
    } else {
        let flags_given =
            !(args.major.is_empty() && args.minor.is_empty() && args.patch.is_empty());
        if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
            let mut missing = Vec::new();
            if !flags_given {
                missing.push("--major/--minor/--patch");
            }
            if args.message.is_none() {
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
            releases_from_flags(workspace, &packages, &args.major, &args.minor, &args.patch)?
        } else {
            let Some(releases) = prompt_releases(&packages)? else {
                info!("Cancelled");
                return Ok(());
            };
            releases
        };
        let summary = if let Some(message) = args.message {
            message
        } else {
            let Some(summary) = prompt_summary()? else {
                info!("Cancelled");
                return Ok(());
            };
            summary
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
    let content = changeset::render(&releases, &summary)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .with_context(|| path.display().to_string())?;

    if !args.empty {
        let mut confirmation = String::from("Summary of changesets:");
        for bump in [Bump::Major, Bump::Minor, Bump::Patch] {
            let names: Vec<&str> = releases
                .iter()
                .filter(|(_, b)| *b == Some(bump))
                .map(|(name, _)| name.as_str())
                .collect();
            if !names.is_empty() {
                let _ = write!(confirmation, "\n{}:  {}", bump.as_str(), names.join(", "));
            }
        }
        info!("{confirmation}");
    }

    info!("Added {}", path.display());

    if args.open {
        open_editor(&path)?;
    }
    Ok(())
}

fn open_editor(path: &Path) -> Result<()> {
    let editor = env::var_os("VISUAL")
        .or_else(|| env::var_os("EDITOR"))
        .unwrap_or_else(|| if cfg!(windows) { "notepad.exe" } else { "vi" }.into());
    #[expect(clippy::unnecessary_debug_formatting)]
    let editor = editor
        .into_string()
        .map_err(|editor| anyhow!("the editor command is not valid UTF-8: {editor:?}"))?;
    let (command, args) = match shell_words::split(&editor) {
        Ok(mut parts) if !parts.is_empty() => (parts.remove(0), parts),
        _ => (editor, Vec::new()),
    };
    process::Command::new(&command)
        .args(args)
        .arg(path)
        .spawn()
        .and_then(|mut child| child.wait())
        .with_context(|| format!("failed to open the editor `{command}`"))?;
    Ok(())
}

type Releases = Vec<(String, Option<Bump>)>;

fn releases_from_flags(
    workspace: &Workspace,
    packages: &[&Member],
    major: &[String],
    minor: &[String],
    patch: &[String],
) -> Result<Releases> {
    let flags = [
        ("--major", Bump::Major, major),
        ("--minor", Bump::Minor, minor),
        ("--patch", Bump::Patch, patch),
    ];

    let mut errors = Vec::new();
    let mut flags_by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (flag, _, names) in &flags {
        for name in *names {
            if workspace.member(name).is_err() {
                errors.push(format!(
                    "the package `{name}` is passed to `{flag}` but is not a workspace member"
                ));
            } else if !packages.iter().any(|member| member.name() == name) {
                errors.push(format!(
                    "the package `{name}` is passed to `{flag}` but is skipped (private, ignored, or without a version)"
                ));
            }
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

    let mut releases = Releases::new();
    for (_, bump, names) in flags {
        for name in names {
            if !releases.iter().any(|(n, _)| n == name) {
                releases.push((name.clone(), Some(bump)));
            }
        }
    }
    Ok(releases)
}

fn cancel_to_none<T>(result: Result<T, InquireError>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(InquireError::OperationCanceled) => Ok(None),
        Err(InquireError::OperationInterrupted) => {
            // Unlike on Esc, inquire does not clean up the prompt frame on
            // Ctrl-C: it stays on screen with the cursor on or right below its
            // last line, so move to a fresh line to avoid overwriting it.
            eprintln!();
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

fn prompt_releases(packages: &[&Member]) -> Result<Option<Releases>> {
    if let [member] = packages {
        const ITEMS: [Bump; 3] = [Bump::Patch, Bump::Minor, Bump::Major];
        let prompt = format!(
            "What kind of change is this for {}? (current version is {})",
            member.name(),
            member.version()
        );
        let Some(option) =
            cancel_to_none(Select::new(&prompt, ITEMS.map(Bump::as_str).to_vec()).raw_prompt())?
        else {
            return Ok(None);
        };
        return Ok(Some(vec![(
            member.name().to_owned(),
            Some(ITEMS[option.index]),
        )]));
    }

    let names: Vec<&str> = packages.iter().map(|member| member.name()).collect();
    let Some(selected) = cancel_to_none(
        MultiSelect::new(
            "Which packages were affected by the changes you made?",
            names,
        )
        .with_validator(
            MinLengthValidator::new(1).with_message("You must select at least one package"),
        )
        .raw_prompt(),
    )?
    else {
        return Ok(None);
    };
    let affected: Vec<&Member> = selected
        .into_iter()
        .map(|option| packages[option.index])
        .collect();

    let labels: Vec<String> = affected
        .iter()
        .map(|member| format!("{}@{}", member.name(), member.version()))
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
        let Some(selected) = cancel_to_none(MultiSelect::new(prompt, items).raw_prompt())? else {
            return Ok(None);
        };
        let bumped: Vec<usize> = selected
            .iter()
            .map(|option| remaining[option.index])
            .collect();
        remaining.retain(|i| !bumped.contains(i));
        releases.extend(
            bumped
                .into_iter()
                .map(|i| (affected[i].name().to_owned(), Some(bump))),
        );
    }
    if !remaining.is_empty() {
        info!(
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
                .map(|i| (affected[i].name().to_owned(), Some(Bump::Patch))),
        );
    }
    Ok(Some(releases))
}

fn prompt_summary() -> Result<Option<String>> {
    let Some(input) = cancel_to_none(
        Text::new("Please enter a summary for this change (leave empty to open your editor)")
            .prompt(),
    )?
    else {
        return Ok(None);
    };
    if !input.trim().is_empty() {
        return Ok(Some(input));
    }
    let edited = edit_summary()?;
    if !edited.is_empty() {
        return Ok(Some(edited));
    }
    loop {
        let Some(input) = cancel_to_none(
            Text::new("Did not find a summary in the edited file. Please enter one").prompt(),
        )?
        else {
            return Ok(None);
        };
        if !input.trim().is_empty() {
            return Ok(Some(input));
        }
    }
}

fn edit_summary() -> Result<String> {
    let mut file = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()
        .context("failed to create a temporary file for the summary")?;
    file.write_all(
        b"\n\n# Please enter a summary for your changes.\n# An empty message aborts the editor.",
    )
    .context("failed to write the summary template")?;
    let path = file.into_temp_path();
    open_editor(&path)?;
    let text = fs::read_to_string(&path).context("failed to read the edited summary")?;
    let text = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text.trim().to_owned())
}
