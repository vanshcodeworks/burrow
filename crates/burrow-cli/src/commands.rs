//! Implementations of the CLI commands that exist in M2.
//!
//! Commands never print secrets except through the explicit `burrow env`
//! output, and every path that starts a long-running process keeps its
//! standard handles out of the daemon.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use burrow_pkg::{Cache, DownloadProgress, Registry};
use clap::ValueEnum;
use toml_edit::{DocumentMut, Item, Table};

use crate::control::{self, ControlCommand, LogEntry};
use crate::daemon;
use crate::project::{self, Project};
use crate::state::{format_bytes, format_uptime, unix_now, RuntimeState, ServiceStatus};
use crate::win;

/// Global options shared by commands.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Explicit project directory.
    pub project: Option<PathBuf>,
    /// Machine-readable output.
    pub json: bool,
}

/// Output formats for `burrow env`.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum EnvFormat {
    /// `KEY=value` lines suitable for dotenv files.
    Dotenv,
    /// A JSON object of environment variables.
    Json,
    /// PowerShell `$env:KEY = 'value'` assignments.
    Powershell,
}

/// The exit status of a child started by `burrow exec`.
///
/// `main` recognizes this type and exits with the child's own code, so scripts
/// and CI pipelines see the real result instead of a generic failure.
#[derive(Debug)]
pub struct ExitStatusError(pub i32);

impl std::fmt::Display for ExitStatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the command exited with code {}", self.0)
    }
}

impl std::error::Error for ExitStatusError {}

/// Implements `burrow up`.
pub async fn up(options: &Options, services: Vec<String>, detach: bool, yes: bool) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    project::ensure_trust(&project, yes)?;

    for (name, service) in &project.config.services {
        if service.expose {
            eprintln!(
                "warning: service `{name}` sets expose = true; it may accept connections from other machines"
            );
            eprintln!(
                "  note: Burrow does not change how your command binds. Make sure it listens only on networks you trust."
            );
        }
    }

    if let Some(state) = control::load_live_state(&project.dir).await? {
        if state.ready {
            bail!(
                "Burrow is already running for project `{}`\n\
                 help: use `burrow status` or `burrow down`",
                state.project
            );
        }
        bail!(
            "Burrow is already starting for project `{}`\n\
             help: wait for `burrow status` to report healthy, or run `burrow down`",
            state.project
        );
    }

    // Warm the shared cache before detaching so download progress is visible
    // here. The daemon repeats the check; a warm cache never touches the
    // network. `--json` stays machine-readable, so progress goes to stderr
    // only in human mode.
    if options.json {
        project::warm_packages(&project, &services, &mut |_, _| {}).await?;
    } else {
        let mut progress = ProgressPrinter::new();
        project::warm_packages(&project, &services, &mut |name, update| {
            progress.update(name, update);
        })
        .await?;
        progress.finish();
    }

    if detach {
        detach_daemon(&project, &services, options.json).await
    } else {
        daemon::run(project, services).await
    }
}

/// Implements the hidden `__daemon` entry point.
pub async fn daemon_entry(options: &Options, services: Vec<String>) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    if !project::is_trusted(&project) {
        bail!(
            "this project has not been approved\n\
             help: run `burrow up` to review and approve it"
        );
    }
    daemon::run(project, services).await
}

