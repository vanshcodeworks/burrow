# Agent queue (priority order; maintain aggressively)

## Phase 1 - launch readiness

- [x] **Compatibility matrix (A5)**: probe and verified/unverified rows; see
      `docs/compatibility.md` and the A5 entry below.
- [x] **Time to first success**: measured from a fresh checkout with empty
      Cargo, npm, and Burrow caches. 162.62 s install + 3.19 s npm + 79.43 s
      first start = **245.24 s**; warm restart **1.87 s**; `doctor` 11 ok.
      Recorded in `docs/benchmarks.md`.
- [x] **Cold-cache README pass**: no new documentation defects; `api.localhost`
      did not resolve here and the documented direct-URL fallback worked.
      `docs/agent/FRICTION.md` cycle 13.
- [x] **Error-message pass**: only a missing `burrow.toml` suggests `init`;
      unknown CLI services list valid names; a missing snapshot points to
      `burrow snapshots`; bad command, health timeout, and port conflict were
      probed and are actionable with clean teardown.
- [x] **Diagnostic test CI fixture**: the bundle test now uses the actual
      temporary project directory from `tempfile` instead of a hardcoded path
      under the local user profile, so redaction works on any CI runner.
      Fixed in cycle 20.
- [ ] **True clean-machine pass**: a VM, Windows Sandbox, or clean CI runner
      with no pre-installed toolchain, including a standard-user account and
      Windows 10. Blocked: none available on this machine.
- [ ] **Run the compatibility probe on CI**: the step is configured in
      `.github/workflows/ci.yml`, but there is no remote/CI run here.
- [ ] **Unverified compatibility rows**: ARM64, OneDrive/UNC, FAT/exFAT,
      third-party antivirus, `LongPathsEnabled=0`, 8.3-disabled volumes.

## Phase 3-5 - launch content

- [x] **Sourced landscape/comparisons**: `docs/comparisons/README.md` compares
      Burrow with Docker Compose, Podman Compose, Process Compose, Overmind,
      VS Code Dev Containers, WinSW, and NSSM. Every competitor claim maps to an
      official source with a 2026-10-26 retrieval date in `SOURCES.md`; each
      section states where the alternative is better. Linked from the README and
      FAQ. Docker Compose was measured separately; the other tools were not run.
- [x] **Reproducible benchmark suite vs Docker Compose where available**
      (tracked as E3 below): `scripts/benchmark-stacks.ps1` measured both stacks
      on 2026-10-25 UTC with the same example and `/api/todos` endpoint, recorded
      machine specs and raw data in `docs/benchmarks/2026-10-25-stacks.json`,
      and reports image pull/build separately. Podman remains unavailable and is
      documented as unverified.
- [ ] **Packaging, trust, and docs site**: release dry-run zip/checksums/SBOM
      (D6) and cargo-deny config (D1) are done; the signing plan is documented
      in `packaging/README.md`; winget/Scoop manifests still need real hashes
      after a human release, and the docs site (E2) is deferred. A human
      publishes.
- [x] **Launch kit drafts**: `docs/launch/README.md`, `announcement.md`,
      `checklist.md`, and `screenshots.md`, plus the updated `docs/demo.md`.
      Announcement copy uses only measured or documented claims and now points at
      the public repository URL. The screenshot plan is complete, but the actual
      capture is a human action tracked in `NEEDS_HUMAN.md`. Drafts only; a human
      posts.

## Tier A - verification and hardening

- [x] **A1 Claim audit**: full map in `CLAIMS.md`; 7 gaps found and queued
      (two closed since: secrets scan A2.5, junction test A2.7 partial).
