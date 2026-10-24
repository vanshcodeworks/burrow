//! Error types for registry loading, downloads, verification, and extraction.

use std::path::PathBuf;

/// A package-manager failure with an actionable message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No registry entry has this name.
    #[error("unknown package `{name}`; available packages: {available}")]
    UnknownPackage {
        /// Requested name.
        name: String,
        /// Comma-separated names that do exist.
        available: String,
    },

    /// The entry exists but no version matches the request.
    #[error(
        "package `{name}` has no version matching `{requested}`; available versions: {available}"
    )]
    UnknownVersion {
        /// Package name.
        name: String,
        /// Version or prefix the user asked for.
        requested: String,
        /// Comma-separated versions from the registry.
        available: String,
    },

    /// The entry deliberately has no native Windows build.
    #[error("package `{name}` is not supported natively: {reason}")]
    UnsupportedNative {
        /// Package name.
        name: String,
        /// Why there is no trustworthy native build.
        reason: String,
    },

    /// A registry document is malformed or inconsistent.
    #[error("invalid registry entry `{name}`: {message}")]
    InvalidRegistry {
        /// Entry name, or the file name when parsing failed early.
        name: String,
        /// What is wrong.
        message: String,
    },

    /// A file or directory could not be read, written, or renamed.
    #[error("could not access {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The HTTP request failed before a complete body was received.
    #[error("download of {url} failed: {source}")]
    Download {
        /// Artifact URL.
        url: String,
        /// Underlying HTTP error.
        #[source]
        source: reqwest::Error,
    },

    /// The server sent fewer bytes than the registry records.
    #[error("download of {url} was incomplete: expected {expected} bytes, got {actual}")]
    IncompleteDownload {
        /// Artifact URL.
        url: String,
        /// Registry size.
        expected: u64,
        /// Bytes received.
        actual: u64,
    },

    /// A downloaded archive did not match its recorded SHA-256.
    #[error(
        "SHA-256 mismatch for {url}: expected {expected}, got {actual}; the download was deleted"
    )]
    HashMismatch {
        /// Artifact URL, or the archive path for local verification.
        url: String,
        /// Registry hash.
        expected: String,
        /// Observed hash.
        actual: String,
    },

    /// An archive entry tried to escape the destination or use a reserved name.
    #[error("archive entry `{entry}` is unsafe: {reason}")]
    UnsafeEntry {
        /// Entry name as recorded in the archive.
        entry: String,
        /// Why it was rejected.
        reason: String,
    },

    /// The archive itself is malformed or exceeds a safety limit.
    #[error("archive is not valid: {message}")]
    Archive {
        /// Parser or limit error.
        message: String,
    },

    /// The server answered with an unexpected status.
    #[error("download of {url} failed with HTTP {status}")]
    HttpStatus {
        /// Artifact URL.
        url: String,
        /// Numeric status code.
        status: u16,
    },

    /// Another process held the download lock for too long.
    #[error("timed out waiting for {}; another Burrow process may be downloading the same package", path.display())]
    LockTimeout {
        /// Lock file that could not be acquired.
        path: PathBuf,
    },

    /// A cache document (marker or reference file) is malformed.
    #[error("invalid cache document {}: {message}", path.display())]
    CorruptCache {
        /// File that could not be understood.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },

    /// The cache directory cannot be determined.
    #[error("could not determine the cache directory: set BURROW_CACHE_DIR or LOCALAPPDATA")]
    NoCacheDirectory,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Result alias for package operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