async fn detach_daemon(project: &Project, services: &[String], json: bool) -> Result<()> {
    crate::win::make_standard_handles_non_inheritable();
    let executable = std::env::current_exe()
        .context("could not find the Burrow executable to start the daemon")?;
    let log_path = project.burrow_dir.join("daemon.log");
    std::fs::create_dir_all(&project.burrow_dir)
        .with_context(|| format!("could not create {}", project.burrow_dir.display()))?;
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("could not open {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("could not open {}", log_path.display()))?;

    let mut command = std::process::Command::new(executable);
    command
        .arg("__daemon")
        .args(services)
        .current_dir(&project.dir)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        command.creation_flags(
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        );
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(first_error) => {
            // Some Job Objects forbid breakaway. Retry without it so Burrow
            // still works inside constrained CI runners.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            command.spawn().with_context(|| {
                format!(
                    "could not start the Burrow daemon ({first_error})\n\
                     help: run `burrow up` without --detach to see logs"
                )
            })?
        }
    };

    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("could not check the daemon")? {
            let tail = read_log_tail(&log_path, 30);
            bail!("the Burrow daemon exited during startup with {status}\n{tail}");
        }

        if let Some(state) = RuntimeState::read(&project.dir)? {
            if state.ready && control::load_live_state(&project.dir).await?.is_some() {
                if json {
                    println!("{}", serde_json::to_string_pretty(&state.public_json())?);
                } else {
                    println!(
                        "Burrow is running in the background for `{}`",
                        state.project
                    );
                    for (name, service) in &state.services {
                        let url = service.route_url.as_deref().unwrap_or(&service.url);
                        println!("  {name:<16} {}  {}", service.status.label(), url);
                    }
                    println!();
                    println!("  logs:   burrow logs -f");
                    println!("  status: burrow status");
                    println!("  stop:   burrow down");
                }
                return Ok(());
            }
        }

        if started.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            let tail = read_log_tail(&log_path, 30);
            bail!("the Burrow daemon did not become ready within 180 seconds\n{tail}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn read_log_tail(path: &Path, lines: usize) -> String {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let tail = contents
        .lines()
        .rev()
        .take(lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    if tail.is_empty() {
        String::new()
    } else {
        format!("--- {} (tail) ---\n{tail}", path.display())
    }
}

/// Implements `burrow down`.
pub async fn down(options: &Options) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let Some(state) = RuntimeState::read(&project.dir)? else {
        if options.json {
            println!("{}", serde_json::json!({"running": false}));
        } else {
            println!("Burrow is not running for this project");
        }
        return Ok(());
    };

    let response =
        control::request_with_timeout(&state, ControlCommand::Shutdown, Duration::from_secs(120))
            .await;
    match response {
        Ok(response) if response.ok => {
            wait_for_stop(&project.dir, Duration::from_secs(120)).await?;
            if options.json {
                println!("{}", serde_json::json!({"running": false, "stopped": true}));
            } else {
                println!("Burrow stopped");
            }
            Ok(())
        }
        Ok(response) => bail!(
            "the running daemon refused to stop: {}\n\
             help: run `burrow status`, then retry `burrow down`",
            response.message
        ),
        Err(error) => {
            if control::reachable(state.control_port).await {
                bail!(
                    "the daemon is listening on 127.0.0.1:{} but did not answer the stop request: {error}\n\
                     help: retry `burrow down`, or inspect {} if the daemon is stuck",
                    state.control_port,
                    RuntimeState::path(&project.dir).display()
                );
            }
            // A crashed daemon leaves jobs to be closed by the OS when its
            // process handle table is torn down. Remove the stale document so
            // the next `burrow up` is not blocked.
            RuntimeState::remove(&project.dir)?;
            if options.json {
                println!(
                    "{}",
                    serde_json::json!({"running": false, "stale_state_removed": true})
                );
            } else {
                println!(
                    "Burrow was not running; removed stale state from a previous run\n\
                     note: its Job Objects were closed when that process exited"
                );
            }
            Ok(())
        }
    }
}

