//! Snapshot, restore, and reset support for service data directories.
//!
//! A snapshot is a full copy of every configured service's data directory under
//! `.burrow/snapshots/<name>/`. Snapshots are built in a temporary directory on
//! the same volume and renamed into place only after every file has been copied
//! and the manifest has been written, so a failed snapshot never appears in
//! `burrow snapshots`.
//!
//! Restoring copies the snapshot into a staging directory first, then swaps one
//! service directory at a time with `rename`. Each service directory is either
//! the old one or the restored one, never a partially copied mix. If a swap
//! fails, directories that were already replaced are rolled back.
//!
//! Snapshots intentionally do not understand database formats. The CLI requires
//! the project daemon to be stopped, which makes a plain directory copy safe for
//! the managed services Burrow supports.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Directory below `.burrow` that holds snapshots.
pub const SNAPSHOT_DIR: &str = "snapshots";
/// File name of the manifest inside one snapshot.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Manifest schema version.
pub const SNAPSHOT_VERSION: u32 = 1;
/// Directory below `.burrow` that holds service data.
const DATA_DIR: &str = "data";
const TEMP_PREFIX: &str = ".burrow-tmp-";
const RESTORE_PREFIX: &str = ".burrow-restore-";
const BACKUP_PREFIX: &str = ".burrow-backup-";

/// One service captured by a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotService {
    /// Service name.
    pub name: String,
    /// Whether its data directory existed when the snapshot was created.
    pub present: bool,
    /// Number of regular files copied.
    pub files: u64,
    /// Total bytes copied.
    pub bytes: u64,
}

/// The manifest stored at the root of every snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Manifest schema version.
    pub version: u32,
    /// Snapshot name.
    pub name: String,
    /// Unix seconds when the snapshot finished copying.
    pub created_at: u64,
    /// Services in the project at snapshot time.
    pub services: Vec<SnapshotService>,
}

impl SnapshotManifest {
    /// Total bytes across every captured data directory.
    pub fn total_bytes(&self) -> u64 {
        self.services.iter().map(|service| service.bytes).sum()
    }

    /// Total files across every captured data directory.
    pub fn total_files(&self) -> u64 {
        self.services.iter().map(|service| service.files).sum()
    }
}

/// Result of a completed restore.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    /// Snapshot name.
    pub name: String,
    /// Service data directories that were replaced or removed.
    pub services: Vec<String>,
}

/// Returns the snapshot root for a project.
pub fn snapshots_dir(burrow_dir: &Path) -> PathBuf {
    burrow_dir.join(SNAPSHOT_DIR)
}

/// Returns the directory for one named snapshot.
pub fn snapshot_dir(burrow_dir: &Path, name: &str) -> PathBuf {
    snapshots_dir(burrow_dir).join(name)
}

/// Validates a user-supplied snapshot name.
///
/// Names become directory names and command-line arguments, so path
/// separators, drive prefixes, and dot-only names are rejected.
pub fn validate_name(name: &str) -> Result<()> {
    let valid_length = !name.is_empty() && name.len() <= 64;
    let valid_start = name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let valid_characters = name.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_' | '.')
    });
    if valid_length && valid_start && valid_characters {
        Ok(())
    } else {
        Err(Error::InvalidSnapshotName(name.to_owned()))
    }
}

fn validate_service_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidServiceName(name.to_owned()))
    }
}

/// Creates a snapshot of `services` under `.burrow/snapshots/<name>/`.
///
/// The caller must stop the project first. Directories are copied with
/// `symlink_metadata`; reparse points and symlinks are rejected rather than
/// followed, so a snapshot can never silently capture data outside the project.
pub fn create_snapshot(
    burrow_dir: &Path,
    name: &str,
    services: &[String],
) -> Result<SnapshotManifest> {
    validate_name(name)?;
    let root = snapshots_dir(burrow_dir);
    fs::create_dir_all(&root).map_err(|error| Error::io(&root, error))?;

    let final_dir = root.join(name);
    if final_dir.exists() {
        return Err(Error::SnapshotExists(name.to_owned()));
    }

    let nonce = format!("{}-{}", std::process::id(), super::unix_now());
    let temporary = root.join(format!("{TEMP_PREFIX}{name}-{nonce}"));
    if temporary.exists() {
        fs::remove_dir_all(&temporary).map_err(|error| Error::io(&temporary, error))?;
    }

    let result = build_snapshot(burrow_dir, name, services, &temporary);
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
    };

    match fs::rename(&temporary, &final_dir) {
        Ok(()) => Ok(manifest),
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(Error::io(&final_dir, error))
        }
    }
}

