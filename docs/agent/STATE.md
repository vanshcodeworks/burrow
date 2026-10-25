# Agent state (overwrite each cycle)

## Current focus

Phase 0 and A5 are complete: adversarial hardening (A2), fuzzing (A3),
property tests (A4), the compatibility matrix (A5), and diagnostic bundles (A6)
are done. Phase 1 continues with a clean-machine/README pass, a
time-to-first-success measurement, and an error-message pass.

## Last cycle summary (2026-10-26, cycle 12)

- **A5 compatibility matrix**: added `scripts/compatibility.ps1`, a
  package-free probe that runs a real project through `up`, `status --json`,
  HTTP 200, `doctor --json`, and `down` in ASCII, spaces, non-ASCII, and
  >260-character directories. Added `docs/compatibility.md` with verified and
  unverified rows, the exact machine, and the probe's limits.
- **The probe found a real bug**: a 279-character project path failed at daemon
  spawn with `could not start the Burrow daemon (The directory name is invalid.
  (os error 267))`. `CreateProcessW` rejects `lpCurrentDirectory` at or beyond
  `MAX_PATH`, rejects the `\\?\` prefix there, and `cmd.exe` treats a verbatim
  directory as UNC and falls back to `C:\Windows`.
- **Fix**: `burrow_supervisor::process_working_directory` strips verbatim
  prefixes from short paths and converts long paths to their 8.3 short name
  (resolved through `GetShortPathNameW` with a verbatim input), with an explicit
  error when the short form is unavailable or still too long. A long
  `lpApplicationName` is shortened the same way. Applied to supervised service
  creation, `cmd` health checks, the detached daemon, `burrow exec`, the
  browser launcher used by `burrow open`, and package init/seed steps. ADR
  0010.
- **Evidence**: 4/4 path shapes passed under PowerShell 7.6.7 and under Windows
  PowerShell 5.1.26100.9711; the long row used a 279-character path. Each run
  reported `doctor 9 ok / 0 warnings` and removed `.burrow/state.json` on
  `down`.
- **PowerShell 5.1 quirk**: a BOM-less script is read as ANSI, so literal
  non-ASCII test paths were mangled into smart quotes and the script failed to
  parse. The probe now builds the non-ASCII path from code points and is pure
  ASCII.
- **CI**: the `check` job now runs the probe on `windows-latest` after the
  release build. No CI run was available here, so this is configuration, not
  evidence.

## Check suite status (cycle 12 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  206 tests / 23 suites** (4 new supervisor unit tests).
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 54.47 s**.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.
- Fuzz targets are excluded from the workspace; see `fuzz/README.md`.

## Environment notes

- Windows 11 Home Single Language 10.0.26200 build 26200 on an AMD Ryzen 7
  4800H, 15.4 GB RAM; `LongPathsEnabled=1`.
- Probe sessions: PowerShell 7.6.7 and Windows PowerShell 5.1.26100.9711,
  non-elevated (medium integrity), locale `en-NZ`.
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

See `QUEUE.md`. Unverified compatibility rows are listed in
`docs/compatibility.md`: Windows 10, ARM64, true standard-user accounts,
OneDrive/UNC project directories, FAT/exFAT, third-party antivirus,
`LongPathsEnabled=0`, and volumes with 8.3 names disabled. The panic hook itself
is not exercised end to end (no supported CLI panic path), crash notes
accumulate without automatic deletion, the Podman path is untested, and the
non-Windows symlink branch is untested. Phase 1 still needs a true clean-machine
pass; cycle 6 used the dev machine with a warm package cache. The fuzz campaigns
are short (30 s per target); a longer soak and a Linux ASan run remain. Property
tests do not model concurrent allocator processes or real crashes.