async fn wait_for_stop(project_dir: &Path, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        if RuntimeState::read(project_dir)?.is_none() {
            return Ok(());
        }
        if started.elapsed() > timeout {
            bail!(
                "the daemon acknowledged the stop request but is still running after {}s\n\
                 help: check `burrow status`, then use Task Manager if necessary",
                timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Implements `burrow status`.
pub async fn status(options: &Options) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let Some(state) = RuntimeState::read(&project.dir)? else {
        if options.json {
            println!("{}", serde_json::json!({"running": false}));
            return Ok(());
        }
        bail!(
            "Burrow is not running for this project\n\
             help: run `burrow up` or `burrow up --detach`"
        );
    };

    let response = match control::request(&state, ControlCommand::Status).await {
        Ok(response) if response.ok => response,
        Ok(response) => bail!("{}", response.message),
        Err(error) => {
            if control::reachable(state.control_port).await {
                bail!(
                    "the daemon is listening on 127.0.0.1:{} but did not answer: {error}",
                    state.control_port
                );
            }
            if options.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "running": false,
                        "stale": true,
                        "pid": state.pid,
                        "services": state.services,
                    })
                );
                return Ok(());
            }
            bail!(
                "Burrow is not running for this project, but stale state remains\
                 (last daemon pid {})\n\
                 help: run `burrow down` to clean it up",
                state.pid
            );
        }
    };

    let Some(state) = response.state else {
        bail!("the daemon returned no status");
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&state.public_json())?);
        return Ok(());
    }

    print_status_table(&state);
    Ok(())
}

fn print_status_table(state: &RuntimeState) {
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    println!(
        "Project: {}  (pid {}, control 127.0.0.1:{})",
        state.project, state.pid, state.control_port
    );
    println!();

    let headers = ["SERVICE", "STATE", "PORT", "RAM", "CPU", "UPTIME", "URL"];
    let mut rows = Vec::new();
    for (name, service) in &state.services {
        let uptime = service
            .started_at
            .map(|started| format_uptime(unix_now().saturating_sub(started)))
            .unwrap_or_else(|| "-".to_owned());
        let cpu = if service.status == ServiceStatus::Healthy {
            format!("{:.1}%", service.cpu_percent)
        } else {
            "-".to_owned()
        };
        let ram = if service.memory_bytes == 0 {
            "-".to_owned()
        } else {
            format_bytes(service.memory_bytes)
        };
        rows.push([
            name.clone(),
            paint_status(service.status, color),
            service.port.to_string(),
            ram,
            cpu,
            uptime,
            service
                .route_url
                .clone()
                .unwrap_or_else(|| service.url.clone()),
        ]);
    }

    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(strip_ansi(cell).len());
        }
    }

    let mut header_line = String::new();
    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            header_line.push_str("  ");
        }
        header_line.push_str(&format!("{header:<width$}", width = widths[index]));
    }
    println!("{header_line}");
    for row in &rows {
        let mut line = String::new();
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                line.push_str("  ");
            }
            let padding = widths[index].saturating_sub(strip_ansi(cell).len());
            line.push_str(cell);
            line.push_str(&" ".repeat(padding));
        }
        println!("{}", line.trim_end());
    }

    let failures: Vec<_> = state
        .services
        .iter()
        .filter_map(|(name, service)| {
            service
                .last_error
                .as_ref()
                .map(|error| format!("  {name}: {error}"))
        })
        .collect();
    if !failures.is_empty() {
        println!();
        println!("Problems:");
        for failure in failures {
            println!("{failure}");
        }
    }
}

