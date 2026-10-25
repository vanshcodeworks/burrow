# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite and A3 fuzzing are complete. Done: orphan-kill (A2.1),
interrupted restore/init/seed (A2.2), corrupt state/ports/secrets (A2.3), port
collisions (A2.4), secrets-scan (A2.5), listener inspection (A2.6), snapshot
entry hazards (A2.7), interrupted-download resume (A2.8), and five cargo-fuzz
targets (A3).

Next: A4 property tests (port allocator stability/collision-freedom, atomic
directory operations, template resolution), A6 `doctor --report`, then the
Phase 1 compatibility matrix and clean-machine pass.

## Last cycle summary (2026-10-26, cycle 9)

- **A3 fuzzing**: added the excluded `fuzz/` crate with targets for
  `burrow_config::parse`, `render_template`, `Registry::add_toml`,
  `extract::validate_entry_path`, and `RuntimeState::parse`. The three parser
  entry points that were private are now public with docs. `burrow-cli` gained
  a small lib target so the state parser is reachable without spawning the CLI;
  `main.rs` imports that module instead of compiling `state.rs` twice.
- **Evidence**: five 30 s campaigns on Windows/MSVC with AddressSanitizer,
  **1,350,504 executions**, no crash, timeout, or artifact. Committed seeds
  live in `fuzz/seeds/`; run instructions and the ASan DLL prerequisite are in
  `fuzz/README.md`.
- **Not exercised**: a long soak (the runs are 30 s each), Linux/macOS ASan,
  and the full archive readers (the path validator is fuzzed directly; tar/zip
  container parsing is not).

## Check suite status (cycle 9 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  179 tests / 20 suites** (new `RuntimeState::parse` unit test).
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green**, 53.45 s.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.
- Fuzz targets are excluded from the workspace; they are run with
  `cargo +nightly fuzz` as documented in `fuzz/README.md`.

## Environment notes

- Windows 11 Home 10.0.26200 on an AMD Ryzen 7 4800H, 15.4 GB RAM.
- Rust 1.94.1 stable via `C:\Users\Asus\.cargo\bin`; workspace MSRV 1.85.
- Nightly `1.101.0-nightly (12433c307 2026-10-24)` and `cargo-fuzz 0.13.2`
  installed locally for A3.
- Windows ASan needs
  `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64`
  on `PATH` (`clang_rt.asan_dynamic-x86_64.dll`).
- Docker Desktop 24.0.7 running; Podman absent.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. A4/A6, the untested Podman path, and the non-Windows symlink
branch are the open verification gaps. Phase 1 still needs a true clean-machine
pass; cycle 6 used the dev machine with a warm package cache. The fuzz campaigns
are short (30 s per target); a longer soak and a Linux ASan run remain.
