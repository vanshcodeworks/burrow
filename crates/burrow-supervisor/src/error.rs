//! Error types for process supervision.

/// Errors returned by [`crate::ManagedProcess`] operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A Win32 call failed.
    #[error("{operation}: {source}")]
    Windows {
        /// The operation that failed, for example `CreateJobObjectW`.
        operation: &'static str,
        /// The underlying Win32 error.
        #[source]
        source: windows::core::Error,
    },

    /// The child process could not be created.
    #[error("could not start `{program}`: {source}")]
    Spawn {
        /// The program that was requested.
        program: String,
        /// The underlying Win32 error.
        #[source]
        source: windows::core::Error,
    },

    /// The process specification is invalid.
    #[error("invalid process specification: {0}")]
    InvalidSpec(&'static str),

    /// The dedicated wait thread could not be started.
    #[error("could not start the process wait thread: {0}")]
    WaitThread(#[source] std::io::Error),

    /// A per-service log file could not be opened.
    #[error("could not open the log file for `{service}`: {source}")]
    LogFile {
        /// Service name.
        service: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The wait thread stopped without reporting an exit status.
    #[error("the process wait channel closed without an exit status")]
    WaitClosed,

    /// Processes survived termination for longer than the allowed timeout.
    #[error("the process tree did not exit within {0:?}")]
    JobNotEmpty(std::time::Duration),
}

/// Convenience alias for supervisor results.
pub type Result<T> = std::result::Result<T, Error>;
