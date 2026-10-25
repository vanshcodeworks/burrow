//! Structured crash notes written when the CLI panics.
//!
//! Notes are local files under `%LOCALAPPDATA%\burrow\crashes` (or the
//! temporary directory when `LOCALAPPDATA` is unavailable). They never leave
//! the machine on their own; `burrow doctor --report` can copy up to ten
//! matching notes into a diagnostic bundle after redaction.
//!
//! Writing must never panic or recurse into the panic hook. Every failure is
//! deliberately ignored: a lost crash note is better than a lost error message.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Schema version of a crash note.
pub const CRASH_VERSION: u32 = 1;
/// Maximum size of a captured backtrace, to keep notes small.
const MAX_BACKTRACE_BYTES: usize = 64 * 1024;
/// Directory name under `%LOCALAPPDATA%\burrow`.
const CRASH_DIR: &str = "crashes";

static PROJECT_DIR: OnceLock<PathBuf> = OnceLock::new();
static INSTALLED: Once = Once::new();

/// One structured crash note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashNote {
    /// Schema version.
    pub version: u32,
    /// RFC 3339 timestamp with millisecond precision.
    pub timestamp: String,
    /// Unix time in milliseconds, used for ordering.
    pub unix_ms: u64,
    /// Process id.
    pub pid: u32,
    /// Thread name, or `<unnamed>`.
    pub thread: String,
    /// Burrow version that panicked.
    pub burrow_version: String,
    /// Operating system, for example `windows`.
    pub os: String,
    /// CPU architecture, for example `x86_64`.
    pub arch: String,
    /// Project directory, when the command had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<PathBuf>,
    /// Panic message.
    pub message: String,
    /// Source location, when the panic carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Captured backtrace, when one could be produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
}

/// A crash note read back from disk.
#[derive(Debug, Clone)]
pub struct StoredCrash {
    /// File the note was read from.
    pub path: PathBuf,
    /// Parsed note.
    pub note: CrashNote,
}

/// Directory where crash notes are stored.
pub fn directory() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(base) => PathBuf::from(base).join("burrow").join(CRASH_DIR),
        None => std::env::temp_dir().join("burrow").join(CRASH_DIR),
    }
}

/// Records the project directory used for later panic notes.
///
/// Only the first call has an effect. A `None` value leaves notes without a
/// project directory, which makes them visible to every project.
pub fn set_project_dir(project_dir: Option<PathBuf>) {
    if let Some(directory) = project_dir {
        let _ = PROJECT_DIR.set(directory);
    }
}

/// Installs the panic hook exactly once, chaining to the previous hook.
///
/// The note is written before the previous hook runs so a custom hook that
/// aborts the process cannot lose it.
pub fn install_hook() {
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = record_panic(info);
            previous(info);
        }));
    });
}

fn record_panic(info: &std::panic::PanicHookInfo<'_>) -> Option<PathBuf> {
    let now = SystemTime::now();
    let unix_ms = now
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let note = CrashNote {
        version: CRASH_VERSION,
        timestamp: humantime::format_rfc3339_millis(now).to_string(),
        unix_ms,
        pid: std::process::id(),
        thread: std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_owned(),
        burrow_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        project_dir: PROJECT_DIR.get().cloned(),
        message: panic_message(info.payload()),
        location: info.location().map(|location| location.to_string()),
        backtrace: capture_backtrace(),
    };
    write_note_in(&directory(), &note).ok()
}

/// Extracts a printable message from a panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

fn capture_backtrace() -> Option<String> {
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    if backtrace.trim().is_empty() {
        return None;
    }
    if backtrace.len() > MAX_BACKTRACE_BYTES {
        let mut truncated = backtrace;
        let mut end = MAX_BACKTRACE_BYTES;
        while end > 0 && !truncated.is_char_boundary(end) {
            end -= 1;
        }
        truncated.truncate(end);
        truncated.push_str("\n[backtrace truncated]\n");
        Some(truncated)
    } else {
        Some(backtrace)
    }
}

