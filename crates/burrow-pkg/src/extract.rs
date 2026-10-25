//! Hardened archive extraction.
//!
//! Every entry is validated before it is written:
//!
//! * absolute paths, drive letters, `..`, backslashes, and reserved Windows
//!   device names are rejected (zip-slip and path traversal);
//! * symbolic links, hard links, and other special entries are rejected;
//! * total uncompressed size and entry count are bounded (zip bombs);
//! * files are created with `create_new`, so an archive cannot follow a link
//!   planted by an earlier entry.
//!
//! Callers extract into a fresh temporary directory and rename it into the
//! content-addressed cache only after extraction succeeds.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::registry::{validate_relative, ArchiveKind};

/// Maximum number of entries accepted from one archive.
const MAX_ENTRIES: usize = 250_000;
/// Maximum uncompressed bytes accepted from one archive (8 GiB).
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Maximum size of one extracted file (4 GiB).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Maximum number of path components in one entry.
const MAX_COMPONENTS: usize = 64;

/// Extracts `archive` of `kind` into `destination`.
///
/// `destination` must be missing or empty. On failure, partially written files
/// are left for the caller to delete with the temporary directory.
pub fn extract(archive: &Path, kind: ArchiveKind, destination: &Path) -> Result<()> {
    if destination.exists() {
        let mut entries =
            std::fs::read_dir(destination).map_err(|error| Error::io(destination, error))?;
        if entries.next().is_some() {
            return Err(Error::Archive {
                message: format!("destination {} is not empty", destination.display()),
            });
        }
    } else {
        std::fs::create_dir_all(destination).map_err(|error| Error::io(destination, error))?;
    }

    match kind {
        ArchiveKind::Zip => extract_zip(archive, destination),
        ArchiveKind::TarGz => {
            let file = File::open(archive).map_err(|error| Error::io(archive, error))?;
            extract_tar(flate2::read::GzDecoder::new(file), destination)
        }
        ArchiveKind::Tar => {
            let file = File::open(archive).map_err(|error| Error::io(archive, error))?;
            extract_tar(file, destination)
        }
        ArchiveKind::Raw => {
            let target = destination.join("bin");
            std::fs::copy(archive, &target).map_err(|error| Error::io(&target, error))?;
            Ok(())
        }
    }
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive).map_err(|error| Error::io(archive, error))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| Error::Archive {
        message: format!(
            "could not read {} as a zip archive: {error}",
            archive.display()
        ),
    })?;
    if zip.len() > MAX_ENTRIES {
        return Err(Error::Archive {
            message: format!(
                "zip archive has {} entries; the limit is {MAX_ENTRIES}",
                zip.len()
            ),
        });
    }

    let mut budget = Budget::default();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| Error::Archive {
            message: format!("could not read zip entry #{index}: {error}"),
        })?;
        let raw_name = entry
            .name()
            .map(|name| name.into_owned())
            .map_err(|error| Error::Archive {
                message: format!("zip entry #{index} has an invalid name: {error}"),
            })?;
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170_000 == 0o120_000 {
                return Err(Error::UnsafeEntry {
                    entry: raw_name,
                    reason: "symbolic links are not extracted".to_owned(),
                });
            }
        }
        let relative = safe_relative(&raw_name)?;
        if entry.is_dir() {
            let target = destination.join(&relative);
            std::fs::create_dir_all(&target).map_err(|error| Error::io(&target, error))?;
            continue;
        }
        let declared = entry.size();
        budget.reserve(&raw_name, declared)?;
        let target = destination.join(&relative);
        create_parents(&target)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| Error::io(&target, error))?;
        let written = copy_limited(&mut entry, &mut output, MAX_FILE_BYTES).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                Error::Archive {
                    message: format!("zip entry `{raw_name}` expands beyond the size limit"),
                }
            } else {
                Error::io(&target, error)
            }
        })?;
        budget.charge(&raw_name, written)?;
        output.flush().map_err(|error| Error::io(&target, error))?;
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, destination: &Path) -> Result<()> {
    let mut tar = tar::Archive::new(reader);
    let entries = tar.entries().map_err(|error| Error::Archive {
        message: format!("could not read tar archive: {error}"),
    })?;
    let mut budget = Budget::default();
    let mut count = 0usize;

    for entry in entries {
        count += 1;
        if count > MAX_ENTRIES {
            return Err(Error::Archive {
                message: format!("tar archive has more than {MAX_ENTRIES} entries"),
            });
        }
        let mut entry = entry.map_err(|error| Error::Archive {
            message: format!("could not read tar entry #{count}: {error}"),
        })?;
        let kind = entry.header().entry_type();
        let path = entry.path().map_err(|error| Error::Archive {
            message: format!("tar entry #{count} has an invalid path: {error}"),
        })?;
        let raw_name = path.to_str().ok_or_else(|| Error::Archive {
            message: format!("tar entry #{count} has a non-UTF-8 path"),
        })?;
        let raw_name = raw_name.to_owned();
        drop(path);

        if !kind.is_file() && !kind.is_dir() {
            return Err(Error::UnsafeEntry {
                entry: raw_name,
                reason: format!("entry type {kind:?} is not a regular file or directory"),
            });
        }
        let relative = safe_relative(&raw_name)?;
        if kind.is_dir() {
            let target = destination.join(&relative);
            std::fs::create_dir_all(&target).map_err(|error| Error::io(&target, error))?;
            continue;
        }

        let declared = entry.header().size().map_err(|error| Error::Archive {
            message: format!("tar entry `{raw_name}` has an invalid size: {error}"),
        })?;
        budget.reserve(&raw_name, declared)?;
        let target = destination.join(&relative);
        create_parents(&target)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| Error::io(&target, error))?;
        let written = copy_limited(&mut entry, &mut output, MAX_FILE_BYTES).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                Error::Archive {
                    message: format!("tar entry `{raw_name}` expands beyond the size limit"),
                }
            } else {
                Error::io(&target, error)
            }
        })?;
        budget.charge(&raw_name, written)?;
        output.flush().map_err(|error| Error::io(&target, error))?;
    }
    Ok(())
}

