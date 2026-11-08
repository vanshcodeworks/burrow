//! Shared, content-addressed package cache.
//!
//! Layout under `%LOCALAPPDATA%\burrow\cache` (or `BURROW_CACHE_DIR`):
//!
//! ```text
//! archives/<sha256>.<ext>          verified download, re-downloadable
//! archives/<sha256>.<ext>.part     partial download, resumed with a Range request
//! packages/<sha256>/               extracted archive (shared by every project)
//! packages/<sha256>/.burrow-package.json
//! refs/<hash-of-project-dir>.json  which packages each project uses
//! locks/<sha256>.lock              cross-process download lock
//! tmp/                             half-extracted archives, deleted on failure
//! ```
//!
//! Nothing is used before its SHA-256 matches the registry, and extracted
//! trees are renamed into place atomically.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::error::{Error, Result};
use crate::registry::{Platform, Resolved};

/// Marker written into every extracted package.
pub const MARKER_FILE: &str = ".burrow-package.json";
const MARKER_VERSION: u32 = 1;
const REF_VERSION: u32 = 1;
const LOCK_STALE_AFTER: Duration = Duration::from_secs(60 * 60);
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(60 * 30);

/// Download progress reported to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Bytes already present, including a resumed prefix.
    pub downloaded: u64,
    /// Expected total from the registry.
    pub total: u64,
}

/// Metadata stored next to an extracted package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    /// Marker schema.
    pub version: u32,
    /// Registry package name.
    pub name: String,
    /// Registry version key.
    pub package_version: String,
    /// Archive SHA-256, also the directory name.
    pub sha256: String,
    /// Archive size in bytes.
    pub size: u64,
    /// Unix timestamp of extraction.
    pub installed_at: u64,
}

/// One package reference held by a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRef {
    /// Canonical `name@version`.
    pub key: String,
    /// Archive SHA-256.
    pub sha256: String,
}

/// Result of `burrow pkg gc`.
#[derive(Debug, Default)]
pub struct GcReport {
    /// Package directories removed.
    pub packages_removed: usize,
    /// Archive files removed.
    pub archives_removed: usize,
    /// Bytes freed.
    pub bytes_freed: u64,
    /// Reference files whose project directory no longer exists.
    pub stale_refs_removed: usize,
}

/// Result of `burrow pkg verify`.
#[derive(Debug, Default)]
pub struct VerifyReport {
    /// Archives whose hash was checked.
    pub archives_checked: usize,
    /// Extracted packages whose marker was checked.
    pub packages_checked: usize,
    /// Human-readable problems; empty means the cache is healthy.
    pub problems: Vec<String>,
}