fn paint_status(status: ServiceStatus, enabled: bool) -> String {
    let code = match status {
        ServiceStatus::Healthy => "32",
        ServiceStatus::Starting | ServiceStatus::Restarting => "33",
        ServiceStatus::Stopped => "90",
        ServiceStatus::Failed => "31",
    };
    if enabled {
        format!("\x1b[{code}m{}\x1b[0m", status.label())
    } else {
        status.label().to_owned()
    }
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

/// Implements `burrow logs`.
pub async fn logs(
    options: &Options,
    service: Option<String>,
    follow: bool,
    since: Option<String>,
) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let Some(state) = RuntimeState::read(&project.dir)? else {
        bail!(
            "Burrow is not running for this project\n\
             help: run `burrow up --detach`, then retry `burrow logs`"
        );
    };

    let cutoff = match &since {
        Some(raw) => {
            let duration = humantime::parse_duration(raw)
                .with_context(|| format!("invalid --since duration `{raw}`"))?;
            Some(control::unix_millis().saturating_sub(duration.as_millis() as u64))
        }
        None => None,
    };

    let initial = control::request(
        &state,
        ControlCommand::Logs {
            service: service.clone(),
            since_seq: 0,
            limit: 2_000,
        },
    )
    .await
    .context("could not read service logs")?;
    if !initial.ok {
        bail!("{}", initial.message);
    }

    let mut last = BTreeMap::<String, u64>::new();
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut json_entries = Vec::new();
    for entry in initial.logs {
        if cutoff.is_some_and(|cutoff| entry.at_unix_ms < cutoff) {
            continue;
        }
        last.insert(entry.service.clone(), entry.seq);
        if options.json {
            json_entries.push(entry);
        } else {
            print_log_entry(&entry, color);
        }
    }
    if options.json && !follow {
        println!("{}", serde_json::to_string_pretty(&json_entries)?);
        return Ok(());
    }
    if options.json {
        println!("{}", serde_json::to_string_pretty(&json_entries)?);
    }
    std::io::stdout().flush().ok();

    if !follow {
        return Ok(());
    }

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }

        let response = match control::request(
            &state,
            ControlCommand::Logs {
                service: service.clone(),
                since_seq: 0,
                limit: 2_000,
            },
        )
        .await
        {
            Ok(response) if response.ok => response,
            Ok(response) => bail!("{}", response.message),
            Err(error) => {
                eprintln!("log stream stopped: {error}");
                break;
            }
        };

        let mut fresh = Vec::new();
        for entry in response.logs {
            if last
                .get(&entry.service)
                .is_some_and(|seq| entry.seq <= *seq)
            {
                continue;
            }
            if cutoff.is_some_and(|cutoff| entry.at_unix_ms < cutoff) {
                continue;
            }
            last.insert(entry.service.clone(), entry.seq);
            fresh.push(entry);
        }
        fresh.sort_by_key(|entry| entry.at_unix_ms);
        for entry in fresh {
            if options.json {
                println!("{}", serde_json::to_string(&entry)?);
            } else {
                print_log_entry(&entry, color);
            }
        }
        std::io::stdout().flush().ok();
    }

    Ok(())
}

fn print_log_entry(entry: &LogEntry, color: bool) {
    let prefix = if color {
        match entry.service.as_str() {
            "api" => "\x1b[36m",
            "web" => "\x1b[35m",
            "db" => "\x1b[33m",
            "cache" => "\x1b[31m",
            _ => "\x1b[34m",
        }
    } else {
        ""
    };
    let reset = if color { "\x1b[0m" } else { "" };
    let stream = if entry.stream == "stderr" && color {
        "\x1b[31m"
    } else {
        ""
    };
    println!(
        "{prefix}{:<12}{reset} {stream}{}{reset}",
        entry.service, entry.text
    );
}

/// Implements `burrow restart`.
pub async fn restart(options: &Options, service: String) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let state = live_state(&project).await?;
    let response = control::request_with_timeout(
        &state,
        ControlCommand::Restart {
            service: service.clone(),
        },
        Duration::from_secs(120),
    )
    .await
    .context("could not reach the Burrow daemon")?;
    if !response.ok {
        bail!("{}", response.message);
    }

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "message": response.message,
                "services": response.state.map(|state| state.services),
            }))?
        );
    } else {
        println!("{}", response.message);
        println!("note: health checks continue in the background; run `burrow status`");
    }
    Ok(())
}

async fn live_state(project: &Project) -> Result<RuntimeState> {
    if let Some(state) = RuntimeState::read(&project.dir)? {
        if control::load_live_state(&project.dir).await?.is_some() {
            return Ok(state);
        }
    }
    bail!(
        "Burrow is not running for this project\n\
         help: run `burrow up --detach` first"
    )
}

