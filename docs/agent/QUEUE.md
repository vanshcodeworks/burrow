# Agent queue (priority order; maintain aggressively)

## Tier A — verification and hardening

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
  - [x] A2.7 (partial) junction rejection in snapshots now tested (real
        mklink /J; error names the path; nothing left behind). Remaining:
        hardlink + ADS in burrow-data, non-Windows symlink path.
  - [x] A2.8 Interrupted-then-resumed package download: a local range server
        proves the `.part` file offset, the `Range` request, the completed
        hash-checked package, and removal of the partial file
        (`cache.rs::an_interrupted_download_resumes_from_the_part_file`).
- [ ] **A3 Fuzzing**: cargo-fuzz targets for config parser, template engine,
      registry parser, extractor path logic, state.json parser. Needs a fuzz
      crate (does not ship in the workspace build).
- [ ] **A4 Property tests**: port allocator stability/collision-freedom,
      atomic dir ops, template resolution (extend existing proptest suite).
- [ ] **A5 Compatibility matrix doc**: Win10/11, non-ASCII paths, long paths,
      spaces, OneDrive dirs, non-admin. Manual test script + docs page.
- [ ] **A6 Crash safety**: `burrow doctor --report` bundle (redacted);
      structured local crash notes.
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

- [ ] D1 cargo-deny config + SBOM; document signing plan (human action).
- [ ] D2 Trust prompt diff on change; `burrow trust` management.
- [ ] D3 Threat model doc additions; review log.
- [ ] D5 Semver/config-schema versioning policy.
- [ ] D6 Release dry-run pipeline (zip + checksums + SBOM locally).

## Tier E — docs/examples

- [ ] E1 Examples with smoke tests (Next.js+Postgres, FastAPI+Postgres+Mailpit).
- [ ] E2 Docs site (mdBook) — defer until content settles.
- [ ] E3 Benchmark suite vs docker compose where available.
- [ ] E5 CLI/TUI polish: plain mode audit, narrow terminals.

## Done (this run)

- Phase 2 complete: idle-stop/wake, watch, container fallback,
  nats+caddy registry entries, ADRs 0003-0005, full docs. All green at
  `73786d7`.
