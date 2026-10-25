# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite is complete except A2.7 hardlink/ADS coverage.
Done: orphan-kill (A2.1), interrupted restore/init/seed (A2.2), corrupt
state/ports/secrets (A2.3), port collisions (A2.4), secrets-scan (A2.5),
listener inspection (A2.6), junction rejection (A2.7 partial), and
interrupted-download resume (A2.8).

Next: A2.7 hardlink/ADS/reserved-name coverage, then A3 fuzzing, A4 property
tests, A6 `doctor --report`, then the Phase 1 compatibility matrix and
clean-machine pass.

## Last cycle summary (2026-10-26, cycle 7)

- **A2.2 crash-safe data operations**: `burrow restore` writes
  `.burrow/data/.burrow-restore-journal.json` before the first swap and marks it
  committed only after every swap. Daemon startup and `snapshot`/`restore`/
  `reset` roll an uncommitted restore back (or finish cleanup for a committed
  one). Staging without a journal is removed; an orphan backup is refused and
  preserved. Non-empty uninitialized data directories are moved to
  `.burrow-init-failed-*` before `initdb` retries; empty directories are left.
  PostgreSQL seeds run with `psql --single-transaction`.
- **A2.3 corrupt secrets**: `doctor` now parses `.burrow/secrets` with the same
  schema as `up`/`env`. A corrupt file fails `up`/`env` and the `secrets` check
  with a delete-to-regenerate remedy, is never overwritten, and leaves the
  running daemon and its `state.json` identity fields intact; `down` still
  works and deleting the file regenerates secrets.
- **Evidence**: ADR 0007, `architecture.md`, `cli.md`, `security.md`,
  `troubleshooting.md`, `CLAIMS.md`, and `QUEUE.md` updated. New tests:
  `burrow-data` 13/13, `burrow-cli` doctor 6/6, `tests/data_safety.rs` 6/6.
- **Not exercised**: a real kill during a live `initdb` or `psql` seed. Recovery
  is proven at the journal/data-directory level and by killing between swaps.

## Check suite status (cycle 7 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  173 tests / 19 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green**, 52.20 s.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.

## Environment notes

- Windows 11 Home 10.0.26200 on an AMD Ryzen 7 4800H, 15.4 GB RAM.
- Rust 1.94.1 stable via `C:\Users\Asus\.cargo\bin`; workspace MSRV 1.85.
- Docker Desktop 24.0.7 running; Podman absent.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. Remaining A2.7, A3/A4/A6, and the untested Podman path are the
open verification gaps. Phase 1 still needs a true clean-machine pass; cycle 6
used the dev machine with a warm package cache.
