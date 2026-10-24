//! `burrow doctor`: inspect the machine and the project for common problems.
//!
//! Checks are read-only. They never download a package, generate a secret, or
//! bind a port. Every failure carries a concrete remedy, and the command exits
//! nonzero when any check fails so scripts can gate on it.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use burrow_pkg::{Cache, Registry};
use serde::Serialize;

use crate::commandline;
use crate::commands::Options;
use crate::control;
use crate::project::{self, Project};
use crate::state::RuntimeState;
use crate::win;

/// Result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    /// Everything is in order.
    Ok,
    /// Worth knowing, but not fatal.
    Warn,
    /// Burrow cannot work until this is fixed.
    Fail,
}

impl CheckStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
        }
    }
}

/// One diagnostic check.
#[derive(Debug, Clone, Serialize)]
struct Check {
    name: String,
    status: CheckStatus,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remedy: Option<String>,
}

impl Check {
    fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
            remedy: None,
        }
    }

    fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }
}

/// The machine-readable report printed by `burrow doctor --json`.
#[derive(Debug, Serialize)]
struct Report {
    ok: bool,
    project: String,
    project_dir: PathBuf,
    summary: Summary,
    checks: Vec<Check>,
}

#[derive(Debug, Serialize)]
struct Summary {
    ok: usize,
    warnings: usize,
    failures: usize,
}

