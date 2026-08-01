use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use saphyr::{Mapping, Scalar, Yaml, YamlEmitter};
use ulid::Ulid;

use crate::{bump::Bump, package_json::PackageJson};

/// Creates a changeset file under `.changeset/` and prints its path to
/// stdout; the directory must already exist (see `init`). Inputs missing from
/// the flags are prompted for interactively when both stdin and stderr are
/// terminals; otherwise the missing flags are reported as an error.
pub(crate) fn run(bump: Option<Bump>, summary: Option<String>) -> Result<()> {
    let package_json = PackageJson::load(Path::new("."))?;

    let changeset_dir = Path::new(".changeset");
    ensure!(
        changeset_dir.is_dir(),
        "{}: changeset directory not found; run `changesette init` to create it",
        changeset_dir.display()
    );

    if let Some(summary) = &summary {
        ensure!(!summary.trim().is_empty(), "the summary must not be empty");
    }

    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        let mut missing = Vec::new();
        if bump.is_none() {
            missing.push("--bump");
        }
        if summary.is_none() {
            missing.push("--message");
        }
        if !missing.is_empty() {
            bail!(
                "missing required flags in non-interactive mode: {}",
                missing.join(", ")
            );
        }
    }

    let bump = match bump {
        Some(bump) => bump,
        None => prompt_bump()?,
    };
    let summary = match summary {
        Some(summary) => summary,
        None => prompt_summary()?,
    };

    let path = changeset_dir.join(format!("changesette-{}.md", Ulid::generate()));
    let mut mapping = Mapping::new();
    mapping.insert(
        Yaml::Value(Scalar::String(package_json.name().into())),
        Yaml::Value(Scalar::String(bump.as_str().into())),
    );
    let mut frontmatter = String::new();
    YamlEmitter::new(&mut frontmatter).dump(&Yaml::Mapping(mapping))?;
    let content = format!("{}\n---\n\n{}\n", frontmatter, summary.trim());
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .with_context(|| path.display().to_string())?;
    println!("{}", path.display());
    Ok(())
}

fn prompt_bump() -> Result<Bump> {
    const ITEMS: [Bump; 3] = [Bump::Patch, Bump::Minor, Bump::Major];
    let index = dialoguer::Select::new()
        .with_prompt("Bump type")
        .items(ITEMS.map(Bump::as_str))
        .default(0)
        .interact()?;
    Ok(ITEMS[index])
}

fn prompt_summary() -> Result<String> {
    let input: String = dialoguer::Input::new()
        .with_prompt("Summary (leave empty to open your editor)")
        .allow_empty(true)
        .interact_text()?;
    if !input.trim().is_empty() {
        return Ok(input);
    }
    let edited = dialoguer::Editor::new()
        .edit("")
        .context("failed to edit the summary")?;
    match edited {
        Some(text) if !text.trim().is_empty() => Ok(text),
        _ => bail!("the summary must not be empty"),
    }
}
