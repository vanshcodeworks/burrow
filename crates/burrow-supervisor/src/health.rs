//! Health checks with retries, a timeout, and actionable errors.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The check to perform before dependents may start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCheck {
    /// Accept any TCP connection on this port.
    Tcp {
        /// Port to connect to on `127.0.0.1`.
        port: u16,
    },
    /// Send a plain HTTP GET and accept any non-5xx status.
    Http {
        /// Port to connect to on `127.0.0.1`.
        port: u16,
        /// Request path, for example `/health`.
        path: String,
    },
    /// Run a command through `cmd.exe /D /S /C`; exit code 0 means healthy.
    Command {
        /// Command line to run.
        command: String,
        /// Working directory for the check.
        cwd: Option<PathBuf>,
        /// Environment layered over Burrow's environment.
        env: BTreeMap<OsString, OsString>,
    },
}

/// Retry and timeout settings for [`wait_for_health`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthOptions {
    /// Total time to keep trying.
    pub timeout: Duration,
    /// Delay between attempts.
    pub interval: Duration,
    /// Maximum number of attempts. The timeout also applies.
    pub retries: u32,
}

impl Default for HealthOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            interval: Duration::from_millis(500),
            retries: 120,
        }
    }
}

/// A successful health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Number of attempts used, including the successful one.
    pub attempts: u32,
    /// Time from the first attempt to success.
    pub elapsed: Duration,
}

/// A failed health check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HealthError {
    /// The service did not become healthy within the configured budget.
    #[error("health check failed after {attempts} attempt(s) in {elapsed:?}: {last}")]
    TimedOut {
        /// Attempts performed.
        attempts: u32,
        /// Time spent.
        elapsed: Duration,
        /// Last underlying failure, shown to the user.
        last: String,
    },
    /// Retrying cannot help.
    #[error("health check is misconfigured: {0}")]
    Invalid(String),
}

/// Polls `check` until it succeeds, the retry budget is exhausted, or the
/// timeout expires.
pub async fn wait_for_health(
    check: &HealthCheck,
    options: HealthOptions,
) -> Result<HealthReport, HealthError> {
    if options.retries == 0 {
        return Err(HealthError::Invalid(
            "retries must be at least 1".to_owned(),
        ));
    }
    if options.timeout.is_zero() || options.interval.is_zero() {
        return Err(HealthError::Invalid(
            "timeout and interval must be greater than zero".to_owned(),
        ));
    }
    if let HealthCheck::Http { path, .. } = check {
        if !path.starts_with('/') {
            return Err(HealthError::Invalid(
                "HTTP health path must start with `/`".to_owned(),
            ));
        }
    }

    let started = Instant::now();
    let mut attempts = 0u32;
    let mut last = String::from("no attempt was made");

    while attempts < options.retries {
        let elapsed = started.elapsed();
        if elapsed >= options.timeout {
            break;
        }
        attempts += 1;

        let remaining = options.timeout.saturating_sub(elapsed);
        let attempt_timeout = options
            .interval
            .max(Duration::from_millis(250))
            .min(remaining);
        match tokio::time::timeout(attempt_timeout, run(check)).await {
            Ok(Ok(())) => {
                return Ok(HealthReport {
                    attempts,
                    elapsed: started.elapsed(),
                })
            }
            Ok(Err(message)) => last = message,
            Err(_) => last = "check attempt did not finish in time".to_owned(),
        }

        if started.elapsed() >= options.timeout {
            break;
        }
        tokio::time::sleep(options.interval).await;
    }

    Err(HealthError::TimedOut {
        attempts,
        elapsed: started.elapsed(),
        last,
    })
}

async fn run(check: &HealthCheck) -> Result<(), String> {
    match check {
        HealthCheck::Tcp { port } => connect(*port).await,
        HealthCheck::Http { port, path } => http_get(*port, path).await,
        HealthCheck::Command { command, cwd, env } => {
            command_check(command, cwd.as_deref(), env).await
        }
    }
}

async fn connect(port: u16) -> Result<(), String> {
    match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("could not connect to 127.0.0.1:{port}: {error}")),
    }
}

