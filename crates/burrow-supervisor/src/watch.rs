//! Recursive directory watching for `watch = [...]` services.
//!
//! One OS watch thread per root reports `ReadDirectoryChangesW` events into a
//! standard channel. The watcher has no opinion about what to do with changes:
//! the daemon maps events to services and debounces restarts. Events are
//! best-effort; a buffer overflow surfaces as [`WatchAction::Overflow`] instead
//! of silent loss.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// What happened to one path inside a watched root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchAction {
    /// A file or directory was created.
    Added,
    /// A file or directory was deleted.
    Removed,
    /// A file's contents or attributes changed.
    Modified,
    /// A file or directory was renamed to this path.
    Renamed,
    /// An event the daemon does not act on individually.
    Other,
    /// The kernel buffer overflowed and changes were lost.
    Overflow,
}

/// One filesystem change inside a watched root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    /// The watched root the event is relative to.
    pub root: PathBuf,
    /// Path of the changed entry relative to `root`.
    pub relative: PathBuf,
    /// What happened.
    pub action: WatchAction,
}

/// Watches a set of directory roots recursively.
///
/// Dropping the watcher stops every watch thread and joins it.
pub struct DirectoryWatcher {
    receiver: std::sync::mpsc::Receiver<WatchEvent>,
    _watches: Vec<crate::win::DirectoryWatch>,
}

impl DirectoryWatcher {
    /// Starts one recursive watch per root.
    ///
    /// Every root must exist and be a directory; a missing root is an error so
    /// configuration mistakes surface at startup instead of silently watching
    /// nothing.
    pub fn new(roots: &[PathBuf]) -> Result<Self> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watches = Vec::with_capacity(roots.len());
        for root in roots {
            if !root.is_dir() {
                return Err(Error::InvalidSpec(
                    "watch root must be an existing directory",
                ));
            }
            watches.push(crate::win::spawn_directory_watch(root, sender.clone())?);
        }
        Ok(Self {
            receiver,
            _watches: watches,
        })
    }

    /// Drains all events that have arrived so far.
    ///
    /// Returns an empty vector when nothing changed; callers poll this on their
    /// own tick.
    pub fn poll(&self) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryWatcher, WatchAction};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// Collects events for up to `timeout`, returning once `want` matched.
    fn collect_until(
        watcher: &DirectoryWatcher,
        timeout: Duration,
        mut want: impl FnMut(&super::WatchEvent) -> bool,
    ) -> Vec<super::WatchEvent> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            for event in watcher.poll() {
                if want(&event) {
                    seen.push(event);
                    if seen.len() == 1 {
                        return seen;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        seen
    }

    #[test]
    fn reports_file_changes_under_a_root() {
        let root = tempfile::tempdir().expect("temp dir");
        let sub = root.path().join("src");
        std::fs::create_dir_all(&sub).expect("create subdirectory");

        let watcher = DirectoryWatcher::new(&[root.path().to_path_buf()]).expect("watch");
        std::fs::write(sub.join("main.rs"), b"fn main() {}").expect("write file");

        let events = collect_until(&watcher, Duration::from_secs(5), |event| {
            event.relative.ends_with("main.rs")
                && matches!(
                    event.action,
                    WatchAction::Added | WatchAction::Modified | WatchAction::Renamed
                )
        });
        assert!(!events.is_empty(), "no event for the created file");
        assert_eq!(events[0].root, root.path());

        std::fs::write(sub.join("main.rs"), b"fn main() { }").expect("modify file");
        let modified = collect_until(&watcher, Duration::from_secs(5), |event| {
            event.relative.ends_with("main.rs") && event.action == WatchAction::Modified
        });
        assert!(!modified.is_empty(), "no event for the modified file");

        std::fs::remove_file(sub.join("main.rs")).expect("remove file");
        let removed = collect_until(&watcher, Duration::from_secs(5), |event| {
            event.relative.ends_with("main.rs") && event.action == WatchAction::Removed
        });
        assert!(!removed.is_empty(), "no event for the removed file");
    }

    #[test]
    fn rejects_roots_that_do_not_exist() {
        let missing = PathBuf::from(r"C:\burrow-definitely-missing-root");
        assert!(DirectoryWatcher::new(&[missing]).is_err());
    }
}