/// Normalizes an archive entry name and rejects anything that could escape.
fn safe_relative(raw_name: &str) -> Result<PathBuf> {
    let mut name = raw_name.trim_end_matches('/');
    while let Some(stripped) = name.strip_prefix("./") {
        name = stripped;
    }
    if name.is_empty() {
        return Err(Error::UnsafeEntry {
            entry: raw_name.to_owned(),
            reason: "empty path".to_owned(),
        });
    }
    if name.len() > 512 {
        return Err(Error::UnsafeEntry {
            entry: raw_name.to_owned(),
            reason: "path is longer than 512 bytes".to_owned(),
        });
    }
    let relative = validate_relative(name).map_err(|reason| Error::UnsafeEntry {
        entry: raw_name.to_owned(),
        reason,
    })?;
    if relative.components().count() > MAX_COMPONENTS {
        return Err(Error::UnsafeEntry {
            entry: raw_name.to_owned(),
            reason: format!("path has more than {MAX_COMPONENTS} components"),
        });
    }
    Ok(relative)
}

fn create_parents(target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    Ok(())
}

fn copy_limited(reader: &mut impl Read, writer: &mut impl Write, limit: u64) -> io::Result<u64> {
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(total);
        }
        total = total.saturating_add(read as u64);
        if total > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "entry exceeds the size limit",
            ));
        }
        writer.write_all(&buffer[..read])?;
    }
}

#[derive(Default)]
struct Budget {
    declared: u64,
    written: u64,
}

impl Budget {
    fn reserve(&mut self, entry: &str, size: u64) -> Result<()> {
        self.declared = self.declared.saturating_add(size);
        if self.declared > MAX_TOTAL_BYTES {
            return Err(Error::Archive {
                message: format!(
                    "archive expands to more than {} GiB at entry `{entry}`",
                    MAX_TOTAL_BYTES / (1024 * 1024 * 1024)
                ),
            });
        }
        Ok(())
    }

