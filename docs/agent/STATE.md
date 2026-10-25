# Agent state (overwrite each cycle)

## Current focus

Phase 3-5 launch content is underway. The sourced landscape/comparisons page and
the reproducible Docker Compose head-to-head are complete and have had a skeptic
review. Next are packaging, trust, and docs-site work (release dry-run,
cargo-deny/SBOM, signing plan, winget/Scoop drafts, E2), then launch-kit drafts.
Phase 1 still needs a true clean-machine pass and a CI execution of the
compatibility probe. Do not write `docs/agent/LAUNCH_READINESS.md` until those
and the human-only launch steps are resolved.

## Last cycle summary (2026-10-26, cycle 16)

- **Skeptic review**: re-read the cycle-15 benchmark claims and corrected three
  overclaims. The 2.67 s API build is a **warm-cache rebuild**, not a cold build:
  removing the image does not clear Docker's build-layer cache (the earlier full
  build was 26.53 s). Compose ran before Burrow with Docker Desktop's VM still
  running, so measurement order is a possible bias. Burrow's `initdb` uses
  `-A trust --no-locale` while the Compose image uses its default locale and
  password auth.
- **Date clarity**: the run timestamp is `2026-10-25T22:34:13Z`, which is
  2026-10-26 locally (`en-NZ`). Prose now says **2026-10-25 UTC**.
- **No numbers changed**: the raw JSON is unchanged; the corrections are
  disclosure and interpretation. No re-measurement was needed.
- **No process/temp work**: Docker Desktop remains stopped from the cycle-15
  cleanup; this was a docs/script-only cycle.

## Benchmark state (2026-10-25 UTC)

- Burrow cold application data **10.7 s**, warm **1.87 / 1.88 / 1.84 s** to
  HTTP 200; daemon working set **10.9-11.0 MB**; whole tree
  **171.5-172.2 MB**; services **160.5-161.3 MB**.
- Docker Compose cold **4.11 s**, warm **3.20 / 1.92 / 2.08 s** to HTTP 200;
  containers **37.2-62.1 MB combined**; PostgreSQL pull **21.57 s**; API rebuild
  **2.67 s** (warm cache). PostgreSQL digest
  `sha256:f4610795587492cd4fb887c2dc3de301b813278bb6f616992061141b5f15ac36`;
  API image 159,806,724 bytes.
- Compose initialized empty application data faster; Burrow was slightly faster
  and more consistent warm. Container memory is not like-for-like because it
  excludes Docker Desktop's VM and daemon.
- Caveats: one Windows 11 Home machine and a small sample; Compose measured
  first; DB init flags differ; API build warm-cache; only Docker Compose was
  run; no Podman; no Linux/macOS host. Raw data:
  `docs/benchmarks/2026-10-25-stacks.json`; method and caveats:
  `docs/benchmarks.md`.

## Check suite status (cycle 16 full run)

- `cargo fmt --all --check`: **green**.
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**.
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  211 tests / 23 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 11.45 s** (no Rust changes;
  recompiled despite cached artifacts).
- `scripts/benchmark-stacks.ps1` parses cleanly; all Markdown links,
  `[S1]`-`[S16]` anchors, and the raw JSON validate.
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
  on `PATH` (`clang_rt.asan-dynamic-x86_64.dll`).
- Docker Desktop 24.0.7 / Compose v2.23.3-desktop.2 were used for the
  head-to-head on WSL 2 kernel 6.18.54.1-microsoft-standard-WSL2 (16 CPUs,
  7.469 GiB). Docker Desktop is stopped; Podman is absent.
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

The comparison page is documentation-based except for Docker Compose: only
Compose was run, on one Windows 11 machine with a small sample. Compose
container memory excludes Docker Desktop's VM and daemon. A cold API image build
was not measured, and measurement order is not randomized. Competitor docs can
change; the source ledger must be updated with the claim.