/// Implements `burrow env`.
///
/// Without a service this prints the project's endpoint variables, which is the
/// form other tools usually consume. Naming a service prints its complete
/// resolved environment instead. Package resolution is planned but never
/// downloaded: printing variables must not fetch a 350 MB database.
pub async fn env(options: &Options, service: Option<String>, format: EnvFormat) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let selected = service.iter().cloned().collect::<Vec<_>>();
    let resolved = project::resolve_for_shell(&project, &selected, false).await?;

    let variables = match &service {
        Some(name) => {
            let plan = resolved
                .services
                .get(name)
                .with_context(|| format!("service `{name}` was not resolved; check burrow.toml"))?;
            plan.env
                .iter()
                .map(|(key, value)| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
                .collect()
        }
        None => {
            let mut variables = BTreeMap::new();
            variables.insert(
                "BURROW_PROJECT".to_owned(),
                project.config.project.name.clone(),
            );
            variables.insert(
                "BURROW_PROJECT_DIR".to_owned(),
                project.dir.to_string_lossy().into_owned(),
            );
            for (name, plan) in &resolved.services {
                let key = env_key(name);
                variables.insert(format!("BURROW_{key}_HOST"), "127.0.0.1".to_owned());
                variables.insert(format!("BURROW_{key}_PORT"), plan.port.to_string());
                variables.insert(format!("BURROW_{key}_URL"), plan.url.clone());
                if let (Some(route), Some(port)) = (&plan.route, resolved.proxy_port) {
                    variables.insert(
                        format!("BURROW_{key}_ROUTE_URL"),
                        burrow_net::proxy::url_for(route, port),
                    );
                }
            }
            variables
        }
    };

    write_environment(&variables, format)
}

fn env_key(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn write_environment(variables: &BTreeMap<String, String>, format: EnvFormat) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = std::io::BufWriter::new(stdout.lock());
    match format {
        EnvFormat::Dotenv => {
            for (key, value) in variables {
                writeln!(output, "{key}={}", dotenv_value(value))?;
            }
        }
        EnvFormat::Json => {
            serde_json::to_writer_pretty(&mut output, variables)?;
            writeln!(output)?;
        }
        EnvFormat::Powershell => {
            for (key, value) in variables {
                writeln!(output, "$env:{key} = '{}'", value.replace('\'', "''"))?;
            }
        }
    }
    output.flush()?;
    Ok(())
}

/// Quotes a dotenv value only when it would otherwise change meaning.
fn dotenv_value(value: &str) -> String {
    let safe = !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-.\\/:@+".contains(character));
    if safe {
        return value.to_owned();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// Implements `burrow exec`.
///
/// The child inherits Burrow's standard handles and environment, with the
/// service's resolved variables layered on top. Its exit code is preserved.
pub async fn exec(options: &Options, service: String, command: Vec<OsString>) -> Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        bail!("burrow exec needs a command, for example `burrow exec web -- node --version`");
    };
    let project = Project::load(options.project.clone())?;
    let resolved =
        project::resolve_for_shell(&project, std::slice::from_ref(&service), true).await?;
    let plan = resolved
        .services
        .get(&service)
        .with_context(|| format!("service `{service}` was not resolved; check burrow.toml"))?;

    // A managed package prepends its bin directories to PATH, so resolve
    // against the service environment before falling back to Burrow's own.
    let search_path = plan
        .env
        .get(OsStr::new("PATH"))
        .cloned()
        .or_else(|| std::env::var_os("PATH"));
    let program = win::resolve_executable(program, &plan.cwd, search_path.as_deref())
        .with_context(|| format!("could not resolve `{}`", program.to_string_lossy()))?;

    let status = std::process::Command::new(&program)
        .args(arguments)
        .current_dir(&plan.cwd)
        .envs(&plan.env)
        .status()
        .with_context(|| format!("could not run {}", program.display()))?;
    if !status.success() {
        return Err(ExitStatusError(status.code().unwrap_or(1)).into());
    }
    Ok(())
}

