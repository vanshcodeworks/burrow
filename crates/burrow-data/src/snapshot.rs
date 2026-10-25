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
//! the old one or the restored one, never a partially copied mix. A journal
//! written before the first swap lets the next `burrow up` (or `restore`)
//! roll an interrupted restore back to the pre-restore directories. If every
//! swap completed, the journal is marked committed first, so recovery finishes
//! the cleanup instead of undoing the restore.
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
const RESTORE_JOURNAL_FILE: &str = ".burrow-restore-journal.json";
const RESTORE_JOURNAL_VERSION: u32 = 1;

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

/// Result of recovering an interrupted restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreRecovery {
    /// True when every swap had completed and recovery only finished cleanup.
    /// False when the restore was rolled back to the pre-restore directories.
    pub committed: bool,
    /// Services named by the interrupted restore journal.
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestoreJournal {
    version: u32,
    nonce: String,
    snapshot: String,
    committed: bool,
    services: Vec<RestoreJournalService>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestoreJournalService {
    name: String,
    present: bool,
    had_original: bool,
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
        if is_reserved_windows_name(name) {
            return Err(Error::ReservedName {
                kind: "snapshot name",
                name: name.to_owned(),
            });
        }
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
        if is_reserved_windows_name(name) {
            return Err(Error::ReservedName {
                kind: "service name",
                name: name.to_owned(),
            });
        }
        Ok(())
    } else {
        Err(Error::InvalidServiceName(name.to_owned()))
    }
}

/// Windows reserves these stems as device names, with or without an extension.
/// A directory such as `.burrow/data/con` or a snapshot named `nul` would not
/// be an ordinary file, so reject them before any path is built.
fn is_reserved_windows_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
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
///
/// A journal is written before the first live directory is touched. If the
/// process dies mid-restore, [`recover_interrupted_restore`] rolls the swapped
/// services back. Once every swap succeeds the journal is marked committed, so
/// recovery finishes cleanup instead of undoing a completed restore.
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

    // A previous restore may have been interrupted. Resolve it before this
    // restore starts so its backups cannot be mistaken for current data.
    recover_interrupted_restore(burrow_dir)?;

    let data_root = burrow_dir.join(DATA_DIR);
    fs::create_dir_all(&data_root).map_err(|error| Error::io(&data_root, error))?;
    let nonce = format!("{}-{}", std::process::id(), super::unix_now());
    let staging = data_root.join(format!("{RESTORE_PREFIX}{nonce}"));
    fs::create_dir_all(&staging).map_err(|error| Error::io(&staging, error))?;

    // Stage every present service before touching live data.
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
    }

    let journal_services = manifest
        .services
        .iter()
        .map(|service| RestoreJournalService {
            name: service.name.clone(),
            present: service.present,
            had_original: data_root.join(&service.name).is_dir(),
        })
        .collect::<Vec<_>>();
    let mut journal = RestoreJournal {
        version: RESTORE_JOURNAL_VERSION,
        nonce: nonce.clone(),
        snapshot: name.to_owned(),
        committed: false,
        services: journal_services,
    };
    write_journal(&data_root, &journal)?;

    let mut completed = Vec::new();
    for service in &manifest.services {
        let target = data_root.join(&service.name);
        let backup = data_root.join(format!("{BACKUP_PREFIX}{nonce}-{}", service.name));
        let entry = journal
            .services
            .iter()
            .find(|entry| entry.name == service.name)
            .expect("journal covers every manifest service");
        let had_original = entry.had_original;
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

    // Every live directory is in its restored state. Mark the journal
    // committed before deleting the old copies: a crash after this point must
    // keep the restored data, not roll it back.
    journal.committed = true;
    write_journal(&data_root, &journal)?;

    for entry in &completed {
        let _ = fs::remove_dir_all(&entry.backup);
    }
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(journal_path(&data_root));

    Ok(RestoreReport {
        name: name.to_owned(),
        services: manifest
            .services
            .iter()
            .map(|service| service.name.clone())
            .collect(),
    })
}

