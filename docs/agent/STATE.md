# Agent state (overwrite each cycle)

## Current focus

Packaging and trust work is complete for this cycle: the local release dry-run
builds a zip plus a CycloneDX 1.5 SBOM and verifies both, cargo-deny is enforced
locally and configured for CI/release, and the signing plan is documented. The
docs site (E2) was assessed and deferred. What remains is human-only
(publish/sign/submit/capture) or blocked on this machine (clean-machine pass, a
remote CI run, Windows 10/ARM64 and other unverified rows). Do not write
`docs/agent/LAUNCH_READINESS.md` until those are resolved.

## Last cycle summary (2026-10-27, cycle 18)

- **Release dry-run and SBOM**: `scripts/release-dry-run.ps1` now builds,
  stages, zips, generates `burrow-<tag>-windows-x86_64.cdx.json` with
  `cargo-cyclonedx 0.5.9`, writes `SHA256SUMS.txt` for both files, extracts the
  zip, and runs the packaged binary. Local sample: 4,734,097-byte zip,
  SHA-256 `257f51589cf957a691add88018177fedcb48ff49cee146fed3920a229b50fb61`;
  SBOM 273,407 bytes with 232 components. The SBOM serial/timestamp change each
  run, so neither hash is a release hash. The release workflow now installs
  cargo-cyclonedx and attaches the SBOM.
- **Dependency policy**: `deny.toml` limits the graph to
  `x86_64-pc-windows-msvc`, allows permissive licenses and crates.io only, and
  documents one `paste` ignore. `cargo deny check --hide-inclusion-graph`
  returned `advisories ok, bans ok, licenses ok, sources ok`. CI and the release
  workflow install and run cargo-deny.
- **Docs and claims**: added the local dry-run/SBOM sections to
  `packaging/README.md`, SBOM steps to the launch checklist, cargo-deny to
  `CONTRIBUTING.md`, and a release/SBOM note to the README and architecture
  page. Qualified `docs/security.md`: CI audits the Rust dependency graph, but
  there is still no runtime sandbox, image scanning, or CVE tracking for
  downloaded service binaries.
- **Reliability fixes found by the gate**: a service log file is now created
  eagerly (best-effort) so a failed service's diagnostic path exists before its
  first output line; the health-check test waits for `[stdout]` and uses a fast
  `ping` fixture; the fixed-port test now takes fresh ephemeral ports and
  retries. The old hard-coded 45123-45130 range is inside this machine's
  excluded TCP range 45064-45163 (Hyper-V/WSL), so every bind failed.
- **E2 docs site**: deferred. mdBook is absent and would add a toolchain/CI
  dependency, a `SUMMARY.md`, and link-rewrite verification; the Markdown docs
  already render on the repository host and links are checked locally.

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

## Check suite status (cycle 18 full run)

- `cargo fmt --all --check`: **green**.
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**.
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  212 tests / 23 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo deny check --hide-inclusion-graph`: **green**
  (`advisories ok, bans ok, licenses ok, sources ok`; duplicate-version
  warnings only).
- `cargo build --workspace --release`: **green in 0.41 s** (cached artifacts).
- `scripts/release-dry-run.ps1`: **green** with the local zip/SBOM/hashes above.
- `scripts/release-dry-run.ps1` parses under PowerShell 5.1 and is pure ASCII;
  `git diff --check` is clean.
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
- cargo-audit 0.22.2, cargo-deny 0.20.2, and cargo-cyclonedx 0.5.9 are installed
  locally.
- Windows TCP exclusions observed here include 45064-45163; tests must not
  hard-code ports in an excluded range.
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

Release assets are still drafts in the sense that nothing is published: no
release, signature, winget/Scoop submission, or screenshots exist. The local
dry-run hash is not a release hash. The SBOM is an unsigned CycloneDX inventory,
not a vulnerability scan or provenance attestation.
