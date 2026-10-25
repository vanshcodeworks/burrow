//! Per-project data directories, initialization, and one-shot seeding.
//!
//! A managed service's data lives in `.burrow/data/<service>/`. Two marker
//! files make the lifecycle idempotent:
//!
//! * `.burrow-initialized.json` is written after every registry `init` step
//!   succeeds. It is removed with the data directory, so `burrow reset` starts
//!   from a clean cluster.
//! * `.burrow-seeded.json` records the seed script hash after the seed command
//!   succeeds. Seeding runs exactly once; changing the script does not re-run
//!   it, because that could corrupt existing rows. `burrow reset` is the
//!   explicit way to reseed.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod snapshot;

pub use snapshot::{
    create_snapshot, list_snapshots, read_manifest, recover_interrupted_restore, restore_snapshot,
    snapshot_dir, snapshots_dir, validate_name, RestoreRecovery, RestoreReport, SnapshotManifest,
    SnapshotService, MANIFEST_FILE, SNAPSHOT_DIR, SNAPSHOT_VERSION,
};

/// Marker written after every `init` command has succeeded.
pub const INIT_MARKER: &str = ".burrow-initialized.json";
/// Marker written after the seed script has run.
pub const SEED_MARKER: &str = ".burrow-seeded.json";

const MARKER_VERSION: u32 = 1;

/// A data-management failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read or written.
    #[error("could not access {}: {source}", path.display())]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A marker file is not valid JSON.
    #[error("{} is not a valid Burrow data marker: {message}", path.display())]
    Marker {
        /// Marker path.
        path: PathBuf,
        /// Parse failure.
        message: String,
    },
    /// A snapshot name is not safe to use as a directory name.
    #[error(
        "invalid snapshot name `{0}`: use 1-64 lowercase letters, digits, `-`, `_`, or `.`, starting with a letter or digit"
    )]
    InvalidSnapshotName(String),
    /// A service name in a snapshot manifest is not safe as a directory name.
    #[error("invalid service name `{0}` in snapshot manifest")]
    InvalidServiceName(String),
    /// The named snapshot does not exist.
    #[error("snapshot `{0}` does not exist")]
    SnapshotNotFound(String),
    /// A snapshot with that name already exists.
    #[error("snapshot `{0}` already exists; choose another name or delete the old snapshot")]
    SnapshotExists(String),
    /// A snapshot manifest is invalid or does not match its directory.
    #[error("{} is not a usable snapshot manifest: {message}", path.display())]
    Manifest {
        /// Manifest path.
        path: PathBuf,
        /// Explanation.
        message: String,
    },
    /// A restore journal is invalid or incomplete.
    #[error("{} is not a usable restore journal: {message}", path.display())]
    RestoreJournal {
        /// Journal path.
        path: PathBuf,
        /// Explanation.
        message: String,
    },
    /// A symlink or reparse point was found inside a data directory.
    #[error(
        "refusing to snapshot reparse point {}; move it outside the service data directory",
        path.display()
    )]
    ReparsePoint {
        /// Path of the reparse point.
        path: PathBuf,
    },
}

impl Error {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Result alias for data operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Initialization marker contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitState {
    /// Marker schema.
    pub version: u32,
    /// Unix timestamp.
    pub initialized_at: u64,
}

/// Seed marker contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedState {
    /// Marker schema.
    pub version: u32,
    /// SHA-256 of the seed script that ran.
    pub seed_sha256: String,
    /// Unix timestamp.
    pub seeded_at: u64,
}

/// Returns the data directory for a service under `burrow_dir`.
pub fn data_dir(burrow_dir: &Path, service: &str) -> PathBuf {
    burrow_dir.join("data").join(service)
}

/// Returns true when the service still needs its registry `init` commands.
///
/// The marker is the authority: a directory populated by a successful run does
/// not need initialization again, and a partially initialized directory is
/// retried because its marker was never written.
pub fn needs_init(data_dir: &Path) -> bool {
    !data_dir.join(INIT_MARKER).is_file()
}

/// Writes the initialization marker atomically.
pub fn mark_initialized(data_dir: &Path) -> Result<()> {
    let state = InitState {
        version: MARKER_VERSION,
        initialized_at: unix_now(),
    };
    write_marker(data_dir, INIT_MARKER, &state)
}

/// Returns true when a seed script has never completed for this data directory.
pub fn needs_seed(data_dir: &Path) -> bool {
    read_seed_state(data_dir).is_none()
}

/// Reads the seed marker, if present and valid.
pub fn read_seed_state(data_dir: &Path) -> Option<SeedState> {
    let path = data_dir.join(SEED_MARKER);
    let text = std::fs::read_to_string(path).ok()?;
    let state: SeedState = serde_json::from_str(&text).ok()?;
    (state.version == MARKER_VERSION && state.seed_sha256.len() == 64).then_some(state)
}

/// Writes the seed marker atomically.
pub fn mark_seeded(data_dir: &Path, seed_sha256: &str) -> Result<()> {
    let state = SeedState {
        version: MARKER_VERSION,
        seed_sha256: seed_sha256.to_owned(),
        seeded_at: unix_now(),
    };
    write_marker(data_dir, SEED_MARKER, &state)
}