/// Implements `burrow open`.
pub async fn open(options: &Options, service: Option<String>) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let state = live_state(&project).await?;
    let (name, url) = choose_open_url(&state, service.as_deref())?;

    if std::env::var_os("BURROW_NO_BROWSER").is_some() {
        println!("{url}");
        return Ok(());
    }

    eprintln!("opening {url} ({name})");
    win::open_url(&url).with_context(|| format!("could not open {url}"))?;
    Ok(())
}

fn choose_open_url(state: &RuntimeState, service: Option<&str>) -> Result<(String, String)> {
    match service {
        Some(name) => {
            let snapshot = state.services.get(name).with_context(|| {
                let known = state
                    .services
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("service `{name}` is not running (running services: {known})")
            })?;
            let url = snapshot
                .route_url
                .clone()
                .or_else(|| {
                    if snapshot.url.starts_with("http://") || snapshot.url.starts_with("https://") {
                        Some(snapshot.url.clone())
                    } else {
                        None
                    }
                })
                .with_context(|| {
                    format!(
                        "service `{name}` has no browser URL; add a `route` to it in burrow.toml \
                         (its endpoint is {})",
                        snapshot.url
                    )
                })?;
            Ok((name.to_owned(), url))
        }
        None => state
            .services
            .iter()
            .find_map(|(name, snapshot)| snapshot.route_url.clone().map(|url| (name.clone(), url)))
            .with_context(|| {
                "no service has a route\n\
                 help: add `route = \"app.localhost\"` to a web service in burrow.toml"
            }),
    }
}

/// Implements `burrow schema`.
pub fn schema() -> Result<()> {
    println!("{}", burrow_config::json_schema());
    Ok(())
}

/// Tracks download progress on stderr without spamming a pipe or CI log.
struct ProgressPrinter {
    interactive: bool,
    last: BTreeMap<String, u64>,
    printed: bool,
}

impl ProgressPrinter {
    fn new() -> Self {
        Self {
            interactive: std::io::stderr().is_terminal(),
            last: BTreeMap::new(),
            printed: false,
        }
    }

    fn update(&mut self, service: &str, progress: DownloadProgress) {
        let total = progress.total.max(1);
        let percent = progress.downloaded.saturating_mul(100) / total;
        let previous = self.last.get(service).copied().unwrap_or(0);
        let step = if self.interactive { 1 } else { 10 };
        if percent < 100 && percent < previous.saturating_add(step) {
            return;
        }
        self.last.insert(service.to_owned(), percent);
        self.printed = true;
        if self.interactive {
            eprint!(
                "\r  {service:<12} {percent:>3}%  {:>10} / {}",
                format_bytes(progress.downloaded),
                format_bytes(progress.total)
            );
            std::io::stderr().flush().ok();
        } else {
            eprintln!(
                "  {service:<12} {percent:>3}%  {:>10} / {}",
                format_bytes(progress.downloaded),
                format_bytes(progress.total)
            );
        }
    }

    fn finish(&mut self) {
        if self.printed && self.interactive {
            eprintln!();
        }
        self.printed = false;
    }
}

