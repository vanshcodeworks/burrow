# Agent queue (priority order; maintain aggressively)

## Tier A — verification and hardening

- [ ] **A1 Claim audit** (in progress): map every roadmap/README claim to a
      test or measurement in `CLAIMS.md`. Flag any claim with no backing.
- [ ] **A2 Adversarial suite**:
  - [ ] A2.1 Kill the daemon parent by every means (taskkill /F on daemon,
        /T tree kill, taskkill the console, hard process exit) and assert zero
        orphan service processes after each. Extend
        `crates/burrow-supervisor/tests/job_lifecycle.rs` and add a CLI-level
        test that kills a detached daemon and proves ports/processes are gone.
  - [ ] A2.2 Interrupted snapshot/restore/seed/initdb: kill the daemon (or the
        burrow process) mid-operation; assert `.burrow` remains recoverable
        (no half-snapshots listed, no half-restored service dirs, markers
        consistent).
  - [ ] A2.3 Stale state after "power loss": corrupt/truncate state.json,
        ports.json, secrets; assert doctor/down/up recover cleanly and never
        delete live state.
  - [ ] A2.4 Port collisions across concurrent projects: two projects with the
        same fixed port; assert the second fails with a remedy, not a crash.
  - [ ] A2.5 Secrets scanning: assert no secret bytes appear in daemon.log,
        service logs, state.json, snapshots, or `burrow status` output for a
        project with a generated password.
  - [ ] A2.6 Non-loopback listener detection: testbin that binds 0.0.0.0;
        assert doctor warns (if implemented) or add the check.
  - [ ] A2.7 Extractor/data attacks: zip-slip, symlink, hardlink, junction,
        reserved names (CON, NUL, trailing dot/space), ADS (`file:stream`) —
        many exist in burrow-pkg tests; audit coverage and fill gaps,
        especially junction/ADS paths in burrow-data (reparse point handling).
- [ ] **A3 Fuzzing**: cargo-fuzz targets for config parser, template engine,
      registry parser, extractor path logic, state.json parser. Needs a fuzz
      crate (does not ship in the workspace build).
- [ ] **A4 Property tests**: port allocator stability/collision-freedom,
      atomic dir ops, template resolution (extend existing proptest suite).
- [ ] **A5 Compatibility matrix doc**: Win10/11, non-ASCII paths, long paths,
      spaces, OneDrive dirs, non-admin. Manual test script + docs page.
- [ ] **A6 Crash safety**: `burrow doctor --report` bundle (redacted);
      structured local crash notes.
- [ ] **A-flake-1** `a_failed_health_check_stops_the_daemon_and_leaves_no_state`
      asserts < 20 s wall time; failed once under compile load. Relax or make
      load-independent.

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