fn write_marker<T: Serialize>(data_dir: &Path, name: &str, value: &T) -> Result<()> {
    std::fs::create_dir_all(data_dir).map_err(|error| Error::io(data_dir, error))?;
    let path = data_dir.join(name);
    let temporary = path.with_extension("tmp");
    let encoded = serde_json::to_vec_pretty(value).map_err(|error| Error::Marker {
        path: path.clone(),
        message: error.to_string(),
    })?;
    std::fs::write(&temporary, encoded).map_err(|error| Error::io(&temporary, error))?;
    std::fs::rename(&temporary, &path).map_err(|error| Error::io(&path, error))?;
    Ok(())
}

/// Computes the SHA-256 of a seed script.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| Error::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| Error::io(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Removes one service's data directory. Used by `burrow reset`.
pub fn reset(data_dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(data_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(data_dir, error)),
    }
}

/// Moves an uninitialized data directory aside before a registry `init` retry.
///
/// A killed `initdb` can leave a non-empty directory without the marker, and
/// `initdb` refuses to reuse it. Moving it to
/// `.burrow/data/.burrow-init-failed-<service>-<pid>-<unix>` lets the retry
/// start clean without deleting whatever the partial run produced. A directory
/// with a valid init marker is never moved.
pub fn quarantine_uninitialized(data_dir: &Path) -> Result<Option<PathBuf>> {
    if !data_dir.exists() || data_dir.join(INIT_MARKER).is_file() {
        return Ok(None);
    }
    // An empty directory is already safe for `initdb` to use, so leave it
    // alone instead of accumulating empty quarantine directories.
    let mut entries = std::fs::read_dir(data_dir).map_err(|error| Error::io(data_dir, error))?;
    if entries.next().is_none() {
        return Ok(None);
    }
    let parent = data_dir.parent().unwrap_or_else(|| Path::new("."));
    let name = data_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "service".to_owned());
    let quarantine = parent.join(format!(
        ".burrow-init-failed-{name}-{}-{}",
        std::process::id(),
        unix_now()
    ));
    std::fs::rename(data_dir, &quarantine).map_err(|error| Error::io(data_dir, error))?;
    Ok(Some(quarantine))
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        hash_file, mark_initialized, mark_seeded, needs_init, needs_seed, quarantine_uninitialized,
        read_seed_state, reset,
    };

    #[test]
    fn init_marker_is_required_and_reset_removes_everything() {
        let directory = tempfile::tempdir().expect("temp");
        let data = directory.path().join("data");
        assert!(needs_init(&data));
        std::fs::create_dir_all(&data).expect("create data");
        assert!(
            needs_init(&data),
            "an existing directory alone is not initialized"
        );

        mark_initialized(&data).expect("mark");
        assert!(!needs_init(&data));
        assert!(data.join(super::INIT_MARKER).is_file());

        reset(&data).expect("reset");
        assert!(!data.exists());
        assert!(needs_init(&data));
    }

    #[test]
    fn seeding_runs_once_and_records_the_script_hash() {
        let directory = tempfile::tempdir().expect("temp");
        let data = directory.path().join("data");
        let seed = directory.path().join("seed.sql");
        std::fs::write(&seed, "CREATE TABLE t (id int);").expect("seed");

        let hash = hash_file(&seed).expect("hash");
        assert!(needs_seed(&data));
        mark_seeded(&data, &hash).expect("mark");
        assert!(!needs_seed(&data));
        assert_eq!(read_seed_state(&data).expect("state").seed_sha256, hash);

        // A changed seed script does not re-run automatically.
        std::fs::write(&seed, "CREATE TABLE t (id bigint);").expect("rewrite seed");
        assert!(!needs_seed(&data));
    }

    #[test]
    fn invalid_markers_are_treated_as_missing() {
        let directory = tempfile::tempdir().expect("temp");
        let data = directory.path().join("data");
        std::fs::create_dir_all(&data).expect("create");
        std::fs::write(data.join(super::SEED_MARKER), "not json").expect("write");
        assert!(needs_seed(&data));
        assert!(read_seed_state(&data).is_none());
    }

    #[test]
    fn a_partial_init_directory_is_moved_aside_not_deleted() {
        let directory = tempfile::tempdir().expect("temp");
        let data = directory.path().join("data").join("db");
        std::fs::create_dir_all(&data).expect("create partial data");
        std::fs::write(data.join("partial.txt"), "half an initdb\n").expect("write partial");

        let quarantine = quarantine_uninitialized(&data)
            .expect("quarantine")
            .expect("partial directory must be moved");
        assert!(!data.exists(), "the retry must get a clean data directory");
        assert_eq!(
            std::fs::read_to_string(quarantine.join("partial.txt")).expect("read partial"),
            "half an initdb\n",
            "the partial run must be preserved"
        );
        assert!(
            quarantine
                .file_name()
                .expect("quarantine name")
                .to_string_lossy()
                .starts_with(".burrow-init-failed-db-"),
            "unexpected quarantine path: {}",
            quarantine.display()
        );

        std::fs::create_dir_all(&data).expect("recreate data");
        mark_initialized(&data).expect("mark initialized");
        assert!(
            quarantine_uninitialized(&data)
                .expect("quarantine initialized")
                .is_none(),
            "an initialized data directory must never be moved"
        );
        assert!(data.join(super::INIT_MARKER).is_file());
    }

    #[test]
    fn an_empty_uninitialized_directory_is_left_in_place() {
        let directory = tempfile::tempdir().expect("temp");
        let data = directory.path().join("data").join("db");
        std::fs::create_dir_all(&data).expect("create empty data");

        assert!(
            quarantine_uninitialized(&data)
                .expect("quarantine empty directory")
                .is_none(),
            "an empty directory is safe for initdb and must not be moved"
        );
        assert!(data.is_dir());
    }
}