/// Implements `burrow pkg list`.
pub fn pkg_list(options: &Options) -> Result<()> {
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    let installed = cache.installed();
    if options.json {
        let packages = installed
            .iter()
            .map(|(sha256, marker)| {
                serde_json::json!({
                    "name": marker.name,
                    "version": marker.package_version,
                    "sha256": sha256,
                    "size": marker.size,
                    "installed_at": marker.installed_at,
                    "path": cache.package_dir(sha256),
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "cache": cache.root(),
                "packages": packages,
                "bytes_on_disk": cache.size_on_disk(),
            }))?
        );
        return Ok(());
    }

    if installed.is_empty() {
        println!("The shared package cache is empty");
        println!("  cache: {}", cache.root().display());
        println!("  help:  burrow pkg add postgres@16");
        return Ok(());
    }

    println!(
        "{:<14} {:<10} {:>12}  SHA-256",
        "NAME", "VERSION", "ARCHIVE"
    );
    for (sha256, marker) in &installed {
        println!(
            "{:<14} {:<10} {:>12}  {}",
            marker.name,
            marker.package_version,
            format_bytes(marker.size),
            &sha256[..sha256.len().min(12)]
        );
    }
    println!();
    println!(
        "{} package(s), {} in {}",
        installed.len(),
        format_bytes(cache.size_on_disk()),
        cache.root().display()
    );
    Ok(())
}

/// Implements `burrow pkg add`.
pub async fn pkg_add(
    options: &Options,
    reference: String,
    service: Option<String>,
    seed: Option<PathBuf>,
) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    let registry = Registry::builtin().map_err(|error| anyhow::anyhow!("{error}"))?;
    let reference = reference.trim().to_owned();
    let resolved = registry
        .resolve(&reference)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let package_key = resolved.key();
    let service_name = service.unwrap_or_else(|| resolved.entry.name.clone());
    validate_service_name(&service_name)?;

    if project.config.services.contains_key(&service_name) {
        bail!(
            "service `{service_name}` already exists in burrow.toml\n\
             help: choose another name with `--service <NAME>`, or remove the existing service"
        );
    }
    if let Some(seed) = &seed {
        if seed.is_absolute()
            || seed
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            bail!(
                "--seed must be a path inside the project, not `{}`",
                seed.display()
            );
        }
        let path = project.dir.join(seed);
        if !path.is_file() {
            bail!("seed script {} does not exist", path.display());
        }
    }

    if options.json {
        project::fetch_reference(&reference, &mut |_, _| {}).await?;
    } else {
        let mut progress = ProgressPrinter::new();
        project::fetch_reference(&reference, &mut |name, update| {
            progress.update(name, update);
        })
        .await?;
        progress.finish();
    }

    let seed_value = seed
        .as_ref()
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    edit_config(&project, |document| {
        let services = document
            .entry("services")
            .or_insert_with(|| Item::Table(Table::new()));
        let table = services
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("`services` in burrow.toml must be a table"))?;
        if table.contains_key(&service_name) {
            bail!("service `{service_name}` already exists in burrow.toml");
        }
        let mut service_table = Table::new();
        service_table.insert("use", toml_edit::value(reference.as_str()));
        if let Some(seed) = &seed_value {
            service_table.insert("seed", toml_edit::value(seed.as_str()));
        }
        table.insert(&service_name, Item::Table(service_table));
        Ok(())
    })?;

    let reloaded = Project::load(Some(project.dir.clone()))?;
    project::record_package_refs(&reloaded)?;

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "service": service_name,
                "package": package_key,
                "config": reloaded.dir.join(burrow_config::CONFIG_FILE),
            }))?
        );
    } else {
        println!(
            "Added `{service_name}` ({package_key}) to {}",
            reloaded.dir.join(burrow_config::CONFIG_FILE).display()
        );
        println!("  data: .burrow/data/{service_name}");
        println!("  next: burrow up");
    }
    Ok(())
}