/// Writes a note atomically into `dir` and returns its path.
fn write_note_in(dir: &Path, note: &CrashNote) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let encoded = serde_json::to_vec_pretty(note)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let base = format!("crash-{}-{}", note.unix_ms, note.pid);
    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            format!("{base}.json")
        } else {
            format!("{base}-{attempt}.json")
        };
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, &encoded)?;
        match std::fs::rename(&temporary, &path) {
            Ok(()) => return Ok(path),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                if error.kind() == std::io::ErrorKind::AlreadyExists || path.exists() {
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not choose a unique crash-note name",
    ))
}

/// Returns the newest notes for a project, including notes without a project.
///
/// Invalid or unreadable files are ignored; a corrupt note must not break
/// `doctor --report`.
pub fn recent(project_dir: &Path, limit: usize) -> Vec<StoredCrash> {
    recent_in(&directory(), project_dir, limit)
}

fn recent_in(dir: &Path, project_dir: &Path, limit: usize) -> Vec<StoredCrash> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(note) = serde_json::from_str::<CrashNote>(&contents) else {
            continue;
        };
        let matches = note
            .project_dir
            .as_deref()
            .is_none_or(|directory| same_path(directory, project_dir));
        if matches {
            notes.push(StoredCrash { path, note });
        }
    }
    notes.sort_by(|left, right| {
        right
            .note
            .unix_ms
            .cmp(&left.note.unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    notes.truncate(limit);
    notes
}

fn same_path(left: &Path, right: &Path) -> bool {
    comparable_path(left).eq_ignore_ascii_case(&comparable_path(right))
}

/// Strips a Windows verbatim prefix so notes written by different code paths
/// still match the canonical project directory.
fn comparable_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{panic_message, recent_in, write_note_in, CrashNote, CRASH_VERSION};

    fn note(unix_ms: u64, project: Option<&str>) -> CrashNote {
        CrashNote {
            version: CRASH_VERSION,
            timestamp: "2026-10-26T12:00:00.000Z".to_owned(),
            unix_ms,
            pid: 42,
            thread: "main".to_owned(),
            burrow_version: "0.1.0".to_owned(),
            os: "windows".to_owned(),
            arch: "x86_64".to_owned(),
            project_dir: project.map(std::path::PathBuf::from),
            message: "boom".to_owned(),
            location: Some("src/main.rs:1:1".to_owned()),
            backtrace: None,
        }
    }

    #[test]
    fn notes_round_trip_and_omit_empty_optional_fields() {
        let encoded = serde_json::to_value(note(1, None)).expect("serialize note");
        assert_eq!(encoded["version"], 1);
        assert!(encoded.get("project_dir").is_none());
        assert!(encoded.get("backtrace").is_none());
        let decoded: CrashNote = serde_json::from_value(encoded).expect("deserialize note");
        assert_eq!(decoded, note(1, None));
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note_in(dir.path(), &note(7, Some(r"C:\shop"))).expect("write note");
        assert!(path.is_file());
        assert!(!path.with_extension("json.tmp").exists());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(names.len(), 1, "unexpected files: {names:?}");
    }

    #[test]
    fn recent_notes_are_filtered_by_project_and_newest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_note_in(dir.path(), &note(10, Some(r"C:\shop"))).expect("write shop");
        write_note_in(dir.path(), &note(30, None)).expect("write global");
        write_note_in(dir.path(), &note(20, Some(r"C:\other"))).expect("write other");

        let notes = recent_in(dir.path(), std::path::Path::new(r"C:\shop"), 10);
        let times: Vec<u64> = notes.iter().map(|stored| stored.note.unix_ms).collect();
        assert_eq!(times, [30, 10]);
    }

    #[test]
    fn recent_notes_honor_the_limit_and_skip_corrupt_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_note_in(dir.path(), &note(1, None)).expect("write one");
        write_note_in(dir.path(), &note(2, None)).expect("write two");
        std::fs::write(dir.path().join("broken.json"), "{").expect("write broken");

        let notes = recent_in(dir.path(), std::path::Path::new(r"C:\shop"), 1);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note.unix_ms, 2);
        assert!(recent_in(dir.path(), std::path::Path::new(r"C:\shop"), 0).is_empty());
    }

    #[test]
    fn panic_payloads_are_rendered_without_debug_quotes() {
        assert_eq!(panic_message(&"plain"), "plain");
        assert_eq!(panic_message(&String::from("owned")), "owned");
        assert_eq!(panic_message(&42u32), "<non-string panic payload>");
    }
}
