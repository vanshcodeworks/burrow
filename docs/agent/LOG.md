# Agent log (append one entry per cycle)

## 2026-10-25 — cycle 1: phase 2 + protocol setup

**Task**: Complete the roadmap's Phase 2 (idle-stop/wake, file watching,
more registry services, container fallback), then stand up the agent
protocol files.

**Outcome**: All four Phase-2 items implemented, tested, documented, and
committed on `agent/work` (`aec71e2..73786d7`). Orphaned daemon from a
cancelled verification command was found and killed; verified zero
leftover processes afterwards (kill-on-close held under `taskkill /F`).

**Commands and real results**:

- `cargo fmt --all --check` — clean after formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean after
  fixing one clippy lint (`manual_slice_size_calculation`) and one test
  fixture missing the new `runtime` field.
- `cargo test --workspace --all-targets -- --test-threads=1` —
  **151 passed, 0 failed** across 14 suites (includes 4 new phase-2
  integration tests). One earlier run failed
  `a_failed_health_check_stops_the_daemon_and_leaves_no_state` on its
  20-second wall-clock assert under compile load; passed on rerun and in
  isolation → tracked as QUEUE A-flake-1.
- Live verifications: nats + caddy `pkg add`/`pkg verify`/`burrow up`
  against real downloads (both healthy, correct URLs and runtime
  labels); container fallback against Docker 24.0.7 (whoami container:
  healthy, labelled, HTTP 200, no leftover container after down).
- `cargo audit` — NOT run: `cargo install cargo-audit` failed with a
  build error (os error 3); retry queued. CI covers it on windows-latest.

**Not verified**: cargo-audit locally; podman path of the container
fallback (no podman installed); ARM64; long-run soak of idle-stop under
hours of traffic.

**Next**: Tier A1 claim audit into `CLAIMS.md`, then A2 adversarial suite.

## 2026-10-25 — cycles 2-3: security upgrade + adversarial suite round 1

**Task**: cargo audit; fix findings; Tier A2 adversarial tests (orphan
kill, secrets scan, junction rejection) using sub agents.

**Outcome**: audit green; three adversarial tests landed; 154 workspace
tests pass; all committed on `agent/work`.

**Commands and real results**:

- `cargo install cargo-audit --locked` (retry with CARGO_TARGET_DIR=/tmp)
  — succeeded; v0.22.2.
- `cargo audit` — found 3 vulnerabilities (RUSTSEC-2026-0367/0368/0369,
  zip 5.1.1, `ZipArchive::extract`-based; Burrow never calls extract but
  the gate is red regardless). Upgraded zip to 9.1.0; extractor adapted
  to zip 9's `entry.name() -> Result<Cow<str>>` (invalid UTF-8 names now
  rejected at read). `cargo audit` re-run: green (3 allowed warnings:
  paste unmaintained, lru unsound — transitive, not ours).
- `cargo test -p burrow-pkg` after upgrade: 17/17 pass.
- Sub agent 1 (junction test): added
  `a_junction_inside_a_data_directory_is_rejected` to
  `crates/burrow-data/src/snapshot.rs`; `cargo test -p burrow-data --lib`
  → 8/8 pass; clippy/fmt clean. Real `mklink /J` junction; rejection
  verified with the exact error wording.
- Sub agent 2 (orphan test): created
  `crates/burrow-cli/tests/adversarial.rs`
  `a_force_killed_daemon_leaves_no_orphans_and_the_project_recovers`;
  3/3 runs passed (10.7–16.6 s); port-refusal proof, stale-state
  recovery, stable-port reuse; tasklist diagnostic on failure; no
  leftover burrow.exe after runs.
- Sub agent 3 (secrets test): spawn failed twice (user concurrency
  limit; then DNS error) — wrote it inline instead:
  `crates/burrow-cli/tests/secrets.rs` passes first run (3.45 s).
- Full suite `cargo test --workspace --all-targets -- --test-threads=1`:
  **154 passed / 0 failed** across 15 suites. phase2 suite took 272 s
  (cold docker image pull); others normal.
- Commits: `fix(deps)` zip upgrade, `test(data)` junction,
  `test(cli)` orphan recovery, `test(cli)` secrets scan.

**Not verified**: podman path; non-Windows symlink path of the snapshot
rejection (test is windows-gated like the repo); the SKIPPED-ENV branch
of the junction test (junctions never need admin here).

**Next**: A2.2 interrupted data operations; A2.3 stale-state corruption;
A2.4 concurrent-project port collisions; A2.6 non-loopback listener
detection; A2.8 download-resume test; A-flake-1.

## 2026-10-26 - cycle 4: stale-state, snapshot debris, and download-resume tests

**Task**: Close A2.2 (partial), A2.3 (partial), A2.8, and A-flake-1; re-run
the full gate on the updated tree.

**Outcome**: Four verification gaps closed or narrowed; full gate green.

**Commands and real results**:

- `cargo test -p burrow-pkg --lib an_interrupted_download_resumes_from_the_part_file`
  - 1/1 pass. The first draft of the local HTTP server used literal LF-only
  string breaks instead of `\r\n`; corrected before the run, so the request
  terminator and response framing are real HTTP/1.1.
- `cargo test -p burrow-cli --test data_safety -- --test-threads=1`
  - 3/3 pass (corrupt state, truncated ports, half-written snapshot).
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **159 passed, 0 failed across 17 suites**.
- A-flake-1: the 20 s wall-clock assertion in
  `a_failed_health_check_stops_the_daemon_and_leaves_no_state` is now 60 s;
  the health-check timeout path itself is unchanged.

**Not verified**: interrupted restore/seed/initdb (A2.2 remainder), corrupt
secrets and doctor live-state preservation (A2.3 remainder), non-loopback
listener detection (A2.6), hardlink/ADS snapshot entries (A2.7 remainder).

