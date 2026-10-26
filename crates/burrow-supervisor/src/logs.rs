//! Per-service log capture: rotating files plus an in-memory ring buffer.
//!
//! Every line is written to a per-service log file and appended to a bounded
//! ring buffer. The ring buffer feeds `burrow logs -f` and the TUI without
//! re-reading files, and is deliberately independent of the file rotation.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::sync::watch;

/// Default number of lines retained per service in memory.
pub const DEFAULT_RING_CAPACITY: usize = 2_000;
/// Default size at which a service log file is rotated.
pub const DEFAULT_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const ROTATED_FILES: usize = 3;

/// One captured output line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Monotonic sequence number, starting at 1.
    pub seq: u64,
    /// `stdout` or `stderr`.
    pub stream: String,
    /// The line without its trailing newline.
    pub text: String,
    /// When the line was captured.
    pub at: SystemTime,
}

/// A bounded in-memory log buffer with a sequence-based change notification.
#[derive(Debug)]
pub struct LogRing {
    capacity: usize,
    next_seq: AtomicU64,
    lines: Mutex<VecDeque<LogLine>>,
    changed: watch::Sender<u64>,
}

impl LogRing {
    /// Creates a ring that keeps at most `capacity` lines.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (changed, _) = watch::channel(0);
        Self {
            capacity,
            next_seq: AtomicU64::new(1),
            lines: Mutex::new(VecDeque::with_capacity(capacity)),
            changed,
        }
    }

    /// Appends one line.
    pub fn push(&self, stream: &str, text: impl Into<String>) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let line = LogLine {
            seq,
            stream: stream.to_owned(),
            text: text.into(),
            at: SystemTime::now(),
        };
        let mut lines = self.lines.lock().unwrap_or_else(|error| error.into_inner());
        if lines.len() == self.capacity {
            lines.pop_front();
        }
        lines.push_back(line);
        let _ = self.changed.send(seq);
    }

    /// Returns up to `limit` lines with a sequence number greater than `since`.
    ///
    /// The returned lines are oldest first. `limit == 0` means no limit.
    pub fn snapshot(&self, since: u64, limit: usize) -> Vec<LogLine> {
        let lines = self.lines.lock().unwrap_or_else(|error| error.into_inner());
        let take = if limit == 0 { usize::MAX } else { limit };
        lines
            .iter()
            .filter(|line| line.seq > since)
            .take(take)
            .cloned()
            .collect()
    }

    /// The most recent sequence number, or 0 when empty.
    pub fn last_seq(&self) -> u64 {
        self.next_seq.load(Ordering::Relaxed).saturating_sub(1)
    }

    /// Subscribes to sequence-number changes, for follow-style consumers.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
}

/// Shared handle used by both output forwarders of one service.
#[derive(Clone, Debug)]
pub(crate) struct LogWriter {
    inner: Arc<Mutex<LogFile>>,
    ring: Arc<LogRing>,
}

#[derive(Debug)]
struct LogFile {
    directory: PathBuf,
    base: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl LogWriter {
    /// Opens `<directory>/<service>.log`, creating the directory and an empty
    /// log file if needed.
    pub(crate) fn open(
        directory: &Path,
        service: &str,
        max_bytes: u64,
        ring: Arc<LogRing>,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let base = directory.join(format!("{service}.log"));
        // Create the file eagerly so a diagnostic path in an error message
        // exists even when the service is killed before its first output line.
        // A failure here is not fatal: the next append retries the open.
        let mut file = None;
        let mut size = 0;
        match OpenOptions::new().create(true).append(true).open(&base) {
            Ok(opened) => {
                size = opened.metadata().map_or(0, |metadata| metadata.len());
                file = Some(opened);
            }
            Err(error) => {
                tracing::warn!(%error, path = %base.display(), "could not create service log file");
            }
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(LogFile {
                directory: directory.to_path_buf(),
                base,
                max_bytes: max_bytes.max(64),
                file,
                size,
            })),
            ring,
        })
    }

    pub(crate) fn write_line(&self, stream: &str, text: &str) {
        self.ring.push(stream, text);
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Err(error) = state.append(stream, text) {
            // Logging must never take a service down. The ring buffer still has
            // the line, and the next append retries opening the file.
            tracing::warn!(%error, "could not write service log file");
        }
    }
}

impl LogFile {
    fn append(&mut self, stream: &str, text: &str) -> std::io::Result<()> {
        let timestamp = humantime::format_rfc3339_millis(SystemTime::now());
        let mut encoded = format!("{timestamp} [{stream}] {text}\r\n").into_bytes();
        let projected = self.size + encoded.len() as u64;
        if projected > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.base)?,
            );
            self.size = self.file.as_ref().map_or(0, |file| {
                file.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            });
        }

        // The new line itself may exceed the cap; write it whole rather than
        // splitting it.
        let file = self.file.as_mut().expect("log file opened above");
        file.write_all(&encoded)?;
        file.flush()?;
        self.size += encoded.len() as u64;
        encoded.clear();
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.file = None;
        self.size = 0;
        if !self.base.exists() {
            return Ok(());
        }

        for index in (1..ROTATED_FILES).rev() {
            let from = rotated_path(&self.base, index);
            let to = rotated_path(&self.base, index + 1);
            if from.exists() {
                let _ = std::fs::remove_file(&to);
                std::fs::rename(&from, &to)?;
            }
        }
        let first = rotated_path(&self.base, 1);
        let _ = std::fs::remove_file(&first);
        std::fs::rename(&self.base, &first)?;
        debug_assert!(self.directory.exists());
        Ok(())
    }
}

fn rotated_path(base: &Path, index: usize) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "service".to_owned());
    base.with_file_name(format!("{stem}.{index}.log"))
}

#[cfg(test)]
mod tests {
    use super::{LogRing, LogWriter};

    #[test]
    fn ring_keeps_the_newest_lines_and_sequence_numbers() {
        let ring = LogRing::new(3);
        for index in 1..=5 {
            ring.push("stdout", format!("line {index}"));
        }

        let all = ring.snapshot(0, 0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].text, "line 3");
        assert_eq!(all[2].text, "line 5");
        assert_eq!(all[2].seq, 5);
        assert_eq!(ring.last_seq(), 5);

        let since = ring.snapshot(4, 0);
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].text, "line 5");
    }

    #[test]
    fn open_creates_an_empty_log_file_before_the_first_line() {
        let temp = tempfile::tempdir().expect("temp dir");
        let ring = std::sync::Arc::new(LogRing::new(10));
        let _writer = LogWriter::open(temp.path(), "api", 1024, ring).expect("writer");

        let path = temp.path().join("api.log");
        assert!(path.is_file(), "log file was not created eagerly");
        assert_eq!(path.metadata().expect("metadata").len(), 0);
    }

    #[test]
    fn writer_rotates_files_and_keeps_logging() {
        let temp = tempfile::tempdir().expect("temp dir");
        let ring = std::sync::Arc::new(LogRing::new(100));
        let writer = LogWriter::open(temp.path(), "api", 120, ring.clone()).expect("writer");

        for index in 0..10 {
            writer.write_line("stdout", &format!("a fairly long line {index}"));
        }

        let current = temp.path().join("api.log");
        let first = temp.path().join("api.1.log");
        assert!(current.is_file());
        assert!(first.is_file());
        assert!(ring.snapshot(0, 0).len() == 10);
        let contents = std::fs::read_to_string(&current).expect("read current log");
        assert!(contents.contains("[stdout]"), "{contents}");
    }
}
