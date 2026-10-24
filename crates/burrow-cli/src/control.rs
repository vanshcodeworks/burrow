//! Loopback control protocol between CLI commands and the daemon.
//!
//! The protocol is one JSON request line and one JSON response line per
//! connection. Authentication is a random bearer token stored in
//! `.burrow/state.json`; the listener binds to `127.0.0.1` only.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::state::{RuntimeState, STATE_VERSION};

/// Maximum accepted request size.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// A command sent to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlCommand {
    /// Check that the daemon is alive.
    Ping,
    /// Return a fresh state snapshot.
    Status,
    /// Return buffered log lines.
    Logs {
        /// Restrict to one service, or `None` for all.
        service: Option<String>,
        /// Return lines with a sequence number greater than this.
        since_seq: u64,
        /// Maximum number of lines; 0 means no limit.
        limit: usize,
    },
    /// Restart a service and its dependents.
    Restart {
        /// Service name.
        service: String,
    },
    /// Stop everything and exit the daemon.
    Shutdown,
}

/// One request line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    /// Protocol version.
    pub version: u32,
    /// Bearer token from the state file.
    pub token: String,
    /// The command to execute.
    pub command: ControlCommand,
}

impl ControlRequest {
    /// Builds a request for a known state document.
    pub fn new(state: &RuntimeState, command: ControlCommand) -> Self {
        Self {
            version: STATE_VERSION,
            token: state.token.clone(),
            command,
        }
    }
}

/// One captured log line sent to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Service name.
    pub service: String,
    /// Monotonic sequence number within this daemon's ring.
    pub seq: u64,
    /// `stdout` or `stderr`.
    pub stream: String,
    /// Line text.
    pub text: String,
    /// Capture time in Unix milliseconds.
    pub at_unix_ms: u64,
}

impl LogEntry {
    /// Builds an entry from a supervisor log line.
    pub fn from_line(service: &str, line: &burrow_supervisor::LogLine) -> Self {
        Self {
            service: service.to_owned(),
            seq: line.seq,
            stream: line.stream.clone(),
            text: line.text.clone(),
            at_unix_ms: line
                .at
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

/// A daemon response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlResponse {
    /// Whether the command succeeded.
    pub ok: bool,
    /// Human-readable explanation, especially on failure.
    #[serde(default)]
    pub message: String,
    /// Fresh state snapshot, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RuntimeState>,
    /// Log entries for a logs request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<LogEntry>,
}

impl ControlResponse {
    /// A successful response carrying state.
    pub fn ok(state: RuntimeState, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            state: Some(state),
            logs: Vec::new(),
        }
    }

    /// A successful response with log lines.
    pub fn logs(state: RuntimeState, logs: Vec<LogEntry>) -> Self {
        Self {
            ok: true,
            message: String::new(),
            state: Some(state),
            logs,
        }
    }

    /// A failed response.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            state: None,
            logs: Vec::new(),
        }
    }
}

/// Sends one request to the daemon and reads one response.
pub async fn request(state: &RuntimeState, command: ControlCommand) -> Result<ControlResponse> {
    request_with_timeout(state, command, Duration::from_secs(8)).await
}

/// Sends one request with an explicit budget, for commands that may wait for a
/// graceful shutdown or a health check.
pub async fn request_with_timeout(
    state: &RuntimeState,
    command: ControlCommand,
    budget: Duration,
) -> Result<ControlResponse> {
    let payload = serde_json::to_vec(&ControlRequest::new(state, command))
        .context("could not encode the control request")?;
    let exchange = async {
        let mut stream = TcpStream::connect(("127.0.0.1", state.control_port))
            .await
            .with_context(|| {
                format!(
                    "could not connect to the Burrow daemon on 127.0.0.1:{}",
                    state.control_port
                )
            })?;
        stream
            .write_all(&payload)
            .await
            .context("could not send the control request")?;
        stream
            .write_all(b"\n")
            .await
            .context("could not send the control request")?;
        stream.flush().await.ok();

        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        let read = tokio::time::timeout(budget, reader.read_until(b'\n', &mut line))
            .await
            .context("the Burrow daemon did not answer in time")??;
        if read == 0 {
            anyhow::bail!("the Burrow daemon closed the connection without answering");
        }
        let text = String::from_utf8_lossy(&line);
        let response: ControlResponse =
            serde_json::from_str(text.trim()).context("the Burrow daemon sent an invalid reply")?;
        Ok(response)
    };

    tokio::time::timeout(budget + Duration::from_secs(1), exchange)
        .await
        .context("the Burrow daemon did not answer in time")?
}

/// Reads the state file and checks that its daemon is reachable.
pub async fn load_live_state(project_dir: &Path) -> Result<Option<RuntimeState>> {
    let Some(state) = RuntimeState::read(project_dir)? else {
        return Ok(None);
    };
    match request(&state, ControlCommand::Ping).await {
        Ok(response) if response.ok => Ok(Some(state)),
        _ => Ok(None),
    }
}

/// Returns true when something accepts TCP connections on `port`.
///
/// This distinguishes "the daemon is gone" from "something is listening but
/// did not authenticate", so callers never delete state that may belong to a
/// live daemon.
pub async fn reachable(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

/// Returns the current Unix time in milliseconds.
pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{ControlCommand, ControlRequest, ControlResponse, LogEntry};
    use crate::state::{RuntimeState, STATE_VERSION};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn state() -> RuntimeState {
        RuntimeState {
            version: STATE_VERSION,
            project: "shop".to_owned(),
            project_dir: PathBuf::from(r"C:\shop"),
            pid: 42,
            token: "secret-token".to_owned(),
            control_port: 45000,
            proxy_port: None,
            started_at: 1,
            config_hash: "hash".to_owned(),
            ready: true,
            services: BTreeMap::new(),
        }
    }

    #[test]
    fn requests_round_trip_as_one_json_line() {
        let request = ControlRequest::new(
            &state(),
            ControlCommand::Logs {
                service: Some("api".to_owned()),
                since_seq: 4,
                limit: 10,
            },
        );
        let encoded = serde_json::to_string(&request).expect("encode");
        assert!(!encoded.contains('\n'));
        let decoded: ControlRequest = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded.version, STATE_VERSION);
        assert_eq!(decoded.token, "secret-token");
    }

    #[test]
    fn public_state_json_never_contains_the_token() {
        let json = state().public_json().to_string();
        assert!(!json.contains("secret-token"), "{json}");
    }

    #[test]
    fn log_entries_serialize_cli_fields() {
        let entry = LogEntry {
            service: "api".to_owned(),
            seq: 1,
            stream: "stdout".to_owned(),
            text: "hello".to_owned(),
            at_unix_ms: 123,
        };
        let json = serde_json::to_value(&entry).expect("json");
        assert_eq!(json["service"], "api");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn error_responses_are_machine_readable() {
        let response = ControlResponse::error("no such service");
        let encoded = serde_json::to_string(&response).expect("encode");
        assert!(encoded.contains("\"ok\":false"));
        assert!(encoded.contains("no such service"));
    }
}