/// Implements `burrow pkg remove`.
pub async fn pkg_remove(options: &Options, name: String) -> Result<()> {
    let project = Project::load(options.project.clone())?;
    if control::load_live_state(&project.dir).await?.is_some() {
        bail!(
            "Burrow is running for this project\n\
             help: run `burrow down` before removing a service"
        );
    }

    let key = if project.config.services.contains_key(&name) {
        Some(name.clone())
    } else {
        project
            .config
            .services
            .iter()
            .find(|(_, service)| {
                service
                    .use_spec
                    .as_deref()
                    .is_some_and(|reference| reference.split('@').next() == Some(name.as_str()))
            })
            .map(|(service_name, _)| service_name.clone())
    };
    let Some(key) = key else {
        bail!(
            "no service named `{name}` in this project\n\
             help: run `burrow status` or inspect burrow.toml"
        );
    };
    if project.config.services[&key].use_spec.is_none() {
        bail!(
            "service `{key}` runs a command instead of a package\n\
             help: remove its `[services.{key}]` table from burrow.toml by hand"
        );
    }
    if project.config.services.len() == 1 {
        bail!(
            "`{key}` is the only service in this project\n\
             help: edit burrow.toml to replace it instead of removing it"
        );
    }

    edit_config(&project, |document| {
        let services = document
            .get_mut("services")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| anyhow::anyhow!("`services` in burrow.toml must be a table"))?;
        if services.remove(&key).is_none() {
            bail!("service `{key}` was not found in burrow.toml");
        }
        Ok(())
    })?;

    let reloaded = Project::load(Some(project.dir.clone()))?;
    project::record_package_refs(&reloaded)?;

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "service": key,
                "removed": true,
            }))?
        );
    } else {
        println!("Removed service `{key}` from burrow.toml");
        println!("  note: cached files stay until `burrow pkg gc`");
    }
    Ok(())
}

/// Implements `burrow pkg gc`.
pub fn pkg_gc(options: &Options) -> Result<()> {
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    let report = cache.gc().map_err(|error| anyhow::anyhow!("{error}"))?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "packages_removed": report.packages_removed,
                "archives_removed": report.archives_removed,
                "stale_refs_removed": report.stale_refs_removed,
                "bytes_freed": report.bytes_freed,
            }))?
        );
    } else if report.packages_removed == 0
        && report.archives_removed == 0
        && report.stale_refs_removed == 0
    {
        println!("Package cache is already clean");
    } else {
        println!(
            "Removed {} package(s) and {} archive(s), freeing {}{}",
            report.packages_removed,
            report.archives_removed,
            format_bytes(report.bytes_freed),
            if report.stale_refs_removed > 0 {
                format!(
                    "; dropped {} stale project reference(s)",
                    report.stale_refs_removed
                )
            } else {
                String::new()
            }
        );
    }
    Ok(())
}

/// Implements `burrow pkg verify`.
pub fn pkg_verify(options: &Options) -> Result<()> {
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    let report = cache.verify().map_err(|error| anyhow::anyhow!("{error}"))?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": report.problems.is_empty(),
                "archives_checked": report.archives_checked,
                "packages_checked": report.packages_checked,
                "problems": report.problems,
            }))?
        );
    } else {
        for problem in &report.problems {
            eprintln!("problem: {problem}");
        }
        if report.problems.is_empty() {
            println!(
                "Package cache is healthy: {} archive(s) and {} package(s) verified",
                report.archives_checked, report.packages_checked
            );
        }
    }
    if !report.problems.is_empty() {
        bail!(
            "package cache verification found {} problem(s)\n\
             help: delete the affected files, or run `burrow pkg gc` to drop unreferenced entries",
            report.problems.len()
        );
    }
    Ok(())
}

fn validate_service_name(name: &str) -> Result<()> {
    let mut characters = name.chars();
    let starts_well = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let rest_well = characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    });
    if !starts_well || !rest_well {
        bail!(
            "invalid service name `{name}`\n\
             help: use lowercase letters, digits, `-`, and `_`, starting with a letter"
        );
    }
    Ok(())
}

fn edit_config(
    project: &Project,
    change: impl FnOnce(&mut DocumentMut) -> Result<()>,
) -> Result<()> {
    let path = project.dir.join(burrow_config::CONFIG_FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let mut document: DocumentMut = text
        .parse()
        .with_context(|| format!("could not parse {}", path.display()))?;
    change(&mut document)?;
    std::fs::write(&path, document.to_string())
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}
