# 0007 - Crash-safe data operations

- Status: Accepted
- Date: 2026-10-26

## Context

Burrow's data commands are plain directory operations. A snapshot is a copy;
a restore stages copies and swaps one service directory at a time; a managed
package's `init` and seed commands run once per data directory. A machine crash,
a force-killed daemon, or a killed `initdb`/`psql` can therefore leave:

- a restore that replaced some service directories and not others;
- backup and staging directories with no record of which restore produced them;
- a non-empty data directory without the initialization marker, which `initdb`
  refuses to reuse;
- a seed script that ran partially and left rows behind.

Each of these is a data-loss risk if the next start guesses instead of
recovering deterministically.

## Decision

1. `burrow restore` writes a journal to
   `.burrow/data/.burrow-restore-journal.json` before it touches the first live
   directory. The journal records a nonce, the snapshot name, whether the
   restore is committed, and for each service whether it was present in the
   snapshot and whether a live directory existed before the swap. It is written
   with a temporary file and `rename`.
2. After every swap succeeds, the journal is rewritten with `committed: true`
   before any backup is deleted. A crash before that point is treated as an
   incomplete restore and rolled back; a crash after it only needs cleanup.
3. `recover_interrupted_restore` runs at daemon startup and before `snapshot`,
   `restore`, and `reset`. An uncommitted journal restores backups in reverse
   swap order and removes restored directories for services that had no
   original. A committed journal keeps the restored directories and removes the
   backups and staging directory. Recovery prints a `note:` line from the CLI
   and logs the action from the daemon.
4. A staging directory (`.burrow-restore-*`) with no journal is deleted: it was
   created before any live directory changed. A backup directory
   (`.burrow-backup-*`) with no journal is an error and is left in place. Burrow
   will not guess which directory is authoritative.
5. A non-empty data directory without the initialization marker is moved to
   `.burrow/data/.burrow-init-failed-<service>-<pid>-<unix>` before `init` is
   retried. An empty directory is left in place because `initdb` can use it.
   A directory with the marker is never moved.
6. PostgreSQL's seed command runs with `psql --single-transaction`, so a killed
   seed rolls back instead of leaving a partial data set. The seed marker is
   still written only after the command exits successfully.

## Consequences

- **Disk space.** A restore temporarily needs space for the snapshot copy, the
  pre-restore directory, and the restored directory. Recovery needs the same
  space until it finishes.
- **A committed restore cannot be undone by recovery.** The journal is the
  authority. Manually deleting it or a backup directory is unsupported and can
  make recovery refuse to start.
- **A crash between the last swap and the commit rolls back.** This is the safe
  choice: the restore was not reported as complete, so the pre-restore data is
  kept and the user can retry.
- **Orphan backups are preserved, not cleaned.** They may be the only copy of
  the data. The error names them so a human can inspect them.
- **Initialization failures accumulate quarantine directories.** Each retry
  preserves the previous partial directory rather than deleting it. Users can
  remove `.burrow-init-failed-*` directories once they no longer need the
  evidence.
- **Only PostgreSQL's seed is transactional today.** Another registry package
  with a seed script needs its own transaction or idempotent script; the marker
  only records that the command exited successfully.
- **Simulated crash coverage.** Tests exercise the journal states and the
  partial-init directory directly, and kill the daemon between swaps at the CLI
  level. A real kill during a live `initdb` or `psql` run was not exercised on
  this machine.