/// Recovers a restore that a crash interrupted.
///
/// Returns `None` when there is nothing to recover. When a journal exists and
/// is not committed, every service that was already swapped is rolled back to
/// its backup, restoring the pre-restore directories. When the journal is
/// committed, the restored directories stay and only backup/staging cleanup is
/// finished. Staging directories without a journal are copies from before the
/// first swap and are removed. Backups without a journal are refused rather
/// than guessed about.
pub fn recover_interrupted_restore(burrow_dir: &Path) -> Result<Option<RestoreRecovery>> {
    let data_root = burrow_dir.join(DATA_DIR);
    let journal_path = journal_path(&data_root);
    let journal = match fs::read_to_string(&journal_path) {
        Ok(text) => {
            let journal: RestoreJournal =
                serde_json::from_str(&text).map_err(|error| Error::RestoreJournal {
                    path: journal_path.clone(),
                    message: error.to_string(),
                })?;
            if journal.version != RESTORE_JOURNAL_VERSION {
                return Err(Error::RestoreJournal {
                    path: journal_path,
                    message: format!(
                        "unsupported restore journal version {} (expected {RESTORE_JOURNAL_VERSION})",
                        journal.version
                    ),
                });
            }
            Some(journal)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(Error::io(&journal_path, error)),
    };

    let Some(journal) = journal else {
        let backups = entries_with_prefix(&data_root, BACKUP_PREFIX)?;
        if !backups.is_empty() {
            return Err(Error::RestoreJournal {
                path: journal_path,
                message: format!(
                    "found restore backups without a journal ({}); inspect them before starting",
                    backups
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        remove_entries_with_prefix(&data_root, RESTORE_PREFIX)?;
        let _ = fs::remove_file(journal_path.with_extension("tmp"));
        return Ok(None);
    };

    let services = journal
        .services
        .iter()
        .map(|service| service.name.clone())
        .collect::<Vec<_>>();

    if journal.committed {
        for service in &journal.services {
            remove_dir_if_exists(
                &data_root.join(format!("{BACKUP_PREFIX}{}-{}", journal.nonce, service.name)),
            )?;
        }
    } else {
        // Reverse order mirrors the swap order. A service with a backup is
        // restored to its original directory; a service that had no original
        // loses the restored copy.
        for service in journal.services.iter().rev() {
            let target = data_root.join(&service.name);
            let backup =
                data_root.join(format!("{BACKUP_PREFIX}{}-{}", journal.nonce, service.name));
            if service.had_original {
                if backup.is_dir() {
                    remove_dir_if_exists(&target)?;
                    fs::rename(&backup, &target).map_err(|error| Error::io(&target, error))?;
                }
            } else {
                remove_dir_if_exists(&target)?;
            }
        }
    }

    remove_dir_if_exists(&data_root.join(format!("{RESTORE_PREFIX}{}", journal.nonce)))?;
    fs::remove_file(&journal_path).map_err(|error| Error::io(&journal_path, error))?;
    let _ = fs::remove_file(journal_path.with_extension("tmp"));

    Ok(Some(RestoreRecovery {
        committed: journal.committed,
        services,
    }))
}

fn journal_path(data_root: &Path) -> PathBuf {
    data_root.join(RESTORE_JOURNAL_FILE)
}

fn write_journal(data_root: &Path, journal: &RestoreJournal) -> Result<()> {
    let path = journal_path(data_root);
    let temporary = path.with_extension("tmp");
    let encoded = serde_json::to_vec_pretty(journal).map_err(|error| Error::RestoreJournal {
        path: path.clone(),
        message: error.to_string(),
    })?;
    fs::write(&temporary, encoded).map_err(|error| Error::io(&temporary, error))?;
    fs::rename(&temporary, &path).map_err(|error| Error::io(&path, error))?;
    Ok(())
}

fn entries_with_prefix(directory: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(directory, error)),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory, error))?;
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            matches.push(entry.path());
        }
    }
    Ok(matches)
}

