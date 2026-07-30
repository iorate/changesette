use std::process::ExitCode;

use clap::Parser;

mod bump;
mod changelog;
mod changeset;
mod commands;
mod config;
mod package_json;
mod package_lock;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create a changeset
    Add {
        /// The bump type to record in the changeset
        #[arg(long)]
        bump: Option<bump::Bump>,
        /// The summary text of the change
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Consume changesets: bump the package version and update CHANGELOG.md
    Version,
    /// Print the current version from package.json
    Current,
    /// Print a version section from CHANGELOG.md
    Changelog {
        /// The version whose section to print (defaults to the latest)
        version: Option<String>,
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
    match Cli::parse().command {
        Command::Add { bump, message } => commands::add::run(bump, message),
        Command::Version => commands::version::run(),
        Command::Current => commands::current::run(),
        Command::Changelog { version } => commands::changelog::run(version),
    }
}
