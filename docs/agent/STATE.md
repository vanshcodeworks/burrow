# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite is complete. Done: orphan-kill (A2.1), interrupted
restore/init/seed (A2.2), corrupt state/ports/secrets (A2.3), port collisions
(A2.4), secrets-scan (A2.5), listener inspection (A2.6), snapshot entry hazards
(A2.7), and interrupted-download resume (A2.8).

Next: A3 fuzzing, A4 property tests, A6 `doctor --report`, then the Phase 1
compatibility matrix and clean-machine pass.

## Last cycle summary (2026-10-26, cycle 8)

- **A2.7 snapshot fidelity**: hard links are copied as independent files;
  NTFS alternate data streams are detected with `FindFirstStreamW` and refuse
  the snapshot instead of being silently dropped; reserved Windows device names
  are rejected for snapshot names, manifest service names, project names,
  service names, and `pkg add --service`.
- **Evidence**: `burrow-data` 16/16 (junction, hard link, alternate stream,
  reserved names, plus the restore/init tests), `burrow-config` 22/22, CLI
  reserved-name test 1/1. ADR 0008, `architecture.md`, `cli.md`,
  `config-reference.md`, `troubleshooting.md`, `CLAIMS.md`, and `QUEUE.md`
  updated.
- **Not exercised**: the non-Windows symlink branch (`cfg(not(windows))`) can
  only run on Linux/macOS. Stream scanning on non-NTFS filesystems treats
  "not supported" as no streams and was not exercised on FAT/exFAT.

## Check suite status (cycle 8 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  178 tests / 19 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green**, 49.00 s.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.

## Environment notes

- Windows 11 Home 10.0.26200 on an AMD Ryzen 7 4800H, 15.4 GB RAM.
- Rust 1.94.1 stable via `C:\Users\Asus\.cargo\bin`; workspace MSRV 1.85.
- Docker Desktop 24.0.7 running; Podman absent.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. A3/A4/A6, the untested Podman path, and the non-Windows symlink
branch are the open verification gaps. Phase 1 still needs a true clean-machine
pass; cycle 6 used the dev machine with a warm package cache.