    fn charge(&mut self, entry: &str, size: u64) -> Result<()> {
        self.written = self.written.saturating_add(size);
        if self.written > MAX_TOTAL_BYTES {
            return Err(Error::Archive {
                message: format!(
                    "archive wrote more than {} GiB at entry `{entry}`",
                    MAX_TOTAL_BYTES / (1024 * 1024 * 1024)
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::extract;
    use crate::registry::ArchiveKind;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in entries {
            zip.start_file(*name, options).expect("start zip entry");
            zip.write_all(contents).expect("write zip entry");
        }
        zip.finish().expect("finish zip");
    }

    #[test]
    fn extracts_a_normal_zip_with_nested_directories() {
        let directory = temp();
        let archive = directory.path().join("normal.zip");
        write_zip(
            &archive,
            &[("bin/tool.exe", b"binary"), ("./share/data.txt", b"data")],
        );
        let output = directory.path().join("out");
        extract(&archive, ArchiveKind::Zip, &output).expect("extract");
        assert_eq!(
            std::fs::read(output.join("bin/tool.exe")).unwrap(),
            b"binary"
        );
        assert_eq!(
            std::fs::read(output.join("share/data.txt")).unwrap(),
            b"data"
        );
    }

    #[test]
    fn rejects_zip_slip_entries() {
        let directory = temp();
        let archive = directory.path().join("evil.zip");
        write_zip(&archive, &[("../escape.txt", b"nope")]);
        let output = directory.path().join("out");
        let error = extract(&archive, ArchiveKind::Zip, &output).expect_err("must reject");
        let text = error.to_string();
        assert!(text.contains("unsafe"), "error was: {text}");
        assert!(!directory.path().join("escape.txt").exists());
    }

    #[test]
    fn rejects_absolute_and_reserved_names() {
        for name in [
            "C:/Windows/evil.txt",
            "CON",
            "bin/trailing. ",
            "bin/colon:name",
        ] {
            let directory = temp();
            let archive = directory.path().join("evil.zip");
            write_zip(&archive, &[(name, b"nope")]);
            let output = directory.path().join("out");
            let error = extract(&archive, ArchiveKind::Zip, &output).expect_err(name);
            assert!(error.to_string().contains("unsafe"), "{name}: {error}");
        }
    }

    #[test]
    fn rejects_tar_path_traversal() {
        let directory = temp();
        let archive = directory.path().join("evil.tar");
        let file = std::fs::File::create(&archive).expect("create tar");
        let mut builder = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        // `append_data` refuses to build malicious paths, so write the raw
        // header bytes exactly like a hostile archive would.
        let name = b"../escape.txt";
        let bytes = header.as_mut_bytes();
        bytes[..name.len()].copy_from_slice(name);
        header.set_cksum();
        builder
            .append(&header, &b"nope"[..])
            .expect("append raw header");
        builder.finish().expect("finish");
        let output = directory.path().join("out");
        let error = extract(&archive, ArchiveKind::Tar, &output).expect_err("must reject");
        assert!(error.to_string().contains("unsafe"), "error: {error}");
    }

    #[test]
    fn rejects_tar_symlinks() {
        let directory = temp();
        let archive = directory.path().join("link.tar");
        let file = std::fs::File::create(&archive).expect("create tar");
        let mut builder = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_cksum();
        builder
            .append_link(&mut header, "link", "C:/Windows/System32")
            .expect("append link");
        builder.finish().expect("finish");
        let output = directory.path().join("out");
        let error = extract(&archive, ArchiveKind::Tar, &output).expect_err("must reject");
        assert!(error.to_string().contains("unsafe"), "error: {error}");
    }

    #[test]
    fn refuses_to_extract_into_a_non_empty_directory() {
        let directory = temp();
        let archive = directory.path().join("normal.zip");
        write_zip(&archive, &[("file.txt", b"data")]);
        let output = directory.path().join("out");
        std::fs::create_dir_all(&output).expect("create output");
        std::fs::write(output.join("existing.txt"), b"keep").expect("write existing");
        let error = extract(&archive, ArchiveKind::Zip, &output).expect_err("must refuse");
        assert!(error.to_string().contains("not empty"), "error: {error}");
        assert_eq!(std::fs::read(output.join("existing.txt")).unwrap(), b"keep");
    }
}