- [ ] **A2 Adversarial suite**:
  - [x] A2.1 (partial) CLI-level: force-killed daemon (taskkill /F, no /T)
        → service port refuses, project recovers on the same port
        (tests/adversarial.rs, 3/3 runs). Remaining kill vectors: /T tree
        kill of the console, logoff scenarios.
  - [x] A2.2 Interrupted data operations: a restore journal
        (`.burrow/data/.burrow-restore-journal.json`) rolls an uncommitted
        restore back on the next `up`/`snapshot`/`restore`/`reset`, or finishes
        cleanup for a committed one; staging without a journal is removed and
        an orphan backup is refused. Partial init directories are moved to
        `.burrow/data/.burrow-init-failed-*` before retry, and PostgreSQL's
        seed runs with `--single-transaction`. Tests: `burrow-data` unit tests
        (rollback, committed cleanup, staging/orphan backup, quarantine) and
        `tests/data_safety.rs` (daemon startup rollback, snapshot rollback).
        A real kill during `initdb`/seed was simulated at the data-directory
        level, not exercised against a live PostgreSQL process.
  - [x] A2.3 Corrupt/truncated `state.json` fails loudly and is never deleted;
        truncated `ports.json` fails startup with an actionable error; corrupt
        `secrets` fails `up`/`env` without overwrite, `doctor --json` reports a
        failure with a remedy while the running project and `state.json`
        identity fields stay intact, and deleting the file regenerates it
        (`tests/data_safety.rs`).
  - [x] A2.4 Port collisions across concurrent projects: a fixed-port
        collision fails the second project with a remedy, not a crash
        (`tests/cli.rs`, `6b815eb`).
  - [x] A2.5 Secrets scanning: tests/secrets.rs scans daemon.log, all
        service logs, state.json, status (human+json), logs, project env,
        and a snapshot; positive control on `burrow env <service>`.
  - [x] A2.6 Non-loopback listener detection: `burrow_supervisor::listening_sockets`
        reads IPv4/IPv6 owner-PID TCP tables and attributes child listeners to
        the supervised root via a Toolhelp process snapshot. `doctor` adds a
        `listeners` warning for non-loopback binds without `expose = true`.
        Tests: supervisor `tests/listeners.rs` (loopback/wildcard/IPv6,
        child attribution, kill cleanup); CLI `tests/listeners.rs` (wildcard
        warns, exposed wildcard accepted, loopback accepted). ADR 0006.
  - [x] A2.7 Snapshot entry hazards: junction rejection tested with a real
        `mklink /J`; hard links copied as independent files; NTFS alternate
        data streams refused with the file and stream named; reserved Windows
        device names rejected for snapshot names and for project/service names
        (`burrow-data` + `burrow-config` tests). Remaining: the non-Windows
        symlink branch can only be exercised on Linux/macOS.
  - [x] A2.8 Interrupted-then-resumed package download: a local range server
        proves the `.part` file offset, the `Range` request, the completed
        hash-checked package, and removal of the partial file
        (`cache.rs::an_interrupted_download_resumes_from_the_part_file`).
- [x] **A3 Fuzzing**: the excluded `fuzz/` crate has five cargo-fuzz targets:
      config parser, template renderer, registry parser, extractor path
      validation, and `state.json` parser. Five 30 s campaigns on
      Windows/MSVC with AddressSanitizer ran **1,350,504 executions** with no
      crash, timeout, or artifact. Seeds are committed in `fuzz/seeds/`;
      prerequisites (nightly, `cargo-fuzz`, the Windows ASan DLL) and the
      target table are in `fuzz/README.md`.
- [x] **A4 Property tests**: `burrow-net` proves automatic ports are unique,
      in range, remembered across save/load, and that reserved/pinned ports are
      never handed out; `burrow-data` round-trips generated file trees through
      snapshot/restore and asserts the manifest counts and that no staging,
      backup, or temporary directory remains; `burrow-config` covers plain-text
      identity, substitution with surrounding text, unclosed/unknown-variable
      errors, and that resolution preserves template-free values. Eight
      properties, **1,840 generated cases**, all green.
- [x] **A5 Compatibility matrix doc**: `scripts/compatibility.ps1` runs a
      package-free project through `up`/`status`/`doctor`/`down` in ASCII,
      spaces, non-ASCII, and >260-character directories. Verified on Windows 11
      x64 build 26200 under PowerShell 7.6.7 and 5.1 (4/4 each). The probe found
      a real long-path failure (`CreateProcessW` os error 267); fixed by
      converting the process working directory and application path to an 8.3
      short name (ADR 0010). Windows 10, ARM64, true standard-user,
      OneDrive/UNC, FAT/exFAT, `LongPathsEnabled=0`, and 8.3-disabled volumes
      remain unverified and are listed as such in `docs/compatibility.md`. The
      CI `check` job now runs the same probe on `windows-latest` after the
      release build; that step was added but not executed here.
