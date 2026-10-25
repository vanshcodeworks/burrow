# Agent state (overwrite each cycle)

## Current focus

Phase 3-5 launch content is underway. The sourced landscape/comparisons page and
a reproducible Docker Compose head-to-head are complete. Next are packaging,
trust, and docs-site work (release dry-run, cargo-deny/SBOM, signing plan,
winget/Scoop drafts, E2), then launch-kit drafts. Phase 1 still needs a true
clean-machine pass and a CI execution of the compatibility probe. Do not write
`docs/agent/LAUNCH_READINESS.md` until those and the human-only launch steps are
resolved.

## Last cycle summary (2026-10-26, cycle 15)

- **Reproducible head-to-head**: added `scripts/benchmark-stacks.ps1`. It
  generates a Compose fixture that mirrors `examples/node-postgres` (same
  `server.js`, schema, and seed), measures both stacks from launch until
  `/api/todos` returns HTTP 200 with the seeded row, and records machine specs.
  Image pull/build is measured separately and excluded from startup. Cleanup runs
  in `finally`, and a missing Docker daemon skips the Compose half instead of
  inventing a number. The only app change is `HOST=0.0.0.0` inside the container
  so Docker can forward the port; the native example binds loopback.
- **Measured results** (2026-10-25, raw data in
  `docs/benchmarks/2026-10-25-stacks.json`): Burrow cold application data
  **10.7 s**, warm **1.87 / 1.88 / 1.84 s**, daemon working set **10.9-11.0 MB**,
  whole tree **171.5-172.2 MB**. Compose cold **4.11 s**, warm
  **3.20 / 1.92 / 2.08 s**, containers **37.2-62.1 MB combined**. Compose image
  pull **21.57 s**, API build **2.67 s**. PostgreSQL image digest
  `sha256:f4610795587492cd4fb887c2dc3de301b813278bb6f616992061141b5f15ac36`;
  API image 159,806,724 bytes.
- **Interpretation**: Compose initialized empty application data faster; Burrow
  was slightly faster and more consistent on warm starts. Container memory is
  not like-for-like with Burrow's daemon RSS because it excludes Docker
  Desktop's VM and daemon.
- **Public docs**: `docs/benchmarks.md` has the method, results, setup times, and
  caveats; the README benchmark paragraph and fair-comparison table use the
  measured Compose numbers; the comparison page's performance and resource
  fairness notes now point to the head-to-head.
- **Agent docs**: closed E3, marked the Phase 3-5 benchmark item done, removed
  the Docker head-to-head from `NEEDS_HUMAN.md` (only Podman parity remains),
  and updated the M6/Phase 3-5 claim rows.
- **Cleanup**: removed the benchmark images (`postgres:16.15-alpine` and the
  generated API image), the three benchmark temp directories, and all benchmark
  containers/volumes; stopped Docker Desktop and its WSL distros. Confirmed no
  `burrow` processes, port 3000 free, and no benchmark temp directories left.

## Check suite status (cycle 15 full run)

- `cargo fmt --all --check`: **green**.
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**.
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  211 tests / 23 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green in 0.62 s** (no Rust changes;
  cached artifacts).
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
  7.469 GiB). Docker Desktop is stopped now; Podman is absent.
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
Compose was run, and only on one Windows 11 machine with a small sample. Compose
container memory excludes Docker Desktop's VM and daemon. Competitor docs can
change; the source ledger must be updated with the claim.
