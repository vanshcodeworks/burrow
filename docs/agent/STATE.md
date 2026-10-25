# Agent state (overwrite each cycle)

## Current focus

Phase 1's executable work is done: the compatibility matrix (A5), a cold-cache
README/time-to-first-success pass, and the error-message pass are complete.
What remains is a true clean-machine VM/Sandbox run and a CI execution of the
compatibility probe; neither is possible on this machine. Do not write
`docs/agent/LAUNCH_READINESS.md` until those and the human-only launch steps
are resolved.

## Last cycle summary (2026-10-26, cycle 13)

- **Time to first success**: followed the README from a fresh `git archive` of
  `examples/node-postgres` with empty `CARGO_HOME`, npm cache, and
  `BURROW_CACHE_DIR`. `cargo install --path crates\burrow-cli --locked` took
  **162.62 s** and downloaded 0.27 GB of crates; `npm install` took **3.19 s**;
  cold `burrow up --detach --yes` to both healthy and a seeded HTTP 200 took
  **79.43 s**. Checkout to first successful request: **245.24 s (4 min 5 s)**.
  A warm restart took **1.87 s**; `doctor` reported 11 ok / 0 warnings /
  0 failures. Recorded in `docs/benchmarks.md`, with caveats, and linked from
  the README.
- **`api.localhost` did not resolve here** (`DNS name does not exist`); the
  direct `http://127.0.0.1:<port>/api/todos` URL returned the seeded row. This
  is the documented fallback, not a Burrow failure.
- **Error-message pass**: fixed three misleading messages and verified three
  runtime failures.
  - Every config error used to append `note: expected burrow.toml ... help: run
    burrow init`, including parse errors, unknown fields, cycles, and reserved
    names. Now only a missing `burrow.toml` gets that hint; existing-but-invalid
    files show only the real error (`project::config_error`, 3 unit tests).
  - `restore <missing>` now adds `help: run burrow snapshots to list existing
    snapshots`; the existing snapshot integration test asserts the message.
  - `exec`, `env`, and `up` with an unknown service used to say `invalid
    configuration at services: unknown service ...`; they now say `unknown
    service ... help: choose one of: ...` (`project::ensure_known_services`,
    2 unit tests).
  - Probed `run = "definitely-not-a-command-xyz"`, a 2 s HTTP health timeout,
    and a fixed port held by a real listener. Each exits 1 with an actionable
    message, removes `.burrow/state.json`, and leaves no `burrow` process.

## Check suite status (cycle 13 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  211 tests / 23 suites** (5 new `project::tests`).
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 127.94 s** (a rebuild after
  the error-message changes; slower than the 54.47 s cycle-12 build).
- `scripts/compatibility.ps1`: **4/4** under PowerShell 7.6.7 and Windows
  PowerShell 5.1.26100.9711, including a 279-character path.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.
- Fuzz targets are excluded from the workspace; see `fuzz/README.md`.

## Environment notes

- Windows 11 Home Single Language 10.0.26200 build 26200 on an AMD Ryzen 7
  4800H (8 cores / 16 threads), 15.4 GB RAM; `LongPathsEnabled=1`.
- Probe sessions: PowerShell 7.6.7 and Windows PowerShell 5.1.26100.9711,
  non-elevated (medium integrity), locale `en-NZ`.
- Rust 1.94.1 stable via `C:\Users\Asus\.cargo\bin`; workspace MSRV 1.85.
  Node 22.13.1, npm 10.9.2.
- Nightly `1.101.0-nightly (12433c307 2026-10-24)` and `cargo-fuzz 0.13.2`
  installed locally for A3.
- Windows ASan needs
  `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64`
  on `PATH` (`clang_rt.asan_dynamic-x86_64.dll`).
- Docker Desktop 24.0.7 running; Podman absent.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.
- The cycle-13 quick-start pass used empty Cargo, npm, and Burrow caches, but
  the OS, Rust toolchain, MSVC build tools, Node, and npm were already
  installed. It is not a clean-machine run.

## Known bugs / follow-ups

See `QUEUE.md`. Unverified compatibility rows are listed in
`docs/compatibility.md`: Windows 10, ARM64, true standard-user accounts,
OneDrive/UNC project directories, FAT/exFAT, third-party antivirus,
`LongPathsEnabled=0`, and volumes with 8.3 names disabled. The panic hook itself
is not exercised end to end (no supported CLI panic path), crash notes
accumulate without automatic deletion, the Podman path is untested, and the
non-Windows symlink branch is untested. A true clean-machine pass and a CI run
of the compatibility probe remain. Fuzz campaigns are short (30 s per target);
a longer soak and a Linux ASan run remain. Property tests do not model
concurrent allocator processes or real crashes. Volume-GUID project paths are
not handled.
