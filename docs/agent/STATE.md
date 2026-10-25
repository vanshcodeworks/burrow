# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite: orphan-kill (A2.1), secrets-scan (A2.5), and
junction rejection (A2.7 partial) are done. Next: A2.2 interrupted
data operations, A2.3 stale-state corruption, A2.4 port collisions,
A2.6 non-loopback detection, A2.8 download-resume test.

## Last cycle summary (2026-10-25, cycles 2-3)

- **Security**: zip upgraded 5.1.1 → 9.1.0 (`fbd6459`), clearing
  RUSTSEC-2026-0367/0368/0369 (up to 9.1 critical, in the unused
  `ZipArchive::extract`); `cargo audit` now green locally (3 allowed
  transitive warnings: paste unmaintained, lru unsound ×2).
- **Adversarial tests landed** (three commits after fbd6459):
  - `test(data)` junction inside a data dir is refused (real `mklink /J`,
    error names the path, no snapshot left behind).
  - `test(cli)` force-killed daemon (`taskkill /F`, no /T) → service port
    refuses within 15 s, then `burrow up` recovers with the same stable
    port. 3/3 runs passed.
  - `test(cli)` generated secrets scanned against daemon.log, all service
    logs, state.json, status (human+json), logs, project-level env, and a
    snapshot; positive control: `burrow env <service>` must print it.
- Sub agents produced the junction and orphan tests (A2.5 written inline
  after two failed spawn attempts: concurrency limit, then DNS error).

## Check suite status (last full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  154 tests / 15 suites**. Note: the phase2 suite took 272 s this run —
  the live container test pulled its Docker image fresh (cold pull);
  normally ~21 s with the image cached.
- `cargo audit`: **green** (installed locally at `~/.cargo/bin`).
- Last green commit: adversarial-tests triple commit on `agent/work`.
- Tree clean; all work committed.

## Environment notes

- Windows 10 x64 (OS build 10.0.26200), Rust 1.94.1 stable via
  `C:\Users\Asus\.cargo\bin` (export PATH in every Git Bash shell).
- Docker Desktop 24.0.7 running (started once; container fallback test
  verified live). cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID`
  args inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. A-flake-1 (health-timeout wall-clock assert) unaddressed;
phase2 suite duration under cold image pull is acceptable but noted.