/// The shared cache root.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Uses `BURROW_CACHE_DIR`, then `%LOCALAPPDATA%\burrow\cache`.
    pub fn discover() -> Result<Self> {
        if let Some(root) = std::env::var_os("BURROW_CACHE_DIR") {
            if root.is_empty() {
                return Err(Error::NoCacheDirectory);
            }
            return Ok(Self::at(PathBuf::from(root)));
        }
        let local = std::env::var_os("LOCALAPPDATA").ok_or(Error::NoCacheDirectory)?;
        if local.is_empty() {
            return Err(Error::NoCacheDirectory);
        }
        Ok(Self::at(PathBuf::from(local).join("burrow").join("cache")))
    }

    /// Uses an explicit root. Useful for tests and `--cache-dir`.
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    /// Cache root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn archives(&self) -> PathBuf {
        self.root.join("archives")
    }

    fn packages(&self) -> PathBuf {
        self.root.join("packages")
    }

    fn refs(&self) -> PathBuf {
        self.root.join("refs")
    }

    fn locks(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Creates the cache layout.
    pub fn prepare(&self) -> Result<()> {
        for directory in [
            self.archives(),
            self.packages(),
            self.refs(),
            self.locks(),
            self.tmp(),
        ] {
            std::fs::create_dir_all(&directory).map_err(|error| Error::io(&directory, error))?;
        }
        Ok(())
    }

    /// Archive path for `platform`, keyed by its verified hash.
    pub fn archive_path(&self, platform: &Platform) -> PathBuf {
        self.archives().join(format!(
            "{}.{}",
            platform.sha256,
            platform.archive.extension()
        ))
    }

    /// Extracted package directory, keyed by the archive hash.
    pub fn package_dir(&self, sha256: &str) -> PathBuf {
        self.packages().join(sha256)
    }

    /// Reads and validates a package marker.
    pub fn marker(&self, sha256: &str) -> Option<Marker> {
        let path = self.package_dir(sha256).join(MARKER_FILE);
        let text = std::fs::read_to_string(path).ok()?;
        let marker: Marker = serde_json::from_str(&text).ok()?;
        if marker.version == MARKER_VERSION && marker.sha256 == sha256 {
            Some(marker)
        } else {
            None
        }
    }

    /// True when the package is extracted and its marker matches.
    pub fn is_installed(&self, resolved: &Resolved<'_>) -> bool {
        self.marker(&resolved.platform.sha256).is_some()
    }

    /// Ensures the package is downloaded, verified, and extracted.
    ///
    /// Returns the directory containing the extracted archive. When the cache
    /// already holds this exact archive the network is not touched, which
    /// makes `burrow up` work offline.
    pub async fn ensure(
        &self,
        resolved: &Resolved<'_>,
        progress: &mut dyn FnMut(DownloadProgress),
    ) -> Result<PathBuf> {
        self.prepare()?;
        let sha256 = resolved.platform.sha256.clone();
        let package_dir = self.package_dir(&sha256);
        if self.marker(&sha256).is_some() {
            return Ok(package_dir);
        }

        let _lock = self.acquire_lock(&sha256).await?;
        if self.marker(&sha256).is_some() {
            return Ok(package_dir);
        }

        let archive = self.ensure_archive(resolved, progress).await?;
        let temporary = self
            .tmp()
            .join(format!("{sha256}-{}-{}", std::process::id(), now_nanos()));
        let extraction_root = temporary.clone();
        let archive_for_task = archive.clone();
        let kind = resolved.platform.archive;
        tokio::task::spawn_blocking(move || {
            crate::extract::extract(&archive_for_task, kind, &extraction_root)
        })
        .await
        .map_err(|error| Error::Archive {
            message: format!("extraction task failed: {error}"),
        })??;

        let marker = Marker {
            version: MARKER_VERSION,
            name: resolved.entry.name.clone(),
            package_version: resolved.version.version.clone(),
            sha256: sha256.clone(),
            size: resolved.platform.size,
            installed_at: unix_now(),
        };
        let marker_path = temporary.join(MARKER_FILE);
        std::fs::write(
            &marker_path,
            serde_json::to_vec_pretty(&marker).map_err(|error| Error::CorruptCache {
                path: marker_path.clone(),
                message: error.to_string(),
            })?,
        )
        .map_err(|error| Error::io(&marker_path, error))?;

        std::fs::create_dir_all(self.packages())
            .map_err(|error| Error::io(self.packages(), error))?;
        match std::fs::rename(&temporary, &package_dir) {
            Ok(()) => {}
            Err(error) => {
                // Another Burrow process may have won the race after our lock
                // check; accept its identical package and discard our copy.
                if self.marker(&sha256).is_some() {
                    let _ = std::fs::remove_dir_all(&temporary);
                } else {
                    let _ = std::fs::remove_dir_all(&temporary);
                    return Err(Error::io(&package_dir, error));
                }
            }
        }

        info!(
            package = %resolved.entry.name,
            version = %resolved.version.version,
            "installed package into the shared cache"
        );
        Ok(package_dir)
    }

    async fn ensure_archive(
        &self,
        resolved: &Resolved<'_>,
        progress: &mut dyn FnMut(DownloadProgress),
    ) -> Result<PathBuf> {
        let platform = resolved.platform;
        let final_path = self.archive_path(platform);
        let expected = platform.size;

        if final_path.exists() {
            let actual_size = std::fs::metadata(&final_path)
                .map_err(|error| Error::io(&final_path, error))?
                .len();
            if actual_size == expected {
                let path = final_path.clone();
                let hash = tokio::task::spawn_blocking(move || sha256_file(&path))
                    .await
                    .map_err(|error| Error::Archive {
                        message: format!("hash task failed: {error}"),
                    })??;
                if hash == platform.sha256 {
                    return Ok(final_path);
                }
                warn!(
                    url = %platform.url,
                    expected = %platform.sha256,
                    actual = %hash,
                    "cached archive failed verification; downloading again"
                );
            }
            std::fs::remove_file(&final_path).map_err(|error| Error::io(&final_path, error))?;
        }

        let part = PathBuf::from(format!("{}.part", final_path.display()));
        let mut offset = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
        if offset > expected {
            std::fs::remove_file(&part).map_err(|error| Error::io(&part, error))?;
            offset = 0;
        }

        progress(DownloadProgress {
            downloaded: offset,
            total: expected,
        });

        if offset < expected {
            self.download(&part, offset, resolved, progress).await?;
        }

        let actual_size = std::fs::metadata(&part)
            .map_err(|error| Error::io(&part, error))?
            .len();
        if actual_size != expected {
            return Err(Error::IncompleteDownload {
                url: platform.url.clone(),
                expected,
                actual: actual_size,
            });
        }

        let part_for_task = part.clone();
        let hash = tokio::task::spawn_blocking(move || sha256_file(&part_for_task))
            .await
            .map_err(|error| Error::Archive {
                message: format!("hash task failed: {error}"),
            })??;
        if hash != platform.sha256 {
            std::fs::remove_file(&part).ok();
            return Err(Error::HashMismatch {
                url: platform.url.clone(),
                expected: platform.sha256.clone(),
                actual: hash,
            });
        }

        if final_path.exists() {
            std::fs::remove_file(&final_path).map_err(|error| Error::io(&final_path, error))?;
        }
        std::fs::rename(&part, &final_path).map_err(|error| Error::io(&final_path, error))?;
        Ok(final_path)
    }

    async fn download(
        &self,
        part: &Path,
        offset: u64,
        resolved: &Resolved<'_>,
        progress: &mut dyn FnMut(DownloadProgress),
    ) -> Result<()> {
        let platform = resolved.platform;
        let client = reqwest::Client::builder()
            .user_agent(concat!("burrow/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|source| Error::Download {
                url: platform.url.clone(),
                source,
            })?;

        let mut request = client.get(&platform.url);
        if offset > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = request.send().await.map_err(|source| Error::Download {
            url: platform.url.clone(),
            source,
        })?;
        let status = response.status();
        let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT && offset > 0;
        if !resuming && !status.is_success() {
            return Err(Error::HttpStatus {
                url: platform.url.clone(),
                status: status.as_u16(),
            });
        }

        let mut downloaded = if resuming { offset } else { 0 };
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(!resuming)
            .append(resuming)
            .write(true)
            .open(part)
            .await
            .map_err(|error| Error::io(part, error))?;

        progress(DownloadProgress {
            downloaded,
            total: platform.size,
        });

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|source| Error::Download {
                url: platform.url.clone(),
                source,
            })?;
            file.write_all(&chunk)
                .await
                .map_err(|error| Error::io(part, error))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > platform.size {
                drop(file);
                std::fs::remove_file(part).ok();
                return Err(Error::IncompleteDownload {
                    url: platform.url.clone(),
                    expected: platform.size,
                    actual: downloaded,
                });
            }
            progress(DownloadProgress {
                downloaded,
                total: platform.size,
            });
        }
        file.flush().await.map_err(|error| Error::io(part, error))?;
        drop(file);

        if downloaded < platform.size {
            // Keep the partial file so the next attempt resumes it.
            return Err(Error::IncompleteDownload {
                url: platform.url.clone(),
                expected: platform.size,
                actual: downloaded,
            });
        }
        Ok(())
    }

    fn acquire_lock(
        &self,
        sha256: &str,
    ) -> impl std::future::Future<Output = Result<LockGuard>> + '_ {
        let path = self.locks().join(format!("{sha256}.lock"));
        async move {
            std::fs::create_dir_all(self.locks())
                .map_err(|error| Error::io(self.locks(), error))?;
            let deadline = Instant::now() + LOCK_WAIT_LIMIT;
            loop {
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(mut file) => {
                        let _ = writeln!(file, "{}", std::process::id());
                        return Ok(LockGuard { path });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let stale = std::fs::metadata(&path)
                            .and_then(|meta| meta.modified())
                            .ok()
                            .and_then(|modified| modified.elapsed().ok())
                            .is_some_and(|age| age > LOCK_STALE_AFTER);
                        if stale {
                            std::fs::remove_file(&path).ok();
                            continue;
                        }
                        if Instant::now() >= deadline {
                            return Err(Error::LockTimeout { path });
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(error) => return Err(Error::io(&path, error)),
                }
            }
        }
    }

    /// Records which content-addressed packages one project uses.
    ///
    /// `burrow pkg gc` keeps every package listed by a reference file whose
    /// project directory still exists, so a stopped project does not lose its
    /// cache when it is started again offline.
    pub fn record_refs(
        &self,
        project_dir: &Path,
        project_name: &str,
        packages: &[PackageRef],
    ) -> Result<()> {
        self.prepare()?;
        let key = sha256_bytes(normalize_path(project_dir).as_bytes());
        let path = self.refs().join(format!("{key}.json"));
        let document = ProjectRefs {
            version: REF_VERSION,
            project: project_name.to_owned(),
            dir: project_dir.to_path_buf(),
            packages: packages.to_vec(),
        };
        let encoded =
            serde_json::to_vec_pretty(&document).map_err(|error| Error::CorruptCache {
                path: path.clone(),
                message: error.to_string(),
            })?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, encoded).map_err(|error| Error::io(&temporary, error))?;
        std::fs::rename(&temporary, &path).map_err(|error| Error::io(&path, error))?;
        Ok(())
    }

    /// Removes packages and archives not referenced by any live project.
    pub fn gc(&self) -> Result<GcReport> {
        self.prepare()?;
        let mut report = GcReport::default();
        let mut live = BTreeSet::new();

        let refs = std::fs::read_dir(self.refs()).map_err(|error| Error::io(self.refs(), error))?;
        for entry in refs {
            let entry = entry.map_err(|error| Error::io(self.refs(), error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let document = match std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<ProjectRefs>(&text).ok())
            {
                Some(document) if document.version == REF_VERSION => document,
                _ => {
                    warn!(path = %path.display(), "ignoring an unreadable cache reference");
                    continue;
                }
            };
            if !document.dir.is_dir() {
                std::fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
                report.stale_refs_removed += 1;
                continue;
            }
            for package in document.packages {
                live.insert(package.sha256);
            }
        }

        let packages = std::fs::read_dir(self.packages())
            .map_err(|error| Error::io(self.packages(), error))?;
        for entry in packages {
            let entry = entry.map_err(|error| Error::io(self.packages(), error))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !live.contains(&name) {
                let bytes = directory_size(&path);
                std::fs::remove_dir_all(&path).map_err(|error| Error::io(&path, error))?;
                report.packages_removed += 1;
                report.bytes_freed += bytes;
            }
        }

        let archives = std::fs::read_dir(self.archives())
            .map_err(|error| Error::io(self.archives(), error))?;
        for entry in archives {
            let entry = entry.map_err(|error| Error::io(self.archives(), error))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let hash = name.split('.').next().unwrap_or_default();
            if live.contains(hash) {
                continue;
            }
            let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            std::fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
            report.archives_removed += 1;
            report.bytes_freed += bytes;
        }

        let temporary =
            std::fs::read_dir(self.tmp()).map_err(|error| Error::io(self.tmp(), error))?;
        for entry in temporary {
            let entry = entry.map_err(|error| Error::io(self.tmp(), error))?;
            let path = entry.path();
            if path.is_dir() {
                report.bytes_freed += directory_size(&path);
                std::fs::remove_dir_all(&path).map_err(|error| Error::io(&path, error))?;
            } else {
                report.bytes_freed += std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                std::fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
            }
        }

        for lock in list_files(&self.locks())? {
            let stale = std::fs::metadata(&lock)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > LOCK_STALE_AFTER);
            if stale {
                std::fs::remove_file(&lock).ok();
            }
        }

        Ok(report)
    }

    /// Re-hashes every cached archive and validates every package marker.
    pub fn verify(&self) -> Result<VerifyReport> {
        self.prepare()?;
        let mut report = VerifyReport::default();

        for archive in list_files(&self.archives())? {
            let name = archive
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let expected = name.split('.').next().unwrap_or_default().to_owned();
            if expected.len() != 64 {
                report
                    .problems
                    .push(format!("{} has no SHA-256 file name", archive.display()));
                continue;
            }
            match sha256_file(&archive) {
                Ok(actual) if actual == expected => report.archives_checked += 1,
                Ok(actual) => report.problems.push(format!(
                    "{} is corrupt: expected {expected}, got {actual}",
                    archive.display()
                )),
                Err(error) => report
                    .problems
                    .push(format!("{} could not be read: {error}", archive.display())),
            }
        }

        for package in list_directories(&self.packages())? {
            let sha256 = package
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            match self.marker(&sha256) {
                Some(marker) => {
                    if marker.size == 0 {
                        report
                            .problems
                            .push(format!("{} has an invalid marker", package.display()));
                    } else {
                        report.packages_checked += 1;
                    }
                }
                None => report
                    .problems
                    .push(format!("{} has no valid marker", package.display())),
            }
        }

        Ok(report)
    }

    /// Total bytes used by archives and extracted packages.
    pub fn size_on_disk(&self) -> u64 {
        directory_size(&self.root)
    }

    /// Installed packages, keyed by content hash.
    pub fn installed(&self) -> BTreeMap<String, Marker> {
        let mut installed = BTreeMap::new();
        if let Ok(directories) = list_directories(&self.packages()) {
            for directory in directories {
                let name = directory
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(marker) = self.marker(&name) {
                    installed.insert(name, marker);
                }
            }
        }
        installed
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectRefs {
    version: u32,
    project: String,
    dir: PathBuf,
    packages: Vec<PackageRef>,
}

struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Hashes a file with SHA-256.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| Error::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
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

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn list_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(directory).map_err(|error| Error::io(directory, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory, error))?;
        if entry.path().is_file() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn list_directories(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    let entries = std::fs::read_dir(directory).map_err(|error| Error::io(directory, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory, error))?;
        if entry.path().is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn directory_size(directory: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total += directory_size(&path);
        } else {
            total += metadata.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{sha256_file, Cache, PackageRef};
    use crate::registry::{ArchiveKind, Registry};

    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry
            .add_toml(
                r#"
schema = 2
name = "demo"
description = "Demo"
license = "MIT"
upstream = "https://example.com/demo"
source = "https://example.com/demo/downloads"

[versions."1.0"]
run = { program = "server", args = [] }
steps = []

[versions."1.0".platforms.windows-x86_64]
url = "https://example.com/demo.zip"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
size = 100
archive = "zip"

[versions."1.0".platforms.windows-x86_64.binaries]
server = "bin/server.exe"
"#,
                "demo",
            )
            .expect("parse demo");
        registry
    }

    #[test]
    fn hashes_files_and_normalizes_the_cache_root() {
        let directory = tempfile::tempdir().expect("temp");
        let file = directory.path().join("data");
        let mut output = std::fs::File::create(&file).expect("create");
        output.write_all(b"abc").expect("write");
        assert_eq!(
            sha256_file(&file).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let cache = Cache::at(directory.path().join("cache"));
        cache.prepare().expect("prepare");
        assert!(cache.root().join("packages").is_dir());
        assert!(cache.root().join("archives").is_dir());
        assert!(cache.root().join("refs").is_dir());
    }

    #[test]
    fn gc_keeps_referenced_content_and_removes_the_rest() {
        let directory = tempfile::tempdir().expect("temp");
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).expect("project");
        let cache = Cache::at(directory.path().join("cache"));
        cache.prepare().expect("prepare");

        let keep = "a".repeat(64);
        let drop = "b".repeat(64);
        for (hash, bytes) in [(&keep, b"keep".as_slice()), (&drop, b"drop".as_slice())] {
            let package = cache.package_dir(hash);
            std::fs::create_dir_all(&package).expect("package");
            std::fs::write(package.join("file"), bytes).expect("file");
            std::fs::write(
                package.join(super::MARKER_FILE),
                serde_json::json!({
                    "version": 1,
                    "name": "demo",
                    "package_version": "1.0",
                    "sha256": hash,
                    "size": bytes.len(),
                    "installed_at": 0,
                })
                .to_string(),
            )
            .expect("marker");
            std::fs::write(cache.archive_path(&archive_platform(hash)), bytes).expect("archive");
        }

        cache
            .record_refs(
                &project,
                "demo",
                &[PackageRef {
                    key: "demo@1.0".to_owned(),
                    sha256: keep.clone(),
                }],
            )
            .expect("record refs");

        let report = cache.gc().expect("gc");
        assert_eq!(report.packages_removed, 1);
        assert_eq!(report.archives_removed, 1);
        assert!(cache.package_dir(&keep).is_dir());
        assert!(!cache.package_dir(&drop).exists());
    }

    fn archive_platform(hash: &str) -> crate::registry::Platform {
        crate::registry::Platform {
            url: "https://example.com/demo.zip".to_owned(),
            sha256: hash.to_owned(),
            size: 4,
            archive: ArchiveKind::Zip,
            binaries: Default::default(),
        }
    }

    #[test]
    fn verify_reports_corrupt_archives_and_missing_markers() {
        let directory = tempfile::tempdir().expect("temp");
        let cache = Cache::at(directory.path().join("cache"));
        cache.prepare().expect("prepare");
        let hash = "c".repeat(64);
        std::fs::write(cache.archive_path(&archive_platform(&hash)), b"nope").expect("archive");
        let report = cache.verify().expect("verify");
        assert_eq!(report.archives_checked, 0);
        assert_eq!(report.problems.len(), 1);
        assert!(
            report.problems[0].contains("corrupt"),
            "{:?}",
            report.problems
        );
        assert!(registry().resolve("demo").is_ok());
    }

    /// Serves one fixed byte string with HTTP range support and records every
    /// request head. Used to prove the resume path without touching network.
    async fn range_server(
        content: &'static [u8],
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind range server");
        let port = listener.local_addr().expect("address").port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = std::sync::Arc::clone(&requests);
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    loop {
                        let read = stream.read(&mut chunk).await.expect("read request");
                        buffer.extend_from_slice(&chunk[..read]);
                        if buffer.windows(4).any(|window| window == b"\r\n\r\n") || read == 0 {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&buffer).into_owned();
                    let range = head
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                        .and_then(|line| line.split_once(':'))
                        .map(|(_, value)| value.trim().to_owned());

                    let (status, extra, body) = match &range {
                        Some(value) => {
                            let offset: u64 = value
                                .strip_prefix("bytes=")
                                .and_then(|rest| rest.split_once('-'))
                                .and_then(|(start, _)| start.parse().ok())
                                .unwrap_or(0);
                            let end = content.len() as u64 - 1;
                            (
                                "HTTP/1.1 206 Partial Content",
                                format!(
                                    "Content-Range: bytes {offset}-{end}/{}\r\n",
                                    content.len()
                                ),
                                &content[offset as usize..],
                            )
                        }
                        None => ("HTTP/1.1 200 OK", String::new(), content),
                    };
                    requests.lock().expect("requests lock").push(head);

                    let response = format!(
                        "{status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write head");
                    stream.write_all(body).await.expect("write body");
                });
            }
        });
        port
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_interrupted_download_resumes_from_the_part_file() {
        use crate::registry::{PackageVersion, Platform, Resolved};
        use std::collections::BTreeMap;
        use std::sync::{Arc, Mutex};

        use sha2::Digest;

        const PREFIX: usize = 16 * 1024;
        const TOTAL: usize = 64 * 1024;
        static CONTENT: &[u8] = &[0xAB_u8; TOTAL];
        let digest = sha2::Sha256::digest(CONTENT);
        let hash = hex::encode(digest);

        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let port = range_server(CONTENT, Arc::clone(&requests)).await;
        let url = format!("http://127.0.0.1:{port}/demo.bin");

        let platform = Platform {
            url: url.clone(),
            sha256: hash.clone(),
            size: TOTAL as u64,
            archive: ArchiveKind::Raw,
            binaries: BTreeMap::from([("bin".to_owned(), "bin".to_owned())]),
        };
        let version = PackageVersion {
            version: "1.0".to_owned(),
            platforms: BTreeMap::from([("windows-x86_64".to_owned(), platform)]),
            run: Some(crate::registry::TemplateCommand {
                program: "bin".to_owned(),
                args: Vec::new(),
            }),
            stop: None,
            steps: Vec::new(),
            url_template: None,
            health: None,
            default_port: None,
            env: BTreeMap::new(),
        };
        let entry = crate::registry::Entry {
            name: "resumedemo".to_owned(),
            description: "Resume demo".to_owned(),
            license: "MIT".to_owned(),
            upstream: "https://example.com/demo".to_owned(),
            source: "https://example.com/demo/downloads".to_owned(),
            unsupported_native: None,
            versions: BTreeMap::from([("1.0".to_owned(), version)]),
        };
        let package_version = entry.versions.get("1.0").expect("version");
        let package_platform = package_version
            .platforms
            .get("windows-x86_64")
            .expect("platform");
        let resolved = Resolved {
            entry: &entry,
            version: package_version,
            platform: package_platform,
        };

        let directory = tempfile::tempdir().expect("temp");
        let cache = Cache::at(directory.path().join("cache"));
        cache.prepare().expect("prepare");

        // Simulate an interrupted download: a partial file with the first
        // PREFIX bytes, as a crashed run would have left behind.
        let archive = cache.archive_path(resolved.platform);
        let part = std::path::PathBuf::from(format!("{}.part", archive.display()));
        std::fs::write(&part, &CONTENT[..PREFIX]).expect("seed part file");

        let mut progress_seen = Vec::new();
        let package = cache
            .ensure(&resolved, &mut |progress| {
                progress_seen.push(progress.downloaded)
            })
            .await
            .expect("resume and complete");

        // The first progress report must start at the part-file offset, and
        // the server must have received exactly that range request.
        assert_eq!(
            progress_seen.first(),
            Some(&(PREFIX as u64)),
            "resume did not start from the part file offset: {progress_seen:?}"
        );
        let heads = requests.lock().expect("requests lock").clone();
        assert!(
            heads
                .iter()
                .any(|head| head.to_ascii_lowercase().contains("range: bytes=16384-")),
            "the server never saw the resume range request:\n{heads:?}"
        );

        // The completed package is verified and the partial file is gone.
        assert!(package.join("bin").is_file(), "raw binary was extracted");
        assert_eq!(
            std::fs::read(package.join("bin"))
                .expect("read binary")
                .len(),
            TOTAL
        );
        assert!(archive.is_file(), "final archive missing");
        assert!(!part.exists(), "the .part file survived completion");
        assert!(cache.marker(resolved.platform.sha256.as_str()).is_some());
    }
}
