//! Package management for Burrow.
//!
//! Reads the built-in registry, performs SHA-256-verified downloads into a
//! shared content-addressed cache, resumes interrupted downloads, and extracts
//! archives with zip-slip and path-traversal hardening. The runtime uses the
//! extracted directory to run a managed service such as PostgreSQL.

pub mod builtin;
pub mod cache;
pub mod error;
pub mod extract;
pub mod registry;

pub use cache::{sha256_file, Cache, DownloadProgress, GcReport, Marker, PackageRef, VerifyReport};
pub use error::{Error, Result};
pub use registry::{
    ArchiveKind, Entry, InitStep, InitWhen, PackageHealth, PackageVersion, Platform, Registry,
    Resolved, TemplateCommand, REGISTRY_SCHEMA, WINDOWS_X86_64,
};
