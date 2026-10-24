//! Process supervision for Burrow.
//!
//! This crate owns the part of Burrow that must be correct above all else:
//! starting a process tree inside a Windows Job Object, enforcing resource
//! limits, and guaranteeing that nothing is left running when Burrow stops.
//!
//! # Platform
//!
//! Windows only. The Win32 calls are confined to the private [`win`] module;
//! no other module in the workspace contains `unsafe` code.
//!
//! # Current scope (M1)
//!
//! * A job object per service with `KILL_ON_JOB_CLOSE`.
//! * Race-free job assignment at process creation (no post-spawn window in
//!   which a child could escape the job).
//! * Job-wide and per-process memory commit limits.
//! * Optional hard CPU rate cap.
//! * stdout/stderr forwarding to `tracing`.
//! * Graceful shutdown: `CTRL_BREAK` to the service's process group, a grace
//!   period, then `TerminateJobObject`.
//!
//! Restart policies, health checks, log files, and state files arrive in M2.
//!
//! # Resource limits are not a security boundary
//!
//! Job Object limits control resource consumption. They do not sandbox the
//! process: a service can still read user files and make network connections.

#![cfg_attr(not(windows), allow(unused))]
#![warn(missing_docs)]

#[cfg(not(windows))]
compile_error!("burrow-supervisor currently supports Windows only");

mod error;
mod health;
mod logs;
mod process;
mod restart;
mod win;

pub use error::{Error, Result};
pub use health::{wait_for_health, HealthCheck, HealthError, HealthOptions, HealthReport};
pub use logs::{LogLine, LogRing, DEFAULT_LOG_MAX_BYTES, DEFAULT_RING_CAPACITY};
pub use process::{ManagedProcess, ProcessSpec, ShutdownOutcome};
pub use restart::{RestartBackoff, RestartDecision, DEFAULT_MAX_FAILURES, DEFAULT_WINDOW};
pub use win::JobStats;
