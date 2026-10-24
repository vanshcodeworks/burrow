//! Managed child processes: process specification, spawning, waiting, shutdown.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::windows::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::win::{self, Job, ProcessHandle, SpawnRequest};

const EMPTY_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const FORCE_TIMEOUT: Duration = Duration::from_secs(5);

/// Declarative description of one supervised process.
///
/// Use the builder methods rather than constructing this directly, so defaults
/// stay consistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// Human-readable service name. Used in logs and thread names.
    pub name: String,
    /// Executable to start.
    pub program: PathBuf,
    /// Arguments passed to the executable.
    pub args: Vec<OsString>,
    /// Working directory. `None` inherits Burrow's working directory.
    pub cwd: Option<PathBuf>,
    /// Environment variables layered over Burrow's environment.
    pub env: BTreeMap<OsString, OsString>,
    /// Job-wide committed-memory cap in bytes.
    pub memory_limit: Option<u64>,
    /// Optional hard CPU rate cap, as a percentage of one CPU.
    pub cpu_limit_percent: Option<u8>,
    /// How long to wait after `CTRL_BREAK` before terminating the tree.
    pub grace_period: Duration,
}

impl ProcessSpec {
    /// Creates a specification with a five second grace period and no limits.
    pub fn new(name: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            memory_limit: None,
            cpu_limit_percent: None,
            grace_period: Duration::from_secs(5),
        }
    }

    /// Appends one argument.
    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.args.push(argument.into());
        self
    }

    /// Appends several arguments.
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory.
    pub fn cwd(mut self, directory: impl Into<PathBuf>) -> Self {
        self.cwd = Some(directory.into());
        self
    }

    /// Adds or overrides one environment variable.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Sets the job-wide committed-memory cap.
    pub fn memory_limit_bytes(mut self, bytes: u64) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// Sets a hard CPU rate cap (1-100, percent of one CPU).
    pub fn cpu_limit_percent(mut self, percent: u8) -> Self {
        self.cpu_limit_percent = Some(percent);
        self
    }

    /// Sets how long graceful shutdown may take before the tree is terminated.
    pub fn grace_period(mut self, period: Duration) -> Self {
        self.grace_period = period;
        self
    }
}

/// What happened during [`ManagedProcess::shutdown`].
#[derive(Debug, Clone, PartialEq)]
pub enum ShutdownOutcome {
    /// The process had already exited before shutdown was requested.
    AlreadyExited {
        /// Final exit status.
        status: ExitStatus,
        /// Whether processes left in the job had to be terminated.
        leftover_terminated: bool,
    },
    /// The process exited within the grace period after `CTRL_BREAK`.
    Graceful {
        /// Final exit status.
        status: ExitStatus,
        /// Whether descendants left in the job had to be terminated.
        leftover_terminated: bool,
    },
    /// The process did not exit within the grace period and the job was terminated.
    Forced {
        /// Final exit status.
        status: ExitStatus,
        /// Whether a `CTRL_BREAK` event could be delivered to the process group.
        graceful_signal_sent: bool,
    },
}

/// A process tree managed in its own Windows Job Object.
///
/// Dropping a `ManagedProcess` closes the job handle. Because the job is
/// created with `KILL_ON_JOB_CLOSE`, that alone guarantees the whole tree is
/// terminated, even if Burrow crashes. Prefer [`ManagedProcess::shutdown`] for
/// an orderly stop.
pub struct ManagedProcess {
    job: Job,
    spec: ProcessSpec,
    process: Arc<ProcessHandle>,
    exit: watch::Receiver<Option<ExitStatus>>,
}

impl ManagedProcess {
    /// Starts a process tree inside a new job object.
    ///
    /// Must be called from within a Tokio runtime: the stdout/stderr forwarders
    /// are Tokio tasks.
    pub async fn spawn(spec: ProcessSpec) -> Result<Self> {
        if spec.name.trim().is_empty() {
            return Err(Error::InvalidSpec("service name must not be empty"));
        }
        if spec.program.as_os_str().is_empty() {
            return Err(Error::InvalidSpec("program path must not be empty"));
        }

        let job = Job::create(spec.memory_limit, spec.cpu_limit_percent)?;
        let request = SpawnRequest {
            program: spec.program.clone().into_os_string(),
            args: spec.args.clone(),
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
        };
        let spawned = win::spawn_in_job(&job, &request)?;
        let pid = spawned.process.pid();
        let stdout = tokio::fs::File::from_std(spawned.stdout);
        let stderr = tokio::fs::File::from_std(spawned.stderr);
        let process = Arc::new(spawned.process);

        let (exit_tx, exit) = watch::channel::<Option<ExitStatus>>(None);
        let waiter = Arc::clone(&process);
        let wait_name = spec.name.clone();
        std::thread::Builder::new()
            .name(format!("burrow-wait-{wait_name}"))
            .spawn(move || match waiter.wait() {
                Ok(code) => {
                    let _ = exit_tx.send(Some(ExitStatus::from_raw(code)));
                }
                Err(error) => {
                    warn!(service = %wait_name, %error, "process wait failed");
                }
            })
            .map_err(Error::WaitThread)?;

        // The forwarders own the pipe read ends and finish at EOF. They are
        // detached deliberately: a service may exit while a grandchild keeps a
        // pipe open, and shutdown must not wait for that.
        let stdout_name = spec.name.clone();
        tokio::spawn(async move {
            forward_output(stdout, stdout_name, "stdout").await;
        });
        let stderr_name = spec.name.clone();
        tokio::spawn(async move {
            forward_output(stderr, stderr_name, "stderr").await;
        });

        debug!(service = %spec.name, pid, "process started");
        Ok(Self {
            job,
            spec,
            process,
            exit,
        })
    }

