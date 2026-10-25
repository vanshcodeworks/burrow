# Agent state (overwrite each cycle)

## Current focus

Phase 3-5 launch content is underway. The sourced landscape/comparisons page is
complete and linked from the README and FAQ. Next are a reproducible benchmark
run against Docker Compose where the engine is available, then packaging/trust/
docs-site work, then launch-kit drafts. Phase 1 still needs a true clean-machine
pass and a CI execution of the compatibility probe. Do not write
`docs/agent/LAUNCH_READINESS.md` until those and the human-only launch steps are
resolved.

## Last cycle summary (2026-10-26, cycle 14)

- **Sourced landscape**: added `docs/comparisons/README.md`, a fair comparison
  with Docker Desktop + Compose, Podman + podman-compose, Process Compose,
  Overmind, VS Code Dev Containers, and the adjacent Windows service wrappers
  WinSW and NSSM. Every competitor section has a "Where it is a better fit than
  Burrow" subsection, and a final section lists where Burrow is the wrong tool.
- **Source ledger**: added `docs/comparisons/SOURCES.md` with 16 official
  sources (S1-S16), each with a URL, the retrieval date `2026-10-26`, and the
  exact claims it supports. The comparison uses `[S1]`-`[S16]` reference links;
  the ledger states the "not documented is not absent" rule. S15 covers Docker's
  overview (isolation, daemon, images), S16 covers Podman's open-source project
  site.
- **No competitor benchmarked**: the page reports only Burrow's existing
  measurements from `docs/benchmarks.md`. It explains that the recorded run had
  no Docker daemon, that Docker Desktop's WSL 2 backend uses dynamic memory
  allocation, and that a single Docker RSS number would be misleading even if
  measured.
- **README and FAQ**: linked the comparison and source ledger from the README
  "Fair comparison" section and the documentation list. Fixed a real
  documentation bug: the FAQ said the fair comparison was in `benchmarks.md`;
  it now points to `comparisons/README.md` and `benchmarks.md`. Replaced the
  README headline's unsupported "fraction of the resources" claim with a factual
  native-services statement, since no Docker comparison was measured.
- **NEEDS_HUMAN**: removed the stale "cargo-audit local install fails" entry
  (cargo-audit 0.22.2 works) and replaced it with the real human-only actions:
  publishing/signing, clean-machine/CI coverage, Podman parity and a Docker
  head-to-head benchmark, and review before posting.

## Check suite status (cycle 14 full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  211 tests / 23 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 0.66 s** (no Rust changes;
  cached artifacts from cycle 13).
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
- Docker Desktop 24.0.7 installed; Podman absent. The recorded benchmark had no
  Docker daemon running, so no Docker number is claimed.
- cargo-audit 0.22.2 installed locally and working.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.
- The cycle-13 quick-start pass used empty Cargo, npm, and Burrow caches, but
  the OS, Rust toolchain, MSVC build tools, Node, and npm were already
  installed. It is not a clean-machine run.

## Known bugs / follow-ups

See `QUEUE.md`. Phase 1 still needs a true clean-machine VM/Sandbox run and a CI
execution of the compatibility probe; Windows 10, ARM64, true standard-user
accounts, OneDrive/UNC, FAT/exFAT, third-party antivirus, `LongPathsEnabled=0`,
and 8.3-disabled volumes are unverified in `docs/compatibility.md`. The panic
hook itself is not exercised end to end, crash notes accumulate without
automatic deletion, the Podman path is untested, and the non-Windows symlink
branch is untested. Fuzz campaigns are short (30 s per target); a longer soak and
a Linux ASan run remain. Property tests do not model concurrent allocator
processes or real crashes. Volume-GUID project paths are not handled.

The comparison page is a documentation comparison, not a test of competitor
behavior: no competitor was run or benchmarked, and a Docker Compose
head-to-head remains E3. Competitor docs can change; the source ledger must be
updated with the claim.
