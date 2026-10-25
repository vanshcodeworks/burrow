//! `burrow` command-line interface.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod commandline;
mod commands;
mod container;
mod control;
mod daemon;
mod doctor;
mod init;
mod project;
mod state;
mod top;
mod win;

/// Docker Compose's reproducibility, at a fraction of the resources, native on Windows.
#[derive(Debug, Parser)]
#[command(name = "burrow", version, about, propagate_version = true)]
struct Cli {
    /// Increase verbosity. Repeat for more detail.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    /// Machine-readable output where supported.
    #[arg(long, global = true)]
    json: bool,

    /// Project directory containing burrow.toml. Defaults to the current directory.
    #[arg(long, global = true, value_name = "DIR")]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

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

    /// Restart one service and its dependents.
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
        /// Service whose complete environment should be printed. Without a
        /// service, prints the project's endpoint variables.
        #[arg(value_name = "SERVICE")]
        service: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value = "dotenv", value_name = "FORMAT")]
        format: commands::EnvFormat,
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

    /// Internal: run the per-project daemon.
    #[command(name = "__daemon", hide = true)]
    Daemon {
        /// Services to start.
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum PkgCommand {
    /// List packages in the shared cache.
    List,

    /// Add a package to this project.
    Add {
        /// Package name, optionally with a version (for example `postgres@16`).
        #[arg(value_name = "NAME")]
        name: String,

        /// Service name to create in burrow.toml. Defaults to the package name.
        #[arg(long, value_name = "NAME")]
        service: Option<String>,

        /// Seed script to run once after the service is first healthy.
        #[arg(long, value_name = "PATH")]
        seed: Option<PathBuf>,
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    let options = commands::Options {
        project: cli.project,
        json: cli.json,
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: could not start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(cli.command, options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(exit) = error.downcast_ref::<commands::ExitStatusError>() {
                // Preserve the child's exact status for scripts and CI. Using
                // `exit` here is safe: the runtime has already finished.
                std::process::exit(exit.0);
            }
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("burrow={default},burrow::service=info,warn")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

async fn run(command: Command, options: commands::Options) -> Result<()> {
    match command {
        Command::Init { template } => crate::init::run(&options, template),
        Command::Up {
            services,
            detach,
            yes,
        } => commands::up(&options, services, detach, yes).await,
        Command::Down => commands::down(&options).await,
        Command::Restart { service } => commands::restart(&options, service).await,
        Command::Status => commands::status(&options).await,
        Command::Logs {
            service,
            follow,
            since,
        } => commands::logs(&options, service, follow, since).await,
        Command::Schema => commands::schema(),
        Command::Daemon { services } => commands::daemon_entry(&options, services).await,
        Command::Open { service } => commands::open(&options, service).await,
        Command::Env { service, format } => commands::env(&options, service, format).await,
        Command::Exec { service, cmd } => commands::exec(&options, service, cmd).await,
        Command::Doctor => crate::doctor::run(&options).await,
        Command::Top => crate::top::run(&options).await,
        Command::Pkg { command } => match command {
            Some(command) => run_pkg(command, &options).await,
            None => commands::pkg_list(&options),
        },
        Command::Snapshot { name } => commands::snapshot(&options, name).await,
        Command::Restore { name } => commands::restore(&options, name).await,
        Command::Snapshots => commands::snapshots(&options),
        Command::Reset { service } => commands::reset(&options, service).await,
    }
}

async fn run_pkg(command: PkgCommand, options: &commands::Options) -> Result<()> {
    match command {
        PkgCommand::List => commands::pkg_list(options),
        PkgCommand::Add {
            name,
            service,
            seed,
        } => commands::pkg_add(options, name, service, seed).await,
        PkgCommand::Remove { name } => commands::pkg_remove(options, name).await,
        PkgCommand::Gc => commands::pkg_gc(options),
        PkgCommand::Verify => commands::pkg_verify(options),
    }
}
