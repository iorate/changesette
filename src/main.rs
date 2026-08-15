use std::process::ExitCode;

use clap::Parser;

mod bump;
mod changelog;
mod changeset;
mod commands;
mod package_json;
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
    /// The packages to record a major bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    major: Vec<String>,
    /// The packages to record a minor bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    minor: Vec<String>,
    /// The packages to record a patch bump for (comma-separated, repeatable)
    #[arg(long, value_name = "PACKAGES", value_delimiter = ',')]
    patch: Vec<String>,
    /// The summary text of the change
    #[arg(short, long)]
    message: Option<String>,
    /// Create a changeset that names no packages
    #[arg(long, conflicts_with_all = ["major", "minor", "patch"])]
    empty: bool,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create the changeset directory
    Init,
    /// Create a changeset (the default command)
    Add(AddArgs),
    /// Consume changesets: bump each named package's version and update its CHANGELOG.md
    Version {
        /// Print the plan without modifying any file
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Print the workspace packages as JSON
    GetPackages,
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
        Command::Add(AddArgs {
            major,
            minor,
            patch,
            message,
            empty,
        }) => commands::add::run(major, minor, patch, message, empty),
        Command::Version { dry_run } => commands::version::run(dry_run),
        Command::GetPackages => commands::get_packages::run(),
        Command::GetChangelogEntry { package, version } => {
            commands::get_changelog_entry::run(&package, &version)
        }
    }
}
