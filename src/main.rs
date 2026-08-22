use std::{path::PathBuf, process::ExitCode};

use clap::Parser;

mod bump;
mod changelog;
mod changeset;
mod commands;
mod config;
mod jsonc;
mod output;
mod package_json;
mod plan;
mod pre;
mod release_plan;
mod skip;
mod snapshot;
mod workspace;

#[derive(Parser)]
#[command(version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    add: AddArgs,
}

#[derive(clap::Args)]
struct AddArgs {
    /// Create a changeset that names no packages
    #[arg(long, conflicts_with_all = ["major", "minor", "patch"])]
    empty: bool,
    /// Open the created changeset in your editor
    #[arg(long)]
    open: bool,
    /// The summary text of the change
    #[arg(short, long)]
    message: Option<String>,
    /// The packages to record a major bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    major: Vec<String>,
    /// The packages to record a minor bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    minor: Vec<String>,
    /// The packages to record a patch bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    patch: Vec<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create the changeset directory
    Init,
    /// Create a changeset (the default command)
    Add(AddArgs),
    /// Consume changesets: bump each named package's version and update its CHANGELOG.md
    Version {
        /// The packages to skip, leaving their changesets in place (comma-separated, repeatable)
        #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
        ignore: Vec<String>,
        /// Create a snapshot release: bump to throwaway `0.0.0-<suffix>` versions instead
        #[arg(
            long,
            value_name = "TAG",
            num_args = 0..=1,
            value_parser = clap::builder::NonEmptyStringValueParser::new()
        )]
        snapshot: Option<Option<String>>,
        /// The snapshot suffix template; the placeholders are {tag}, {timestamp}, and {datetime}
        #[arg(
            long,
            value_name = "TEMPLATE",
            requires = "snapshot",
            value_parser = clap::builder::NonEmptyStringValueParser::new()
        )]
        snapshot_prerelease_template: Option<String>,
        /// Succeed even when there are no unreleased changesets
        #[arg(short, long)]
        allow_no_changesets: bool,
        /// Write the release plan to the file (or stdout with `-`) as JSON
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Enter or exit pre-release mode
    Pre {
        #[command(subcommand)]
        command: PreCommand,
    },
    /// Print the packages to be bumped by `version`
    Status {
        /// Show the new versions and the changeset files
        #[arg(short, long)]
        verbose: bool,
        /// Write the release plan to the file (or stdout with `-`) as JSON instead
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Print the workspace packages as JSON
    GetPackages {
        /// List every workspace member, including the packages `version` skips
        #[arg(long)]
        all: bool,
    },
    /// Print a version section from a package's CHANGELOG.md
    GetChangelogEntry {
        /// The name of the package
        package: String,
        /// The version whose section to print
        version: semver::Version,
    },
}

#[derive(clap::Subcommand)]
enum PreCommand {
    /// Enter pre-release mode: `version` will bump to `-<tag>.<n>` prerelease versions
    Enter {
        /// The prerelease tag to use (the `beta` of `1.1.0-beta.0`)
        tag: String,
    },
    /// Exit pre-release mode: the next `version` will bump to final versions
    Exit,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Add(cli.add)) {
        Command::Init => commands::init::run(),
        Command::Add(AddArgs {
            major,
            minor,
            patch,
            message,
            empty,
            open,
        }) => commands::add::run(major, minor, patch, message, empty, open),
        Command::Version {
            ignore,
            snapshot,
            snapshot_prerelease_template,
            allow_no_changesets,
            output,
        } => {
            let snapshot = snapshot.map(|tag| snapshot::Snapshot {
                tag,
                template: snapshot_prerelease_template,
            });
            commands::version::run(
                &ignore,
                allow_no_changesets,
                output.as_deref(),
                snapshot.as_ref(),
            )
        }
        Command::Pre { command } => match command {
            PreCommand::Enter { tag } => commands::pre::enter(&tag),
            PreCommand::Exit => commands::pre::exit(),
        },
        Command::Status { verbose, output } => commands::status::run(verbose, output.as_deref()),
        Command::GetPackages { all } => commands::get_packages::run(all),
        Command::GetChangelogEntry { package, version } => {
            commands::get_changelog_entry::run(&package, &version)
        }
    }
}