**Next**: A2.6 and the A2.2/A2.3 remainders, then A3 fuzzing.

## 2026-10-26 - cycle 5: real listener inspection for `expose`

**Task**: Close A2.6 by inspecting the TCP sockets actually owned by supervised
process trees and warning in `doctor` when a service is reachable without
`expose = true`; re-run the full gate.

**Outcome**: A2.6 implemented, documented in ADR 0006, and covered by five new
integration tests. Full gate green.

**What changed**:

- `burrow-supervisor`: `Listener`, `listening_sockets(roots)`, `process_tree`,
  IPv4/IPv6 `GetExtendedTcpTable` collection. Child listeners are attributed to
  the supervised root PID. `Listener::is_loopback` treats any non-loopback
  address (not just the wildcard) as reachable.
- `burrow-cli`: `doctor` gains the `listeners` check. It warns for non-loopback
  listeners whose service lacks `expose`, names the service/PID/address/port,
  and never fails startup. If the OS query fails it reports a warning, not a
  clean project.
- `burrow-testbin`: new `bind <ADDRESS>` mode using `$PORT`.
- Docs: ADR 0006, `security.md`, `cli.md`, `troubleshooting.md`, testbin README.

**Commands and real results**:

- `cargo test -p burrow-supervisor --test listeners -- --test-threads=1` -
  2/2 pass (loopback/wildcard/IPv6 binds; child attribution; listener gone
  after the host job closes).
- `cargo test -p burrow-cli --test listeners -- --test-threads=1` - 3/3 pass
  (wildcard without `expose` warns; wildcard with `expose` accepted; loopback
  accepted).
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **164 passed, 0 failed across 19 suites**.
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 1m03s.

**Not verified**: UDP listeners (not inspected), processes outside the
supervised tree, a locked-down machine that denies the owner-PID query (the
warning path is implemented but not exercised), and Podman.

**Next**: A2.2 interrupted restore/seed/initdb recovery, A2.3 corrupt secrets
and `doctor` live-state preservation, then A2.7 hardlink/ADS coverage.

## 2026-10-26 - cycle 6: new-user README pass

**Task**: Follow only the README on a fresh project state; log every friction
point and fix documentation that misled a new user.

**Outcome**: The documented Node.js + PostgreSQL path worked end to end. Four
documentation contradictions were found and fixed; no product code changed.

**Commands and real results**:

- Removed `examples\node-postgres\.burrow` (gitignored runtime state), ran
  `npm install`, then `burrow up --detach --yes` with the release binary.
- Fresh cluster, warm package cache: both services healthy in **10.28 s**
  (the README had implied every later start is ~2 s; fixed).
- `curl http://api.localhost/api/todos` returned **200** and the seeded row.
- `burrow doctor --json`: **11 ok / 0 warnings / 0 failures**, including
  `listeners`: "2 supervised listeners; none accept non-loopback traffic
  without expose".
- `BURROW_NO_BROWSER=1 burrow open` printed `http://api.localhost`.
- `burrow down`: **0.61 s**, both service PIDs gone, `state.json` removed.
- Fixed README limitations (registry packages, opt-in `watch`, container
  fallback), FAQ registry/watch answers, and the warm-start wording.
- Created `docs/agent/FRICTION.md`.

**Not verified**: this was the development machine, not a VM/Sandbox/CI clean
machine; `cargo install` was not re-run; the package cache was warm. The Phase 1
clean-machine pass still has to repeat this from a fresh environment.

**Next**: A2.2 interrupted restore/seed/initdb recovery, A2.3 corrupt secrets
and `doctor` live-state preservation, then A2.7 hardlink/ADS coverage.



## 2026-10-26 - cycle 7: crash-safe restore/init/seed and corrupt secrets

**Task**: Close A2.2 (interrupted restore/seed/initdb) and A2.3 (corrupt
secrets and `doctor` live-state preservation) before launch work.

**Outcome**: Restores are journaled and recover deterministically; partial
initialization is preserved; PostgreSQL seeds are transactional; corrupt
secrets fail loudly without data loss.

**Commands and real results**:

- `cargo test -p burrow-data` - **13/13 pass** (uncommitted rollback, committed
  cleanup, staging without a journal, orphan backup refusal, junction, partial
  init quarantine, empty directory left in place, seed marker lifecycle).
- `cargo test -p burrow-cli --bin burrow doctor` - **6/6 pass**.
- `cargo test -p burrow-cli --test data_safety -- --test-threads=1` -
  **6/6 pass** (daemon startup rollback, snapshot rollback, corrupt secrets).
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **173 passed, 0 failed across 19 suites**.
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 52.20 s.

**Behavior changes**:

- `restore` writes `.burrow/data/.burrow-restore-journal.json` before the first
  swap and marks it committed before deleting backups. The next `up`,
  `snapshot`, `restore`, or `reset` rolls back an uncommitted restore or
  finishes cleanup for a committed one, printing a `note:` line.
- A non-empty data directory without the init marker is moved to
  `.burrow/data/.burrow-init-failed-<service>-<pid>-<unix>` before retry; empty
  directories are left alone.
- `psql` seeds run with `--single-transaction`.
- `doctor` parses secrets with the `up`/`env` schema; a corrupt file fails the
  `secrets` check with a delete-to-regenerate remedy and is never rewritten.

**Not verified**: a real kill during a live `initdb` or `psql` seed. Recovery is
proven at the journal/data-directory level and by killing between swaps.
Orphan backups remain preserved for manual inspection by design.

**Next**: A2.7 hardlink/ADS/reserved-name coverage, then A3 fuzzing, A4
property tests, A6 `doctor --report`, and the Phase 1 clean-machine pass.
