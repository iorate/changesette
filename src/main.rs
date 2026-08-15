use std::process::ExitCode;

use clap::Parser;

mod bump;
mod changelog;
mod changeset;
mod commands;
mod package_json;
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
    /// The bump type to record in the changeset
    #[arg(short, long)]
    bump: Option<bump::Bump>,
    /// The summary text of the change
    #[arg(short, long)]
    message: Option<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create the changeset directory
    Init,
    /// Create a changeset (the default command)
    Add(AddArgs),
    /// Consume changesets: bump the package version and update CHANGELOG.md
    Version {
        /// Print the plan to stderr instead of modifying any file
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Print a package's version from its package.json
    GetVersion {
        /// The name of the package
        package: String,
    },
    /// Print a version section from a package's CHANGELOG.md
    GetChangelogEntry {
        /// The name of the package
        package: String,
        /// The version whose section to print
        version: String,
    },
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
        Command::Add(AddArgs { bump, message }) => commands::add::run(bump, message),
        Command::Version { dry_run } => commands::version::run(dry_run),
        Command::GetVersion { package } => commands::get_version::run(&package),
        Command::GetChangelogEntry { package, version } => {
            commands::get_changelog_entry::run(&package, &version)
        }
    }
}
