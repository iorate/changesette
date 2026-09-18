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
    bump::Bump,
    changeset,
    workspace::{PackageNotFound, Versioned, Workspace},
};

pub struct AddArgs {
    pub empty: bool,
    pub open: bool,
    pub message: Option<String>,
    pub major: Vec<String>,
    pub minor: Vec<String>,
    pub patch: Vec<String>,
}

pub fn run(workspace: &Workspace, args: AddArgs) -> Result<()> {
    ensure!(
        !args.open || (io::stdin().is_terminal() && io::stderr().is_terminal()),
        "cannot use --open in non-interactive mode"
    );

    let changeset_dir = workspace.changeset_dir();
    let mut versioned: Vec<Versioned> = workspace.versioned().collect();
    versioned.sort_by_key(Versioned::name);
    if versioned.is_empty() {
        let skipped: Vec<String> = workspace
            .packages()
            .filter_map(|package| {
                package
                    .skip_reason()
                    .map(|reason| format!("{package}: {reason}"))
            })
            .collect();
        if skipped.is_empty() {
            bail!("no packages to version");
        }
        bail!(
            "no packages to version; every package is skipped ({})",
            skipped.join(", ")
        );
    }
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
            releases_from_flags(workspace, &args.major, &args.minor, &args.patch)?
        } else {
            let Some(releases) = prompt_releases(&versioned)? else {
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

pub type Releases = Vec<(String, Option<Bump>)>;

pub fn releases_from_flags(
    workspace: &Workspace,
    major: &[String],
    minor: &[String],
    patch: &[String],
) -> Result<Releases> {
    let flags = [
        ("--major", Bump::Major, major),
        ("--minor", Bump::Minor, minor),
        ("--patch", Bump::Patch, patch),
    ];

    let mut flags_by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (flag, _, names) in &flags {
        for name in *names {
            match workspace.package(name) {
                None => bail!("`{flag}`: {}", PackageNotFound::new(name, workspace)),
                Some(package) => {
                    if let Some(reason) = package.skip_reason() {
                        bail!("`{flag}`: package `{name}` is skipped: {reason}");
                    }
                }
            }
            let entry = flags_by_name.entry(name).or_default();
            if !entry.contains(flag) {
                entry.push(flag);
            }
        }
    }
    for (name, name_flags) in &flags_by_name {
        ensure!(
            name_flags.len() == 1,
            "the package `{name}` is passed to multiple bump type flags: {}",
            name_flags.join(", ")
        );
    }

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

fn prompt_releases(versioned: &[Versioned]) -> Result<Option<Releases>> {
    if let [versioned] = versioned {
        const ITEMS: [Bump; 3] = [Bump::Patch, Bump::Minor, Bump::Major];
        let prompt = format!(
            "What kind of change is this for {}? (current version is {})",
            versioned.name(),
            versioned.version()
        );
        let Some(option) =
            cancel_to_none(Select::new(&prompt, ITEMS.map(Bump::as_str).to_vec()).raw_prompt())?
        else {
            return Ok(None);
        };
        return Ok(Some(vec![(
            versioned.name().to_owned(),
            Some(ITEMS[option.index]),
        )]));
    }

    let names: Vec<&str> = versioned.iter().map(Versioned::name).collect();
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
    let affected: Vec<Versioned> = selected
        .into_iter()
        .map(|option| versioned[option.index])
        .collect();

    let labels: Vec<String> = affected
        .iter()
        .map(|versioned| format!("{}@{}", versioned.name(), versioned.version()))
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
