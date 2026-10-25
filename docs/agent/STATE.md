# Agent state (overwrite each cycle)

## Current focus

Tier A2, A3 fuzzing, and A4 property tests are complete. Done: orphan-kill
(A2.1), interrupted restore/init/seed (A2.2), corrupt state/ports/secrets
(A2.3), port collisions (A2.4), secrets-scan (A2.5), listener inspection (A2.6),
snapshot entry hazards (A2.7), interrupted-download resume (A2.8), five
cargo-fuzz targets (A3), and eight property tests for ports, snapshots, and
templates (A4).

Next: A6 `doctor --report`, then the Phase 1 compatibility matrix and
clean-machine pass.

## Last cycle summary (2026-10-26, cycle 10)

- **A4 property tests**: added `burrow-net/tests/properties.rs`,
  `burrow-data/tests/properties.rs`, and
  `burrow-config/tests/template_properties.rs`.
  - Ports: automatic assignments are unique, in 40000-49999, stable on
    re-allocation, preserved across save/load with no `.tmp` left, and
    reserved/pinned ports are never handed out.
  - Snapshots: generated file trees round-trip through snapshot and restore;
    manifest file/byte counts match; post-snapshot files do not survive; absent
    services stay absent; no staging, backup, or temporary directory remains.
  - Templates: plain text is unchanged, substitutions keep surrounding text,
    unclosed and unknown variables error, and resolution preserves
    template-free values.
- **Evidence**: eight properties, **1,840 generated cases**, all green. The
  first snapshot property run found a generator bug (a generated file was also
  a parent directory); the generator now uses a flat namespace plus one fixed
  nested directory, and the final run passes 48 cases.
- **Not exercised**: concurrent allocator processes, real crashes during the
  property runs (crash recovery has separate simulated tests), and non-Windows
  filesystems.

## Check suite status (cycle 10 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  187 tests / 23 suites** (8 new property tests).
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green, cached in 0.41 s** (only
  dev-dependencies changed; the previous full build was 53.45 s).
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.
- Fuzz targets are excluded from the workspace; see `fuzz/README.md`.

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

See `QUEUE.md`. A6, the untested Podman path, and the non-Windows symlink branch
are the open verification gaps. Phase 1 still needs a true clean-machine pass;
cycle 6 used the dev machine with a warm package cache. The fuzz campaigns are
short (30 s per target); a longer soak and a Linux ASan run remain. Property
tests do not model concurrent allocator processes or real crashes.