    /// The operating-system process id of the direct child.
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// The service name from the specification.
    pub fn name(&self) -> &str {
        &self.spec.name
    }

    /// Returns the exit status if the process has already exited.
    pub fn try_wait(&self) -> Option<ExitStatus> {
        *self.exit.borrow()
    }

    /// Waits for the direct child to exit.
    ///
    /// This does not wait for descendants. Use [`Self::shutdown`] to guarantee
    /// the whole tree is gone.
    pub async fn wait(&self) -> Result<ExitStatus> {
        let mut receiver = self.exit.clone();
        loop {
            if let Some(status) = *receiver.borrow() {
                return Ok(status);
            }
            receiver.changed().await.map_err(|_| Error::WaitClosed)?;
        }
    }

    /// Stops the whole tree.
    ///
    /// Sends `CTRL_BREAK` to the service's process group, waits up to `grace`
    /// (default: [`ProcessSpec::grace_period`]) for a clean exit, then
    /// terminates the job. Any descendants still in the job are terminated in
    /// either case, so this returns only once the job is empty.
    pub async fn shutdown(&mut self, grace: Option<Duration>) -> Result<ShutdownOutcome> {
        if let Some(status) = self.try_wait() {
            let leftover_terminated = !self.wait_job_empty(DRAIN_TIMEOUT).await?;
            if leftover_terminated {
                self.job.terminate(1)?;
                self.ensure_empty(FORCE_TIMEOUT).await?;
            }
            return Ok(ShutdownOutcome::AlreadyExited {
                status,
                leftover_terminated,
            });
        }

        let grace = grace.unwrap_or(self.spec.grace_period);
        let signal = win::send_ctrl_break(self.pid());
        let graceful_signal_sent = signal.is_ok();
        if let Err(error) = signal {
            debug!(
                service = %self.spec.name,
                %error,
                "console control event unavailable; forced termination will be used"
            );
        }

        if graceful_signal_sent && !grace.is_zero() {
            match tokio::time::timeout(grace, self.wait()).await {
                Ok(status) => {
                    let status = status?;
                    let leftover_terminated = !self.wait_job_empty(DRAIN_TIMEOUT).await?;
                    if leftover_terminated {
                        self.job.terminate(1)?;
                        self.ensure_empty(FORCE_TIMEOUT).await?;
                    }
                    return Ok(ShutdownOutcome::Graceful {
                        status,
                        leftover_terminated,
                    });
                }
                Err(_) => {
                    debug!(
                        service = %self.spec.name,
                        "grace period elapsed; terminating job"
                    );
                }
            }
        }

        self.job.terminate(1)?;
        let status = self.wait().await?;
        self.ensure_empty(FORCE_TIMEOUT).await?;
        Ok(ShutdownOutcome::Forced {
            status,
            graceful_signal_sent,
        })
    }

    /// Terminates the whole tree immediately.
    pub async fn kill(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.try_wait() {
            if !self.wait_job_empty(Duration::ZERO).await? {
                self.job.terminate(1)?;
            }
            self.ensure_empty(FORCE_TIMEOUT).await?;
            return Ok(status);
        }
        self.job.terminate(1)?;
        let status = self.wait().await?;
        self.ensure_empty(FORCE_TIMEOUT).await?;
        Ok(status)
    }

    async fn wait_job_empty(&self, timeout: Duration) -> Result<bool> {
        let started = Instant::now();
        loop {
            if self.job.active_processes()? == 0 {
                return Ok(true);
            }
            if started.elapsed() >= timeout {
                return Ok(false);
            }
            tokio::time::sleep(EMPTY_POLL_INTERVAL).await;
        }
    }

    async fn ensure_empty(&self, timeout: Duration) -> Result<()> {
        if self.wait_job_empty(timeout).await? {
            Ok(())
        } else {
            Err(Error::JobNotEmpty(timeout))
        }
    }
}

async fn forward_output<R>(reader: R, service: String, stream: &'static str)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match lines.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {
                while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                    buffer.pop();
                }
                let line = String::from_utf8_lossy(&buffer);
                info!(target: "burrow::service", service = %service, stream = stream, "{line}");
            }
            Err(error) => {
                debug!(service = %service, stream = stream, %error, "log stream ended with an error");
                break;
            }
        }
    }
}
