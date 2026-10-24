//! `burrow` command-line interface.
//!
//! M1 provides the full command surface and a clean "not implemented" error
//! for every command whose handler arrives in a later milestone. The CLI is
//! synchronous in M1; async configuration loading and supervision arrive in
//! M2.

use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};

/// Docker Compose's reproducibility, at a fraction of the resources, native on Windows.
#[derive(Debug, Parser)]
#[command(name = "burrow", version, about, propagate_version = true)]
struct Cli {
    /// Increase verbosity. Repeat for more detail.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[arg(long, global = true, help = "machine-readable output where supported")]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

// The arguments are parsed now so `--help` documents the planned surface.
// Their handlers arrive in later milestones; remove this allowance as each
// command is implemented.
#[allow(dead_code)]
#[derive(Debug, Subcommand)]
enum Command {
    /// Scaffold a `burrow.toml` and a project layout.
    Init {
        /// Start from a named template.
        #[arg(long, value_name = "NAME")]
        template: Option<String>,
    },

    /// Start all services, or only the named services.
    Up {
        /// Services to start. Defaults to every service in the project.
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Start in the background instead of streaming logs.
        #[arg(long)]
        detach: bool,

        /// Skip the trust confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Stop all services for the current project.
    Down,

    /// Restart one service.
    Restart {
        /// Service to restart.
        #[arg(value_name = "SERVICE")]
        service: String,
    },

    /// Show service state, ports, memory, CPU, and uptime.
    Status,

    /// Print service logs.
    Logs {
        /// Show logs for one service. Defaults to all services.
        #[arg(value_name = "SERVICE")]
        service: Option<String>,

        /// Follow the log stream.
        #[arg(short, long)]
        follow: bool,

        /// Show entries since a duration (for example `10m` or `1h`).
        #[arg(long, value_name = "DURATION")]
        since: Option<String>,
    },

    /// Open a routed service URL in the default browser.
    Open {
        /// Service to open. Defaults to the project's main route.
        #[arg(value_name = "SERVICE")]
        service: Option<String>,
    },

    /// Print the resolved environment for the project.
    Env {
        /// Output format.
        #[arg(long, value_enum, default_value = "dotenv", value_name = "FORMAT")]
        format: EnvFormat,
    },

    /// Run a command with a service's resolved environment.
    Exec {
        /// Service whose environment should be used.
        #[arg(value_name = "SERVICE")]
        service: String,

        /// Command and arguments to run.
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<OsString>,
    },

    /// Diagnose common local environment problems.
    Doctor,

    /// Open the live terminal dashboard.
    Top,

    /// Manage cached packages and registry entries.
    Pkg {
        #[command(subcommand)]
        command: Option<PkgCommand>,
    },

    /// Create a named snapshot of a service's data.
    Snapshot {
        /// Snapshot name.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Restore a named snapshot.
    Restore {
        /// Snapshot name.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// List snapshots for this project.
    Snapshots,

    /// Reset one service's data directory.
    Reset {
        /// Service to reset.
        #[arg(value_name = "SERVICE")]
        service: String,
    },

    /// Write the `burrow.toml` JSON Schema to stdout.
    Schema,
}

// The names are parsed now so `--help` documents the planned surface. Remove
// this allowance when the package commands are implemented in M3.
#[allow(dead_code)]
#[derive(Debug, Subcommand)]
enum PkgCommand {
    /// List packages in the shared cache.
    List,

    /// Add a package to this project.
    Add {
        /// Package name, optionally with a version (for example `postgres@16`).
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Remove a package from this project.
    Remove {
        /// Package name.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Delete cached packages that are no longer referenced.
    Gc,

    /// Verify cached package hashes.
    Verify,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EnvFormat {
    /// `KEY=value` lines suitable for dotenv files.
    Dotenv,
    /// A JSON object of environment variables.
    Json,
    /// PowerShell `$env:KEY = 'value'` assignments.
    Powershell,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Global flags are accepted in M1; their output modes are wired up with
    // the first command handlers in later milestones.
    let _global_options = (cli.verbose, cli.json);

    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Init { .. } => Err(not_implemented("init", "M5")),
        Command::Up { .. } => Err(not_implemented("up", "M2")),
        Command::Down => Err(not_implemented("down", "M2")),
        Command::Restart { .. } => Err(not_implemented("restart", "M2")),
        Command::Status => Err(not_implemented("status", "M2")),
        Command::Logs { .. } => Err(not_implemented("logs", "M2")),
        Command::Open { .. } => Err(not_implemented("open", "M4")),
        Command::Env { .. } => Err(not_implemented("env", "M4")),
        Command::Exec { .. } => Err(not_implemented("exec", "M4")),
        Command::Doctor => Err(not_implemented("doctor", "M5")),
        Command::Top => Err(not_implemented("top", "M5")),
        Command::Pkg { command } => match command {
            Some(command) => run_pkg(command),
            None => Err(not_implemented("pkg", "M3")),
        },
        Command::Snapshot { .. } => Err(not_implemented("snapshot", "M6")),
        Command::Restore { .. } => Err(not_implemented("restore", "M6")),
        Command::Snapshots => Err(not_implemented("snapshots", "M6")),
        Command::Reset { .. } => Err(not_implemented("reset", "M6")),
        Command::Schema => Err(not_implemented("schema", "M5")),
    }
}

fn run_pkg(command: PkgCommand) -> Result<()> {
    match command {
        PkgCommand::List => Err(not_implemented("pkg list", "M3")),
        PkgCommand::Add { .. } => Err(not_implemented("pkg add", "M3")),
        PkgCommand::Remove { .. } => Err(not_implemented("pkg remove", "M3")),
        PkgCommand::Gc => Err(not_implemented("pkg gc", "M3")),
        PkgCommand::Verify => Err(not_implemented("pkg verify", "M3")),
    }
}

/// Builds the standard error for a command that arrives in a later milestone.
fn not_implemented(command: &str, milestone: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "`burrow {command}` is not implemented yet (planned for {milestone})\n\
         note: see docs/roadmap.md for the milestone plan"
    )
}