fn build_snapshot(
    burrow_dir: &Path,
    name: &str,
    services: &[String],
    temporary: &Path,
) -> Result<SnapshotManifest> {
    fs::create_dir_all(temporary).map_err(|error| Error::io(temporary, error))?;
    let mut seen = BTreeSet::new();
    let mut captured = Vec::new();
    for service in services {
        validate_service_name(service)?;
        if !seen.insert(service.clone()) {
            continue;
        }
        let source = burrow_dir.join(DATA_DIR).join(service);
        let destination = temporary.join(DATA_DIR).join(service);
        let (files, bytes) = if source.is_dir() {
            copy_tree(&source, &destination)?
        } else {
            (0, 0)
        };
        captured.push(SnapshotService {
            name: service.clone(),
            present: source.is_dir(),
            files,
            bytes,
        });
    }

    let manifest = SnapshotManifest {
        version: SNAPSHOT_VERSION,
        name: name.to_owned(),
        created_at: super::unix_now(),
        services: captured,
    };
    let manifest_path = temporary.join(MANIFEST_FILE);
    let encoded = serde_json::to_vec_pretty(&manifest).map_err(|error| Error::Manifest {
        path: manifest_path.clone(),
        message: error.to_string(),
    })?;
    fs::write(&manifest_path, encoded).map_err(|error| Error::io(&manifest_path, error))?;
    Ok(manifest)
}

/// Lists snapshots, newest first. Entries without a valid manifest are an
/// error: silently hiding them would make a broken snapshot look lost.
pub fn list_snapshots(burrow_dir: &Path) -> Result<Vec<SnapshotManifest>> {
    let root = snapshots_dir(burrow_dir);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(&root, error)),
    };

    let mut snapshots = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(&root, error))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with(TEMP_PREFIX) {
            continue;
        }
        snapshots.push(read_manifest(&path)?);
    }
    snapshots.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(snapshots)
}