async fn http_get(port: u16, path: &str) -> Result<(), String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| format!("could not connect to 127.0.0.1:{port}: {error}"))?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("could not send the health request: {error}"))?;

    let mut response = Vec::with_capacity(1024);
    let mut buffer = [0u8; 1024];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                if response.windows(4).any(|window| window == b"\r\n\r\n")
                    || response.len() >= 16 * 1024
                {
                    break;
                }
            }
            Err(error) => {
                return Err(format!("could not read the health response: {error}"));
            }
        }
    }

    let headers = String::from_utf8_lossy(&response);
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "the server closed the connection without a response".to_owned())?;
    let mut parts = status_line.split_whitespace();
    let protocol = parts.next().unwrap_or_default();
    let status: u16 = parts
        .next()
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("malformed HTTP status line: `{status_line}`"))?;
    if !protocol.starts_with("HTTP/") {
        return Err(format!("malformed HTTP status line: `{status_line}`"));
    }
    if status >= 500 {
        return Err(format!("health endpoint returned HTTP {status}"));
    }
    Ok(())
}

async fn command_check(
    command: &str,
    cwd: Option<&Path>,
    env: &BTreeMap<OsString, OsString>,
) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("health command is empty".to_owned());
    }
    let mut process = tokio::process::Command::new("cmd.exe");
    process.args(["/D", "/S", "/C", command]);
    if let Some(cwd) = cwd {
        let cwd = crate::process_working_directory(cwd).map_err(|error| error.to_string())?;
        process.current_dir(cwd);
    }
    for (key, value) in env {
        process.env(key, value);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }

    match process.status().await {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("health command exited with {status}")),
        Err(error) => Err(format!("could not run the health command: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{wait_for_health, HealthCheck, HealthOptions};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn tcp_check_waits_for_a_listener() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        let accept = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let report = wait_for_health(
            &HealthCheck::Tcp { port },
            HealthOptions {
                timeout: Duration::from_secs(5),
                interval: Duration::from_millis(20),
                retries: 20,
            },
        )
        .await
        .expect("healthy");
        assert_eq!(report.attempts, 1);
        accept.await.expect("accept task");
    }

    #[tokio::test]
    async fn http_check_accepts_a_200_and_rejects_500() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        let server = tokio::spawn(async move {
            for status in ["200 OK", "500 Internal Server Error"] {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer).await.expect("read request");
                let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let options = HealthOptions {
            timeout: Duration::from_secs(3),
            interval: Duration::from_millis(20),
            retries: 1,
        };
        wait_for_health(
            &HealthCheck::Http {
                port,
                path: "/health".to_owned(),
            },
            options,
        )
        .await
        .expect("first response is healthy");

        let error = wait_for_health(
            &HealthCheck::Http {
                port,
                path: "/health".to_owned(),
            },
            options,
        )
        .await
        .expect_err("500 is unhealthy");
        assert!(error.to_string().contains("HTTP 500"), "{error}");
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn command_check_uses_the_exit_code() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let healthy = HealthCheck::Command {
            command: "exit 0".to_owned(),
            cwd: Some(cwd.clone()),
            env: BTreeMap::new(),
        };
        wait_for_health(
            &healthy,
            HealthOptions {
                timeout: Duration::from_secs(3),
                interval: Duration::from_millis(20),
                retries: 1,
            },
        )
        .await
        .expect("exit 0 is healthy");

        let unhealthy = HealthCheck::Command {
            command: "exit 3".to_owned(),
            cwd: Some(cwd),
            env: BTreeMap::new(),
        };
        let error = wait_for_health(
            &unhealthy,
            HealthOptions {
                timeout: Duration::from_secs(1),
                interval: Duration::from_millis(20),
                retries: 1,
            },
        )
        .await
        .expect_err("exit 3 is unhealthy");
        assert!(error.to_string().contains("exit"), "{error}");
    }

    #[tokio::test]
    async fn timeout_is_reported_with_the_last_error() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        drop(listener);

        let error = wait_for_health(
            &HealthCheck::Tcp { port },
            HealthOptions {
                timeout: Duration::from_millis(150),
                interval: Duration::from_millis(30),
                retries: 100,
            },
        )
        .await
        .expect_err("nothing is listening");
        assert!(matches!(error, super::HealthError::TimedOut { .. }));
    }
}