/// Implements `burrow doctor`.
pub async fn run(options: &Options) -> Result<()> {
    let directory = project_directory(options)?;
    let mut checks = Vec::new();

    platform_check(&mut checks);

    let project = match Project::load(Some(directory.clone())) {
        Ok(project) => {
            checks.push(Check::new(
                "config",
                CheckStatus::Ok,
                format!(
                    "burrow.toml is valid ({} service{})",
                    project.config.services.len(),
                    if project.config.services.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            ));
            Some(project)
        }
        Err(error) => {
            let has_config = directory.join("burrow.toml").is_file();
            let detail = if has_config {
                format!("{error:#}")
            } else {
                format!("no burrow.toml in {}", directory.display())
            };
            checks.push(
                Check::new("config", CheckStatus::Fail, single_line(&detail)).with_remedy(
                    "run `burrow init` in the project directory, or pass --project <DIR>",
                ),
            );
            None
        }
    };

    let cache = cache_check(&mut checks);
    let live_state = daemon_check(&directory, project.as_ref(), &mut checks).await;

    if let Some(project) = &project {
        trust_check(project, &mut checks);
        secrets_check(project, &mut checks);
        packages_check(project, cache.as_ref(), &mut checks);
        ports_check(project, live_state.as_ref(), &mut checks);
        commands_check(project, &mut checks);
        proxy_check(project, live_state.as_ref(), &mut checks);
    }

    let summary = Summary {
        ok: checks
            .iter()
            .filter(|check| check.status == CheckStatus::Ok)
            .count(),
        warnings: checks
            .iter()
            .filter(|check| check.status == CheckStatus::Warn)
            .count(),
        failures: checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .count(),
    };

    let project_name = project
        .as_ref()
        .map(|project| project.config.project.name.clone())
        .unwrap_or_else(|| {
            directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".to_owned())
        });

    let report = Report {
        ok: summary.failures == 0,
        project: project_name,
        project_dir: directory.clone(),
        summary,
        checks,
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }

    if report.summary.failures > 0 {
        bail!(
            "doctor found {} problem{}",
            report.summary.failures,
            if report.summary.failures == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    Ok(())
}

fn project_directory(options: &Options) -> Result<PathBuf> {
    project::resolve_directory(options.project.clone())
}

fn platform_check(checks: &mut Vec<Check>) {
    let architecture = std::env::consts::ARCH;
    let detail = format!("{} ({architecture})", std::env::consts::OS);
    if architecture == "x86_64" {
        checks.push(Check::new("platform", CheckStatus::Ok, detail));
    } else {
        checks.push(
            Check::new(
                "platform",
                CheckStatus::Warn,
                format!("{detail}; Burrow publishes x86-64 builds only"),
            )
            .with_remedy("run the x86-64 build under Windows emulation, or build from source"),
        );
    }

    if win::is_elevated() {
        checks.push(
            Check::new(
                "session",
                CheckStatus::Warn,
                "this terminal is elevated (running as administrator)",
            )
            .with_remedy(
                "open a normal terminal; PostgreSQL refuses to initialize with an \
                 administrator token",
            ),
        );
    } else {
        checks.push(Check::new(
            "session",
            CheckStatus::Ok,
            "normal user token (not elevated)",
        ));
    }
}

fn cache_check(checks: &mut Vec<Check>) -> Option<Cache> {
    let cache = match Cache::discover() {
        Ok(cache) => cache,
        Err(error) => {
            checks.push(
                Check::new("cache", CheckStatus::Fail, format!("{error}")).with_remedy(
                    "set BURROW_CACHE_DIR to a writable directory, or make sure LOCALAPPDATA is set",
                ),
            );
            return None;
        }
    };

    let root = cache.root();
    if !root.exists() {
        checks.push(Check::new(
            "cache",
            CheckStatus::Ok,
            format!(
                "{} does not exist yet; it is created on the first download",
                root.display()
            ),
        ));
        return Some(cache);
    }
    if !root.is_dir() {
        checks.push(
            Check::new(
                "cache",
                CheckStatus::Fail,
                format!("{} exists but is not a directory", root.display()),
            )
            .with_remedy("move the file away or set BURROW_CACHE_DIR to another directory"),
        );
        return Some(cache);
    }
    if writable(root) {
        checks.push(Check::new(
            "cache",
            CheckStatus::Ok,
            format!(
                "{} ({})",
                root.display(),
                crate::state::format_bytes(cache.size_on_disk())
            ),
        ));
    } else {
        checks.push(
            Check::new(
                "cache",
                CheckStatus::Fail,
                format!("{} is not writable", root.display()),
            )
            .with_remedy(
                "fix the directory permissions, or set BURROW_CACHE_DIR to a writable directory",
            ),
        );
    }
    Some(cache)
}

/// Writes and removes a tiny probe file. The name includes the process id and
/// a timestamp so parallel doctor runs do not collide.
fn writable(directory: &Path) -> bool {
    let probe = directory.join(format!(
        ".burrow-doctor-{}-{}",
        std::process::id(),
        control::unix_millis()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

async fn daemon_check(
    directory: &Path,
    project: Option<&Project>,
    checks: &mut Vec<Check>,
) -> Option<RuntimeState> {
    let state = match RuntimeState::read(directory) {
        Ok(state) => state,
        Err(error) => {
            checks.push(
                Check::new(
                    "daemon",
                    CheckStatus::Fail,
                    single_line(&format!("{error:#}")),
                )
                .with_remedy(
                    "delete .burrow/state.json if no Burrow process is running, then retry",
                ),
            );
            return None;
        }
    };

    let Some(stored) = state else {
        checks.push(Check::new(
            "daemon",
            CheckStatus::Ok,
            "not running (normal when the project is stopped)",
        ));
        return None;
    };

    match control::load_live_state(directory).await {
        Ok(Some(live)) => {
            let state = if live.ready { "ready" } else { "starting" };
            checks.push(Check::new(
                "daemon",
                CheckStatus::Ok,
                format!(
                    "running (pid {}, {state}, {} service{})",
                    live.pid,
                    live.services.len(),
                    if live.services.len() == 1 { "" } else { "s" }
                ),
            ));
            if let Some(project) = project {
                if live.config_hash != project.config_hash {
                    checks.push(
                        Check::new(
                            "config drift",
                            CheckStatus::Warn,
                            "burrow.toml changed since the daemon started; the running services use the old configuration",
                        )
                        .with_remedy("run `burrow down` and `burrow up` to apply the change"),
                    );
                }
            }
            Some(live)
        }
        Ok(None) => {
            checks.push(
                Check::new(
                    "daemon",
                    CheckStatus::Warn,
                    format!(
                        "stale state: the daemon on 127.0.0.1:{} is not answering",
                        stored.control_port
                    ),
                )
                .with_remedy(
                    "run `burrow down` to clean up; if it reports no daemon, delete .burrow/state.json",
                ),
            );
            None
        }
        Err(error) => {
            checks.push(
                Check::new(
                    "daemon",
                    CheckStatus::Fail,
                    single_line(&format!("{error:#}")),
                )
                .with_remedy("delete .burrow/state.json if no Burrow process is running"),
            );
            None
        }
    }
}

fn trust_check(project: &Project, checks: &mut Vec<Check>) {
    if project::is_trusted(project) {
        checks.push(Check::new(
            "trust",
            CheckStatus::Ok,
            format!(
                "commands approved for this configuration (hash {})",
                short_hash(&project.config_hash)
            ),
        ));
    } else {
        checks.push(
            Check::new(
                "trust",
                CheckStatus::Warn,
                "this configuration is not approved yet",
            )
            .with_remedy("run `burrow up` and review the command summary; use --yes only in CI"),
        );
    }
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn secrets_check(project: &Project, checks: &mut Vec<Check>) {
    let path = project.burrow_dir.join("secrets");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            checks.push(Check::new(
                "secrets",
                CheckStatus::Ok,
                "not generated yet; the first `burrow up` creates them",
            ));
            return;
        }
        Err(error) => {
            checks.push(
                Check::new("secrets", CheckStatus::Fail, format!("{error}"))
                    .with_remedy("check the permissions of .burrow/secrets"),
            );
            return;
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(error) => {
            checks.push(
                Check::new(
                    "secrets",
                    CheckStatus::Fail,
                    format!(".burrow/secrets is not valid JSON: {error}"),
                )
                .with_remedy(
                    "delete .burrow/secrets to regenerate it; generated service passwords will change",
                ),
            );
            return;
        }
    };

    let missing = project
        .config
        .services
        .keys()
        .filter(|name| {
            parsed
                .get(name.as_str())
                .and_then(|value| value.as_str())
                .is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        checks.push(Check::new(
            "secrets",
            CheckStatus::Ok,
            format!(
                "generated passwords present for {} service{}",
                project.config.services.len(),
                if project.config.services.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        ));
    } else {
        checks.push(
            Check::new(
                "secrets",
                CheckStatus::Warn,
                format!("missing generated password for: {}", missing.join(", ")),
            )
            .with_remedy("run `burrow up`; missing passwords are generated automatically"),
        );
    }
}

fn packages_check(project: &Project, cache: Option<&Cache>, checks: &mut Vec<Check>) {
    let registry = match Registry::builtin() {
        Ok(registry) => registry,
        Err(error) => {
            checks.push(Check::new(
                "registry",
                CheckStatus::Fail,
                format!("built-in registry did not parse: {error}"),
            ));
            return;
        }
    };

    for (name, service) in &project.config.services {
        let Some(reference) = &service.use_spec else {
            continue;
        };
        let resolved = match registry.resolve(reference) {
            Ok(resolved) => resolved,
            Err(error) => {
                checks.push(
                    Check::new(
                        format!("package {name}"),
                        CheckStatus::Fail,
                        format!("{error}"),
                    )
                    .with_remedy(
                        "use a reference listed by `burrow pkg list`, for example `postgres@16`",
                    ),
                );
                continue;
            }
        };
        let key = resolved.key();
        let Some(cache) = cache else {
            checks.push(Check::new(
                format!("package {name}"),
                CheckStatus::Warn,
                format!("{key} cannot be inspected because the cache is unavailable"),
            ));
            continue;
        };
        if !cache.is_installed(&resolved) {
            checks.push(Check::new(
                format!("package {name}"),
                CheckStatus::Warn,
                format!(
                    "{key} is not downloaded yet ({:.0} MB); the first `burrow up` verifies its SHA-256",
                    resolved.platform.size as f64 / (1024.0 * 1024.0)
                ),
            ));
            continue;
        }

        let root = cache.package_dir(&resolved.platform.sha256);
        let missing = resolved
            .platform
            .binaries
            .iter()
            .filter(|(_, relative)| {
                !root
                    .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .is_file()
            })
            .map(|(role, _)| role.clone())
            .collect::<Vec<_>>();
        if missing.is_empty() {
            checks.push(Check::new(
                format!("package {name}"),
                CheckStatus::Ok,
                format!("{key} installed in {}", root.display()),
            ));
        } else {
            checks.push(
                Check::new(
                    format!("package {name}"),
                    CheckStatus::Fail,
                    format!(
                        "{key} is incomplete; missing binaries: {}",
                        missing.join(", ")
                    ),
                )
                .with_remedy(format!(
                    "run `burrow pkg verify`, then remove `{}` and run `burrow up` again",
                    root.display()
                )),
            );
        }
    }
}

fn ports_check(project: &Project, live: Option<&RuntimeState>, checks: &mut Vec<Check>) {
    for (name, service) in &project.config.services {
        let Some(port) = service.port.fixed() else {
            continue;
        };
        let used_by_project = live
            .and_then(|state| state.services.get(name))
            .is_some_and(|snapshot| snapshot.port == port);
        if used_by_project {
            checks.push(Check::new(
                format!("port {port}"),
                CheckStatus::Ok,
                format!("held by `{name}` in the running project"),
            ));
        } else if burrow_net::is_free(port) {
            checks.push(Check::new(
                format!("port {port}"),
                CheckStatus::Ok,
                format!("free for `{name}`"),
            ));
        } else if live.is_some() {
            checks.push(Check::new(
                format!("port {port}"),
                CheckStatus::Ok,
                format!("in use while the project's daemon is running (`{name}`)"),
            ));
        } else {
            checks.push(
                Check::new(
                    format!("port {port}"),
                    CheckStatus::Fail,
                    format!("`{name}` pins this port, but it is already in use"),
                )
                .with_remedy(format!(
                    "stop the other program, or change `port` for `{name}` in burrow.toml; \
                     `Get-NetTCPConnection -LocalPort {port}` shows the owner"
                )),
            );
        }
    }
}

fn commands_check(project: &Project, checks: &mut Vec<Check>) {
    for (name, service) in &project.config.services {
        let Some(command) = &service.run else {
            continue;
        };
        let cwd = match &service.cwd {
            Some(cwd) if cwd.is_absolute() => cwd.clone(),
            Some(cwd) => project.dir.join(cwd),
            None => project.dir.clone(),
        };
        if !cwd.is_dir() {
            checks.push(
                Check::new(
                    format!("command {name}"),
                    CheckStatus::Fail,
                    format!("working directory {} does not exist", cwd.display()),
                )
                .with_remedy(format!(
                    "create `{}`, or fix `cwd` for `{name}`",
                    cwd.display()
                )),
            );
            continue;
        }

        let (program, _) = match commandline::split(command) {
            Ok(split) => split,
            Err(error) => {
                checks.push(
                    Check::new(
                        format!("command {name}"),
                        CheckStatus::Fail,
                        format!("{error}"),
                    )
                    .with_remedy("fix the quoting in this service's `run` value"),
                );
                continue;
            }
        };
        let search_path = std::env::var_os("PATH");
        match win::resolve_executable(std::ffi::OsStr::new(&program), &cwd, search_path.as_deref())
        {
            Ok(resolved) => checks.push(Check::new(
                format!("command {name}"),
                CheckStatus::Ok,
                format!("`{program}` resolves to {}", resolved.display()),
            )),
            Err(error) => checks.push(
                Check::new(
                    format!("command {name}"),
                    CheckStatus::Fail,
                    error.to_string(),
                )
                .with_remedy(format!(
                    "install `{program}` and make sure it is on PATH, or change `run` for `{name}`"
                )),
            ),
        }
    }
}

fn proxy_check(project: &Project, live: Option<&RuntimeState>, checks: &mut Vec<Check>) {
    let routes = project
        .config
        .services
        .iter()
        .filter_map(|(name, service)| {
            service
                .route
                .as_ref()
                .map(|route| (name.clone(), route.clone()))
        })
        .collect::<Vec<_>>();
    if routes.is_empty() {
        return;
    }

    if let Some(live) = live {
        if let Some(port) = live.proxy_port {
            checks.push(Check::new(
                "proxy",
                CheckStatus::Ok,
                format!(
                    "listening on port {port} for {} route{}",
                    routes.len(),
                    if routes.len() == 1 { "" } else { "s" }
                ),
            ));
        }
    } else if burrow_net::is_free(80) {
        checks.push(Check::new(
            "proxy",
            CheckStatus::Ok,
            "port 80 is free; routed URLs will have no port suffix",
        ));
    } else {
        checks.push(Check::new(
            "proxy",
            CheckStatus::Warn,
            "port 80 is in use; routed URLs will include a fallback port",
        ));
    }

    let exposed = project
        .config
        .services
        .iter()
        .filter(|(_, service)| service.expose)
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    if !exposed.is_empty() {
        checks.push(
            Check::new(
                "exposure",
                CheckStatus::Warn,
                format!(
                    "these services accept non-loopback traffic through the proxy: {}",
                    exposed.join(", ")
                ),
            )
            .with_remedy("remove `expose = true` unless another machine must reach them"),
        );
    }
}

fn print_human(report: &Report) {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let color = output.is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let _ = writeln!(
        output,
        "Burrow doctor · {} ({})",
        report.project,
        report.project_dir.display()
    );
    for check in &report.checks {
        let (prefix, reset) = if color {
            match check.status {
                CheckStatus::Ok => ("\x1b[32m", "\x1b[0m"),
                CheckStatus::Warn => ("\x1b[33m", "\x1b[0m"),
                CheckStatus::Fail => ("\x1b[31m", "\x1b[0m"),
            }
        } else {
            ("", "")
        };
        let _ = writeln!(
            output,
            "  {prefix}{:<5}{reset} {:<18} {}",
            check.status.label(),
            check.name,
            check.detail
        );
        if let Some(remedy) = &check.remedy {
            let _ = writeln!(output, "        {:18} help: {remedy}", "");
        }
    }
    let summary = &report.summary;
    let _ = writeln!(
        output,
        "\n{} ok, {} warning{}, {} problem{}",
        summary.ok,
        summary.warnings,
        if summary.warnings == 1 { "" } else { "s" },
        summary.failures,
        if summary.failures == 1 { "" } else { "s" }
    );
}

fn single_line(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::{single_line, Check, CheckStatus};

    #[test]
    fn human_status_labels_are_stable() {
        assert_eq!(CheckStatus::Ok.label(), "ok");
        assert_eq!(CheckStatus::Warn.label(), "warn");
        assert_eq!(CheckStatus::Fail.label(), "FAIL");
    }

    #[test]
    fn multiline_errors_become_one_readable_line() {
        assert_eq!(
            single_line("first line\n\n  second line  \n"),
            "first line; second line"
        );
    }

    #[test]
    fn checks_serialize_without_an_empty_remedy() {
        let check = Check::new("config", CheckStatus::Ok, "valid");
        let value = serde_json::to_value(&check).expect("serialize check");
        assert_eq!(value["status"], "ok");
        assert!(value.get("remedy").is_none());
    }
}
