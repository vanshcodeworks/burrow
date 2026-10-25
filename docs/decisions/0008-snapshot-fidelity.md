# 0008 - Snapshot fidelity on Windows

- Status: Accepted
- Date: 2026-10-26

## Context

Snapshots are plain directory copies. On Windows, a directory entry can carry
more than the bytes of its default stream:

- a hard link shares one file record with another path, so copying it produces
  an independent file;
- an NTFS alternate data stream (for example `record.txt:metadata`) is a second
  byte stream that `std::fs::copy` does not copy;
- names such as `con`, `nul`, `com1`, and `lpt1` refer to devices even with an
  extension, so a service or snapshot named `con` does not create an ordinary
  directory.

Silently losing a stream or writing to a device path would violate the same
rule as following a junction: a snapshot must not claim to contain data it
does not contain.

## Decision

1. Hard links are copied as independent files. Contents and byte counts are
   preserved; the link relationship is not. This is documented rather than
   rejected because hard links are ordinary files with no external path, and
   refusing them would block legitimate data directories.
2. Before copying a file or directory, Burrow enumerates its NTFS streams with
   `FindFirstStreamW`/`FindNextStreamW`. Any stream other than the default
   `::$DATA` refuses the snapshot with `alternate data stream ... would not be
   copied`. On filesystems that do not support streams, the "not supported"
   errors are treated as an empty stream list.
3. Reserved Windows device names are rejected when configuration is parsed
   (project and service names) and when a snapshot name or manifest service name
   is validated. The check is case-insensitive and applies to the stem before
   the first dot.
4. The checks run before the snapshot is renamed into place, so a refused
   snapshot leaves no listed entry and no staging debris beyond the existing
   temporary-directory cleanup.

## Consequences

- **A data directory with an alternate stream cannot be snapshotted** until the
  stream is removed or the file is moved out. This is intentional: a partial
  copy is worse than a clear failure. The error names the file and stream.
- **Stream scanning costs one extra query per file and directory.** For a large
  PostgreSQL cluster this is measurable but small compared with copying the
  bytes.
- **Hard-link identity is not restored.** Applications that depend on two paths
  sharing an inode must recreate the link after a restore.
- **The reserved-name list is duplicated in `burrow-config` and `burrow-data`.**
  They validate different inputs at different layers and cannot share a crate
  without adding a dependency for a small, stable OS rule.
- **Non-Windows symlinks are still covered only by the generic reparse-point
  helper.** The Windows junction test and the alternate-stream tests run here;
  a Linux/macOS symlink test has to run on those platforms.
