# Agent state (overwrite each cycle)

## Current focus

Phase 0 is complete: A2 adversarial hardening, A3 fuzzing, A4 property tests,
and A6 `doctor --report` are done. The next work is Phase 1: the compatibility
matrix (A5), a clean-machine/README pass, time-to-first-success measurement,
and an error-message pass.

## Last cycle summary (2026-10-26, cycle 11)

- **A6 diagnostic bundles**: `burrow doctor --report` writes a local zip
  (default `.burrow/reports/`, `--report-dir <DIR>` overrides and implies
  `--report`) and prints its path. Entries: `README.txt`, `report.json`,
  `environment.json`, redacted `config.redacted.json` and `state.redacted.json`
  when present, a daemon-log tail, and up to ten crash notes for the project.
  Service logs are excluded because Burrow cannot vouch for application output;
  `environment.json` lists their names and sizes.
- **Crash notes**: a panic hook writes structured JSON under
  `%LOCALAPPDATA%\burrow\crashes` (temp fallback) before chaining to the
  previous hook, atomically through a temporary file. Notes record version,
  RFC 3339 and Unix-ms timestamps, PID, thread, OS/arch, project directory,
  message, location, and an optional backtrace.
- **Redaction**: known secrets, state token, sensitive env values, URL
  passwords, Bearer tokens, and common password/token/secret assignments become
  `<redacted>`; project/home/`%LOCALAPPDATA%` paths become placeholders in raw
  and JSON-escaped form. All config `env` values and command arguments are
  redacted.
- **Two redaction bugs found by manual release-binary inspection and fixed**:
  JSON-escaped paths were not matched, and an unquoted assignment scan consumed
  the closing JSON quote, producing invalid JSON. Regression tests now parse
  every JSON entry.
- **Evidence**: 202 tests / 23 suites green; manual bundle 2,494 bytes with all
  JSON entries valid and no raw or escaped secret/path leaks.

## Check suite status (cycle 11 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  202 tests / 23 suites** (15 new A6 tests).
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 53.33 s**; a later rebuild
  after the redaction fix took 54.56 s.
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

See `QUEUE.md`. Open verification gaps: the panic hook itself is not exercised
end to end (no supported CLI panic path), crash notes accumulate without
automatic deletion, the untested Podman path, and the non-Windows symlink
branch. Phase 1 still needs a true clean-machine pass; cycle 6 used the dev
machine with a warm package cache. The fuzz campaigns are short (30 s per
target); a longer soak and a Linux ASan run remain. Property tests do not model
concurrent allocator processes or real crashes.