- [x] **A6 Crash safety**: `burrow doctor --report` writes a local redacted
      bundle (report, environment, redacted config/state, daemon-log tail, up
      to 10 crash notes for the project); the CLI panic hook writes structured
      notes under `%LOCALAPPDATA%\burrow\crashes`. Nothing is uploaded and
      service logs are excluded by design. Tests: 5 crash unit, 7 bundle and
      redaction unit, 3 CLI integration; a manual release-binary bundle was
      checked for leaks and JSON validity. ADR 0009. Remaining gap: the hook
      itself has no supported end-to-end panic path, and crash notes accumulate
      without automatic deletion.
- [x] **A-flake-1** `a_failed_health_check_stops_the_daemon_and_leaves_no_state`
      asserted < 20 s wall time and failed once under compile load. Relaxed to
      60 s; the health-check timeout itself is unchanged.

## Tier B — resources

- [ ] B1.1 Measure idle memory saved by idle_stop on the nats/caddy services;
      record in `BENCH_HISTORY.md` (B1 core design is done — ADR 0003).
- [ ] B2 Registry run profiles (`profile = "tiny"|"balanced"|"performance"`)
      with measured RSS per profile.
- [ ] B3 Startup: measure warm `burrow up` before/after any change; keep
      history (baseline exists in docs/benchmarks.md).
- [ ] B4 Dedup/pkg-gc safety, CoW snapshots on ReFS/Dev Drive (probe support,
      fall back to copy).
- [ ] B5 Daemon/TUI RSS regression guard in CI.

## Tier C — experience

- [ ] C2 `burrow import compose`.
- [ ] C3 `burrow.lock` + `burrow update` diff.
- [ ] C5 Better `init` detection list.
- [ ] C6 `before_start`/`after_healthy` hooks + one-shot jobs.
- [ ] C7 `burrow db shell|dump|load|url`.
- [ ] C8 More registry services (MariaDB? SQLite tools? Mailpit two-port
      design needed) + `burrow registry lint` + scheduled hash re-verification.
- [ ] C9 Shell completions; `env --format cmd` (good-first-issue list).
- [ ] C12 `burrow share` bundles.
- [ ] C13 Done for Docker; verify podman flag parity when podman is available.

## Tier D — trust/release

- [x] D1 cargo-deny config + SBOM; document signing plan (human action).
      `deny.toml` is checked locally and in CI; `cargo-cyclonedx` generates a
      CycloneDX 1.5 SBOM for the release; the signing secrets and unsigned
      fallback are documented in `packaging/README.md`. Signing itself remains
      a human action.
- [ ] D2 Trust prompt diff on change; `burrow trust` management.
- [ ] D3 Threat model doc additions; review log.
- [ ] D5 Semver/config-schema versioning policy.
- [x] D6 Release dry-run pipeline (zip + checksums + SBOM locally).
      `scripts/release-dry-run.ps1` builds, stages, zips, generates the
      CycloneDX 1.5 SBOM, writes `SHA256SUMS.txt`, extracts, and verifies the
      packaged binary. It never tags, signs, uploads, or publishes. The hash is
      local-only: a rebuild changes the zip and the SBOM identifiers vary.

## Tier E — docs/examples

- [ ] E1 Examples with smoke tests (Next.js+Postgres, FastAPI+Postgres+Mailpit).
- [ ] E2 Docs site (mdBook) - deferred after a cycle-18 assessment. mdBook is
      not installed here; adding it means another toolchain and CI dependency,
      a `SUMMARY.md`, and re-verification of every relative link and anchor.
      The current Markdown renders on the repository host and its links are
      checked locally. Revisit after the launch docs settle.
- [x] E3 (Phase 3-5) Benchmark suite vs Docker Compose: `scripts/benchmark-stacks.ps1`
      measured both stacks on 2026-10-25 UTC; raw JSON is
      `docs/benchmarks/2026-10-25-stacks.json`. Podman remains unavailable and
      is stated as unverified rather than inferred.
- [ ] E5 CLI/TUI polish: plain mode audit, narrow terminals.

## Done (this run)

- Phase 2 complete: idle-stop/wake, watch, container fallback,
  nats+caddy registry entries, ADRs 0003-0005, full docs. All green at
  `73786d7`.