/// Reads and validates one snapshot manifest.
pub fn read_manifest(snapshot_dir: &Path) -> Result<SnapshotManifest> {
    let path = snapshot_dir.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
    let manifest: SnapshotManifest =
        serde_json::from_str(&text).map_err(|error| Error::Manifest {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if manifest.version != SNAPSHOT_VERSION {
        return Err(Error::Manifest {
            path,
            message: format!(
                "unsupported snapshot version {} (expected {SNAPSHOT_VERSION})",
                manifest.version
            ),
        });
    }
    Ok(manifest)
}

/// Restores a snapshot, replacing the data directory of every service it
/// contains.
///
/// The caller must stop the project first. The restored manifest is authoritative
/// for which services are touched: services added to `burrow.toml` after the
/// snapshot was taken are left alone.
pub fn restore_snapshot(burrow_dir: &Path, name: &str) -> Result<RestoreReport> {
    validate_name(name)?;
    let source = snapshot_dir(burrow_dir, name);
    if !source.is_dir() {
        return Err(Error::SnapshotNotFound(name.to_owned()));
    }
    let manifest = read_manifest(&source)?;
    if manifest.name != name {
        return Err(Error::Manifest {
            path: source.join(MANIFEST_FILE),
            message: format!(
                "snapshot directory `{name}` contains manifest for `{}`",
                manifest.name
            ),
        });
    }

    let data_root = burrow_dir.join(DATA_DIR);
    fs::create_dir_all(&data_root).map_err(|error| Error::io(&data_root, error))?;
    let nonce = format!("{}-{}", std::process::id(), super::unix_now());
    let staging = data_root.join(format!("{RESTORE_PREFIX}{nonce}"));
    fs::create_dir_all(&staging).map_err(|error| Error::io(&staging, error))?;

    // Stage every present service before touching live data.
    let mut present = Vec::new();
    for service in &manifest.services {
        validate_service_name(&service.name)?;
        if !service.present {
            continue;
        }
        let from = source.join(DATA_DIR).join(&service.name);
        if !from.is_dir() {
            let _ = fs::remove_dir_all(&staging);
            return Err(Error::Manifest {
                path: source.join(MANIFEST_FILE),
                message: format!(
                    "service `{}` is marked present but its data is missing",
                    service.name
                ),
            });
        }
        let to = staging.join(&service.name);
        if let Err(error) = copy_tree(&from, &to) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        present.push(service.name.clone());
    }

    let mut completed = Vec::new();
    for service in &manifest.services {
        let target = data_root.join(&service.name);
        let backup = data_root.join(format!("{BACKUP_PREFIX}{nonce}-{}", service.name));
        let had_original = target.is_dir();
        if had_original {
            if let Err(error) = fs::rename(&target, &backup) {
                rollback(&completed);
                let _ = fs::remove_dir_all(&staging);
                return Err(Error::io(&target, error));
            }
        }

        let mut installed = false;
        if service.present {
            let staged = staging.join(&service.name);
            if let Err(error) = fs::rename(&staged, &target) {
                if had_original {
                    let _ = fs::rename(&backup, &target);
                }
                rollback(&completed);
                let _ = fs::remove_dir_all(&staging);
                return Err(Error::io(&target, error));
            }
            installed = true;
        }
        completed.push(Completed {
            service: service.name.clone(),
            backup,
            had_original,
            installed,
        });
    }

    for entry in &completed {
        let _ = fs::remove_dir_all(&entry.backup);
    }
    let _ = fs::remove_dir_all(&staging);

    Ok(RestoreReport {
        name: name.to_owned(),
        services: manifest
            .services
            .iter()
            .map(|service| service.name.clone())
            .collect(),
    })
}

struct Completed {
    service: String,
    backup: PathBuf,
    had_original: bool,
    installed: bool,
}

fn rollback(completed: &[Completed]) {
    for entry in completed.iter().rev() {
        let data_root = entry.backup.parent().unwrap_or_else(|| Path::new("."));
        let target = data_root.join(&entry.service);
        if entry.installed {
            let _ = fs::remove_dir_all(&target);
        }
        if entry.had_original {
            let _ = fs::rename(&entry.backup, &target);
        }
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(u64, u64)> {
    fs::create_dir_all(destination).map_err(|error| Error::io(destination, error))?;
    let mut files = 0;
    let mut bytes = 0;
    let entries = fs::read_dir(source).map_err(|error| Error::io(source, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(source, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| Error::io(&path, error))?;
        if is_reparse_point(&metadata) {
            return Err(Error::ReparsePoint { path });
        }
        let destination_path = destination.join(entry.file_name());
        if metadata.is_dir() {
            let (child_files, child_bytes) = copy_tree(&path, &destination_path)?;
            files += child_files;
            bytes += child_bytes;
        } else if metadata.is_file() {
            fs::copy(&path, &destination_path).map_err(|error| Error::io(&path, error))?;
            files += 1;
            bytes += metadata.len();
        }
    }
    Ok((files, bytes))
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::{create_snapshot, list_snapshots, restore_snapshot, snapshot_dir, validate_name};

    fn service(name: &str) -> String {
        name.to_owned()
    }

    #[test]
    fn snapshot_names_cannot_escape_the_snapshot_directory() {
        for name in ["", "..", "../x", "a/b", "A", ".hidden", "-start"] {
            assert!(validate_name(name).is_err(), "`{name}` must be rejected");
        }
        for name in ["a", "nightly", "before_migration", "v1.2"] {
            assert!(validate_name(name).is_ok(), "`{name}` must be accepted");
        }
    }

    #[test]
    fn snapshot_round_trip_preserves_files_and_absent_services() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let db = burrow.join("data").join("db");
        std::fs::create_dir_all(&db).expect("create data");
        std::fs::write(db.join("PG_VERSION"), "16\n").expect("write version");
        std::fs::create_dir_all(db.join("base")).expect("create nested");
        std::fs::write(db.join("base").join("1"), vec![7u8; 1024]).expect("write nested");

        let manifest =
            create_snapshot(&burrow, "before", &[service("db"), service("api")]).expect("snapshot");
        assert_eq!(manifest.services.len(), 2);
        assert_eq!(manifest.services[0].files, 2);
        assert_eq!(manifest.services[0].bytes, 1027);
        assert!(!manifest.services[1].present);
        assert!(!burrow.join("snapshots").join(".burrow-tmp").exists());

        std::fs::write(db.join("PG_VERSION"), "broken\n").expect("modify");
        std::fs::create_dir_all(burrow.join("data").join("api")).expect("create api data");

        let report = restore_snapshot(&burrow, "before").expect("restore");
        assert_eq!(report.services, vec!["db".to_owned(), "api".to_owned()]);
        assert_eq!(
            std::fs::read_to_string(db.join("PG_VERSION")).expect("read"),
            "16\n"
        );
        assert!(!burrow.join("data").join("api").exists());
        assert_eq!(list_snapshots(&burrow).expect("list").len(), 1);
    }

    #[test]
    fn duplicate_snapshots_and_unknown_restores_are_errors() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        create_snapshot(&burrow, "nightly", &[]).expect("snapshot");
        let error = create_snapshot(&burrow, "nightly", &[]).expect_err("duplicate");
        assert!(error.to_string().contains("already exists"));
        let error = restore_snapshot(&burrow, "missing").expect_err("missing");
        assert!(error.to_string().contains("does not exist"));
        assert!(snapshot_dir(&burrow, "nightly")
            .join("manifest.json")
            .is_file());
    }

    #[test]
    fn temp_directories_are_not_listed() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let temp = burrow.join("snapshots").join(".burrow-tmp-abandoned");
        std::fs::create_dir_all(&temp).expect("create temp");
        assert!(list_snapshots(&burrow).expect("list").is_empty());
    }
}
