use std::{path::PathBuf, process::ExitCode};

use clap::Parser;

mod bump;
mod changelog;
mod changeset;
mod commands;
mod jsonc;
mod output;
mod package_json;
mod plan;
mod pre;
mod release_plan;
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
        /// Write the release plan to the file (or stdout with `-`) as pretty-printed JSON
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
        /// Write the release plan to the file (or stdout with `-`) as pretty-printed JSON instead
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Print the workspace packages as JSON
    GetPackages,
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
        }) => commands::add::run(major, minor, patch, message, empty),
        Command::Version { ignore, output } => commands::version::run(&ignore, output.as_deref()),
        Command::Pre { command } => match command {
            PreCommand::Enter { tag } => commands::pre::enter(&tag),
            PreCommand::Exit => commands::pre::exit(),
        },
        Command::Status { verbose, output } => commands::status::run(verbose, output.as_deref()),
        Command::GetPackages => commands::get_packages::run(),
        Command::GetChangelogEntry { package, version } => {
            commands::get_changelog_entry::run(&package, &version)
        }
    }
}
