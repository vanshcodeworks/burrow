//! Per-project runtime state persisted in `.burrow/state.json`.
//!
//! The state file is written by the daemon and read by `status`, `down`, and
//! `logs` when the control socket is unavailable. It contains the IPC token, so
//! the file is a local convenience, not a security boundary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Current state-file and protocol version.
pub const STATE_VERSION: u32 = 1;
/// Directory inside the project that holds runtime state.
pub const BURROW_DIR: &str = ".burrow";
/// File name of the state document.
pub const STATE_FILE: &str = "state.json";

/// Lifecycle state of one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceStatus {
    /// A process is starting and its health check has not passed yet.
    Starting,
    /// The health check passed (or no check is configured).
    Healthy,
    /// Waiting for a backoff delay before the next attempt.
    Restarting,
    /// Stopped deliberately, or exited without a restart policy.
    Stopped,
    /// Stopped by idle-stop; it wakes on the next connection.
    Idle,
    /// Gave up after repeated failures.
    Failed,
}

impl ServiceStatus {
    /// A short label for tables.
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Healthy => "healthy",
            Self::Restarting => "restarting",
            Self::Stopped => "stopped",
            Self::Idle => "idle",
            Self::Failed => "failed",
        }
    }
}

/// Last-known state and metrics for one service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    /// Lifecycle state.
    pub status: ServiceStatus,
    /// Direct child process id, when a process exists.
    pub pid: Option<u32>,
    /// Allocated port.
    pub port: u16,
    /// Generated URL.
    pub url: String,
    /// Reverse-proxy hostname, when configured.
    pub route: Option<String>,
    /// Full browser URL for the route, when the proxy is running.
    #[serde(default)]
    pub route_url: Option<String>,
    /// Number of automatic restarts since the daemon started.
    pub restarts: u32,
    /// Unix seconds when the current process started.
    pub started_at: Option<u64>,
    /// Exit code of the last exited process.
    pub exit_code: Option<i32>,
    /// Working-set memory of the process tree.
    pub memory_bytes: u64,
    /// CPU usage sampled over the last interval, as a percentage of one core.
    pub cpu_percent: f32,
    /// Last actionable error, if the service failed or is restarting.
    pub last_error: Option<String>,
    /// How the service runs: a registry key such as `postgres@16`, a
    /// container reference such as `container:redis:7`, or `None` for a
    /// plain `run` command.
    #[serde(default)]
    pub runtime: Option<String>,
}

/// The complete state document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    /// Schema version.
    pub version: u32,
    /// Project name from `burrow.toml`.
    pub project: String,
    /// Absolute project directory.
    pub project_dir: PathBuf,
    /// Daemon process id.
    pub pid: u32,
    /// IPC bearer token.
    pub token: String,
    /// Loopback control port.
    pub control_port: u16,
    /// Port of the reverse proxy, when the project has routed services.
    #[serde(default)]
    pub proxy_port: Option<u16>,
    /// Unix seconds when the daemon started.
    pub started_at: u64,
    /// Hash of the approved executable configuration.
    pub config_hash: String,
    /// True once every requested service is healthy.
    pub ready: bool,
    /// Services keyed by name.
    pub services: BTreeMap<String, ServiceSnapshot>,
}

impl RuntimeState {
    /// Path of the state document inside a project's `.burrow` directory.
    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(BURROW_DIR).join(STATE_FILE)
    }

    /// Writes the state atomically.
    pub fn write(&self, project_dir: &Path) -> Result<()> {
        let path = Self::path(project_dir);
        let parent = path.parent().expect("state path has a parent");
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
        let temporary = path.with_extension("json.tmp");
        let encoded = serde_json::to_vec_pretty(self).context("could not encode runtime state")?;
        std::fs::write(&temporary, encoded)
            .with_context(|| format!("could not write {}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .with_context(|| format!("could not replace {}", path.display()))?;
        Ok(())
    }

    /// Reads the state document, if it exists.
    pub fn read(project_dir: &Path) -> Result<Option<Self>> {
        let path = Self::path(project_dir);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()))
            }
        };
        let state: Self = serde_json::from_str(&contents)
            .with_context(|| format!("{} is not valid Burrow state", path.display()))?;
        Ok(Some(state))
    }

    /// Removes the state document. Missing files are not an error.
    pub fn remove(project_dir: &Path) -> Result<()> {
        match std::fs::remove_file(Self::path(project_dir)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("could not remove {}", Self::path(project_dir).display())),
        }
    }

    /// A state view without the IPC token, safe to print.
    pub fn public_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "project": self.project,
            "project_dir": self.project_dir,
            "pid": self.pid,
            "control_port": self.control_port,
            "proxy_port": self.proxy_port,
            "started_at": self.started_at,
            "config_hash": self.config_hash,
            "ready": self.ready,
            "services": self.services,
        })
    }
}

/// Returns the current Unix time in seconds.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Formats a duration roughly the way users read uptime.
pub fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Formats a byte count with binary units.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