fn remove_entries_with_prefix(directory: &Path, prefix: &str) -> Result<()> {
    for path in entries_with_prefix(directory, prefix)? {
        remove_dir_if_exists(&path)?;
    }
    Ok(())
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path, error)),
    }
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
    if let Some(stream) = first_alternate_stream(source)? {
        return Err(Error::AlternateDataStream {
            path: source.to_owned(),
            stream,
        });
    }
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
            if let Some(stream) = first_alternate_stream(&path)? {
                return Err(Error::AlternateDataStream { path, stream });
            }
            fs::copy(&path, &destination_path).map_err(|error| Error::io(&path, error))?;
            files += 1;
            bytes += metadata.len();
        }
    }
    Ok((files, bytes))
}

/// Returns the first non-default NTFS stream on `path`, if any.
///
/// `fs::copy` copies the default stream only. Silently dropping an alternate
/// stream would make a snapshot incomplete, so callers refuse instead.
#[cfg(windows)]
fn first_alternate_stream(path: &Path) -> Result<Option<String>> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Foundation::ERROR_HANDLE_EOF;
    use windows::Win32::Storage::FileSystem::{
        FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
        WIN32_FIND_STREAM_DATA,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data = WIN32_FIND_STREAM_DATA::default();
    let handle = match unsafe {
        FindFirstStreamW(
            PCWSTR(wide.as_ptr()),
            FindStreamInfoStandard,
            &mut data as *mut _ as *mut core::ffi::c_void,
            None,
        )
    } {
        Ok(handle) => handle,
        Err(error) if stream_scan_unavailable(error.code()) => return Ok(None),
        Err(error) => return Err(Error::io(path, std::io::Error::other(error.message()))),
    };

    let mut found = None;
    loop {
        let name = stream_name(&data);
        if !name.eq_ignore_ascii_case("::$DATA") {
            found = Some(name.strip_suffix(":$DATA").unwrap_or(&name).to_owned());
            break;
        }
        match unsafe { FindNextStreamW(handle, &mut data as *mut _ as *mut core::ffi::c_void) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0) => break,
            Err(error) => {
                unsafe {
                    let _ = FindClose(handle);
                }
                return Err(Error::io(path, std::io::Error::other(error.message())));
            }
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    Ok(found)
}

#[cfg(windows)]
fn stream_scan_unavailable(code: windows::core::HRESULT) -> bool {
    use windows::Win32::Foundation::{
        ERROR_HANDLE_EOF, ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED,
    };

    // FAT and other filesystems do not support streams; an empty listing and
    // "not supported" both mean there is nothing that `fs::copy` would drop.
    code == windows::core::HRESULT::from_win32(ERROR_HANDLE_EOF.0)
        || code == windows::core::HRESULT::from_win32(ERROR_INVALID_FUNCTION.0)
        || code == windows::core::HRESULT::from_win32(ERROR_NOT_SUPPORTED.0)
}

#[cfg(windows)]
fn stream_name(data: &windows::Win32::Storage::FileSystem::WIN32_FIND_STREAM_DATA) -> String {
    let length = data
        .cStreamName
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(data.cStreamName.len());
    String::from_utf16_lossy(&data.cStreamName[..length])
}

#[cfg(not(windows))]
fn first_alternate_stream(_path: &Path) -> Result<Option<String>> {
    Ok(None)
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
    use super::{
        create_snapshot, list_snapshots, recover_interrupted_restore, restore_snapshot,
        snapshot_dir, validate_name, RestoreJournal, RestoreJournalService, BACKUP_PREFIX,
        RESTORE_JOURNAL_FILE, RESTORE_JOURNAL_VERSION, RESTORE_PREFIX,
    };

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
    fn reserved_windows_device_names_are_rejected() {
        for name in ["con", "nul", "com1", "lpt9", "aux", "prn", "con.txt"] {
            let error = validate_name(name).expect_err("reserved snapshot name");
            assert!(
                error.to_string().contains("reserved Windows device name"),
                "`{name}` produced: {error}"
            );
        }
        for name in ["con", "NUL", "com1"] {
            let error = super::validate_service_name(name).expect_err("reserved service name");
            assert!(
                error.to_string().contains("reserved Windows device name"),
                "`{name}` produced: {error}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn hardlinked_files_are_copied_as_independent_files() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let db = burrow.join("data").join("db");
        std::fs::create_dir_all(&db).expect("create data");
        let original = db.join("original.txt");
        std::fs::write(&original, "shared\n").expect("write original");
        let link = db.join("link.txt");
        std::fs::hard_link(&original, &link).expect("create hard link");

        let manifest = create_snapshot(&burrow, "linked", &[service("db")]).expect("snapshot");
        assert_eq!(manifest.services[0].files, 2);
        let copied = snapshot_dir(&burrow, "linked").join("data").join("db");
        assert_eq!(
            std::fs::read_to_string(copied.join("original.txt")).expect("read copy"),
            "shared\n"
        );
        assert_eq!(
            std::fs::read_to_string(copied.join("link.txt")).expect("read link copy"),
            "shared\n"
        );

        // The snapshot is a plain copy: editing it must not change live data.
        std::fs::write(copied.join("link.txt"), "snapshot only\n").expect("edit snapshot copy");
        assert_eq!(
            std::fs::read_to_string(&original).expect("read original"),
            "shared\n"
        );
        assert_eq!(
            std::fs::read_to_string(&link).expect("read link"),
            "shared\n"
        );

        // And editing the live link must not change the snapshot.
        std::fs::write(&link, "live only\n").expect("edit live link");
        assert_eq!(
            std::fs::read_to_string(copied.join("link.txt")).expect("read link copy"),
            "snapshot only\n"
        );
    }

    #[cfg(windows)]
    #[test]
    fn alternate_data_streams_are_refused_instead_of_dropped() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let db = burrow.join("data").join("db");
        std::fs::create_dir_all(&db).expect("create data");
        let record = db.join("record.txt");
        std::fs::write(&record, "visible\n").expect("write record");
        let stream = db.join("record.txt:metadata");
        std::fs::write(&stream, "hidden\n").expect("write alternate stream");
        assert_eq!(
            std::fs::read_to_string(&stream).expect("read alternate stream"),
            "hidden\n"
        );

        let error = create_snapshot(&burrow, "streamed", &[service("db")])
            .expect_err("alternate stream must be refused");
        let message = error.to_string();
        assert!(
            message.contains("alternate data stream `:metadata`"),
            "{message}"
        );
        assert!(message.contains("record.txt"), "{message}");
        assert!(
            !snapshot_dir(&burrow, "streamed").exists(),
            "a refused snapshot must leave nothing behind"
        );
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

    fn write_test_journal(burrow: &std::path::Path, nonce: &str, committed: bool) {
        let journal = RestoreJournal {
            version: RESTORE_JOURNAL_VERSION,
            nonce: nonce.to_owned(),
            snapshot: "before".to_owned(),
            committed,
            services: vec![RestoreJournalService {
                name: "db".to_owned(),
                present: true,
                had_original: true,
            }],
        };
        std::fs::write(
            burrow.join("data").join(RESTORE_JOURNAL_FILE),
            serde_json::to_vec_pretty(&journal).expect("journal JSON"),
        )
        .expect("write journal");
    }

    #[test]
    fn an_interrupted_restore_is_rolled_back_to_the_original_directory() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let data = burrow.join("data");
        let live = data.join("db");
        std::fs::create_dir_all(&live).expect("create live data");
        std::fs::write(live.join("state.txt"), "current\n").expect("write current");

        // Simulate the exact crash point after `target -> backup` and
        // `staged -> target`: the snapshot copy is live, the original is in
        // the backup, the staging directory still exists, and the journal is
        // not committed.
        let nonce = "test-rollback";
        let staging = data.join(format!("{RESTORE_PREFIX}{nonce}"));
        std::fs::create_dir_all(staging.join("db")).expect("create staging");
        std::fs::write(staging.join("db").join("state.txt"), "snapshot\n").expect("write staged");
        let backup = data.join(format!("{BACKUP_PREFIX}{nonce}-db"));
        std::fs::rename(&live, &backup).expect("move original to backup");
        std::fs::rename(staging.join("db"), &live).expect("install staged copy");
        write_test_journal(&burrow, nonce, false);

        let recovery = recover_interrupted_restore(&burrow)
            .expect("recover")
            .expect("recovery happened");
        assert!(!recovery.committed);
        assert_eq!(recovery.services, vec!["db".to_owned()]);
        assert_eq!(
            std::fs::read_to_string(live.join("state.txt")).expect("read live"),
            "current\n",
            "the pre-restore directory must be restored"
        );
        assert!(!backup.exists(), "backup must be consumed");
        assert!(!staging.exists(), "staging must be removed");
        assert!(
            !data.join(RESTORE_JOURNAL_FILE).exists(),
            "journal must be removed"
        );
    }

    #[test]
    fn a_committed_restore_is_kept_and_only_cleaned_up() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let data = burrow.join("data");
        let live = data.join("db");
        std::fs::create_dir_all(&live).expect("create live data");
        std::fs::write(live.join("state.txt"), "restored\n").expect("write restored");

        let nonce = "test-committed";
        let backup = data.join(format!("{BACKUP_PREFIX}{nonce}-db"));
        std::fs::create_dir_all(&backup).expect("create backup");
        std::fs::write(backup.join("state.txt"), "old\n").expect("write old");
        let staging = data.join(format!("{RESTORE_PREFIX}{nonce}"));
        std::fs::create_dir_all(&staging).expect("create staging");
        write_test_journal(&burrow, nonce, true);

        let recovery = recover_interrupted_restore(&burrow)
            .expect("recover")
            .expect("recovery happened");
        assert!(recovery.committed);
        assert_eq!(
            std::fs::read_to_string(live.join("state.txt")).expect("read live"),
            "restored\n",
            "a committed restore must not be rolled back"
        );
        assert!(!backup.exists(), "backup must be cleaned up");
        assert!(!staging.exists(), "staging must be cleaned up");
        assert!(!data.join(RESTORE_JOURNAL_FILE).exists());
    }

    #[test]
    fn staging_without_a_journal_is_removed_but_backups_are_refused() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let data = burrow.join("data");
        let staging = data.join(format!("{RESTORE_PREFIX}abandoned"));
        std::fs::create_dir_all(staging.join("db")).expect("create staging");
        std::fs::write(staging.join("db").join("state.txt"), "copy\n").expect("write copy");

        assert!(
            recover_interrupted_restore(&burrow)
                .expect("recover")
                .is_none(),
            "staging alone is not a journaled restore"
        );
        assert!(!staging.exists(), "staging copy must be removed");

        let backup = data.join(format!("{BACKUP_PREFIX}orphan-db"));
        std::fs::create_dir_all(&backup).expect("create orphan backup");
        let error = recover_interrupted_restore(&burrow).expect_err("backup without journal");
        assert!(
            error.to_string().contains("without a journal"),
            "error was: {error}"
        );
        assert!(
            backup.exists(),
            "an unexplained backup must be preserved for inspection"
        );
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

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_a_data_directory_is_rejected() {
        let project = tempfile::tempdir().expect("temp");
        let burrow = project.path().join(".burrow");
        let db = burrow.join("data").join("db");
        std::fs::create_dir_all(&db).expect("create data");
        std::fs::write(db.join("PG_VERSION"), "16\n").expect("write version");

        let target = project.path().join("outside");
        std::fs::create_dir_all(&target).expect("create junction target");
        std::fs::write(target.join("secret.txt"), "outside").expect("write secret");
        let junction = db.join("escape");
        let link = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .expect("run mklink");
        if !link.status.success() || !junction.exists() {
            let stderr = String::from_utf8_lossy(&link.stderr).to_lowercase();
            let environmental = stderr.contains("denied") || stderr.contains("privilege");
            let prefix = if environmental { "SKIPPED-ENV: " } else { "" };
            panic!(
                "{prefix}mklink did not create junction `{}`: status {:?}, stderr: {stderr}",
                junction.display(),
                link.status
            );
        }

        let error = create_snapshot(&burrow, "suspicious", &[service("db")]).expect_err("junction");
        let message = error.to_string();
        assert!(message.contains("refusing to snapshot reparse point"));
        assert!(message.contains(&junction.display().to_string()));

        std::fs::remove_dir(&junction).expect("remove junction");
        assert!(!snapshot_dir(&burrow, "suspicious").exists());
        let leftovers = std::fs::read_dir(burrow.join("snapshots"))
            .expect("snapshots dir")
            .count();
        assert_eq!(leftovers, 0);
    }
}
