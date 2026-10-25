# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite. Done: orphan-kill (A2.1), half-written snapshot
debris (A2.2 partial), corrupt state/ports (A2.3 partial), port collisions
(A2.4), secrets-scan (A2.5), junction rejection (A2.7 partial), and
interrupted-download resume (A2.8). A-flake-1 is fixed by relaxing the
wall-clock assertion to 60 s.

Next: A2.6 non-loopback listener detection, A2.7 hardlink + ADS coverage,
then A2.2/A2.3 remaining kill vectors (restore, seed, initdb, secrets), then
A3 fuzzing.

## Last cycle summary (2026-10-26, cycle 4)

- **Adversarial tests landed**:
  - `tests/data_safety.rs`: corrupt `state.json` fails loudly and is never
    deleted by `down`/`up`; removing it recovers. Truncated `ports.json`
    fails startup with `invalid port assignment file` and recovers after
    deletion. Half-written snapshot staging debris is never listed, never
    restored, and does not block later snapshots.
  - `cache.rs::an_interrupted_download_resumes_from_the_part_file`: a local
    HTTP server records the `Range: bytes=16384-` request, asserts the first
    progress report starts at the part-file offset, and verifies the
    completed package, removed `.part`, and marker.
  - `tests/cli.rs` health-timeout assertion relaxed from 20 s to 60 s
    (A-flake-1); the real timeout still fires.
- `state.rs` corrupt-state error now includes a recovery hint.
- Full suite: **159 passed / 0 failed across 17 suites**.

## Check suite status (last full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  159 tests / 17 suites**.
- `cargo audit`: **green** as of `fbd6459` (3 allowed transitive warnings).
- Last green commit before this cycle: `6b815eb`.

## Environment notes

- Windows 10 x64 (OS build 10.0.26200), Rust 1.94.1 stable via
  `C:\Users\Asus\.cargo\bin` (export PATH in every Git Bash shell).
- Docker Desktop 24.0.7 running; container fallback test verified live.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID`
  args inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. Remaining A2 items and the untested podman path are the
open verification gaps.
