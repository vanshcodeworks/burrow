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

## 2026-10-26 - cycle 8: snapshot fidelity (A2.7)

**Task**: Close the remaining A2.7 gaps: hard links, NTFS alternate data
streams, and reserved Windows device names.

**Outcome**: A snapshot now either captures the data it reports or refuses with
a named file/stream. Names that would resolve to Windows devices are rejected
before any path is built.

**Commands and real results**:

- `cargo test -p burrow-data` - **16/16 pass** (hard links copied as independent
  files; `:metadata` stream refused; `con`/`nul`/`com1`/`lpt9`/`aux`/`prn`/
  `con.txt` rejected; existing junction and restore tests still green).
- `cargo test -p burrow-config --test config` - **22/22 pass** (reserved project
  and service names).
- `cargo test -p burrow-cli --test cli pkg_add_rejects_reserved_service_names_before_writing`
  - **1/1 pass**; `burrow.toml` is unchanged and nothing is downloaded.
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **178 passed, 0 failed across 19 suites**.
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 49.00 s.

**Behavior changes**:

- Snapshot copies hard links as independent files (contents preserved, link
  relationship not preserved) and documents it.
- A non-default NTFS stream (`file:name`) refuses the snapshot with
  `alternate data stream `:name` would not be copied`; the file and stream are
  named. Filesystems without stream support are treated as having none.
- Reserved Windows device names are rejected in configuration, snapshot
  manifests, snapshot names, and `pkg add --service`.

**Not verified**: the non-Windows symlink branch; stream scanning on FAT/exFAT
(the "not supported" path is implemented but not exercised here).

**Next**: A3 fuzzing, A4 property tests, A6 `doctor --report`, then the Phase 1
compatibility matrix and clean-machine pass.

## 2026-10-26 - cycle 9: parser fuzzing (A3)

**Task**: Add coverage-guided fuzzing for the parsers that read user and
attacker-controlled files.

**Outcome**: Five cargo-fuzz targets run under AddressSanitizer with no crash,
timeout, or artifact. The fuzz crate is excluded from the workspace, so normal
builds and tests are unchanged.

**Commands and real results**:

- `rustup toolchain install nightly --profile minimal` - installed
  `1.101.0-nightly (12433c307 2026-10-24)`.
- `cargo install cargo-fuzz --locked` - installed `cargo-fuzz 0.13.2`.
- `cargo +nightly fuzz run config_parse -- -max_total_time=30 -print_final_stats=1 fuzz/seeds/config_parse`
  - **96,105 executions**, 0 crashes, 0 artifacts, peak RSS 449 MB.
- `cargo +nightly fuzz run config_template ...` - **294,395 executions**, 0
  crashes, peak RSS 507 MB.
- `cargo +nightly fuzz run registry_parse ...` - **70,859 executions**, 0
  crashes, peak RSS 465 MB.
- `cargo +nightly fuzz run extract_path ...` - **535,360 executions**, 0
  crashes, peak RSS 420 MB. The target asserts every accepted path is relative
  and contains only normal components.
- `cargo +nightly fuzz run state_parse ...` - **353,785 executions**, 0
  crashes, peak RSS 519 MB.
- Total: **1,350,504 executions** across the five 30 s campaigns.
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **179 passed, 0 failed across 20 suites**.
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 53.45 s.

**Setup notes**:

- `--sanitizer none` fails to link on MSVC with unresolved
  `__start___sancov_pcs`/`__stop___sancov_cntrs`. The default ASan build links,
  but the target exits with `STATUS_DLL_NOT_FOUND` until
  `clang_rt.asan_dynamic-x86_64.dll` (from the Visual Studio MSVC toolset) is
  on `PATH`. Both facts are recorded in `fuzz/README.md`.
- `burrow-cli` now has a `src/lib.rs` exposing `state`; `main.rs` imports it as
  `burrow_cli::state` so the module is compiled once. The fuzz target calls the
  new pure `RuntimeState::parse`.
- `render_template` and `extract::validate_entry_path` are now public with doc
  comments; they are the exact functions used by the normal paths.

**Not verified**: a long soak (30 s per target), Linux/macOS ASan, and the tar
and zip readers themselves. The extractor target fuzzes the path validator that
both readers call before writing, not the archive container parsers.

**Next**: A4 property tests (port allocator, atomic directory operations,
template resolution), then A6 `doctor --report` and the Phase 1 compatibility
matrix.

## 2026-10-26 - cycle 10: property tests for ports, snapshots, and templates (A4)

**Task**: Extend the adversarial suite with property tests for the port
allocator, atomic snapshot/restore directory operations, and template
resolution.

**Outcome**: Eight properties covering **1,840 generated cases** pass. One
generator bug was found and fixed; no product bug was found.

**Properties and real results**:

- `crates/burrow-net/tests/properties.rs` - 2 properties x 256 cases:
  - automatic ports are unique across services and inside
    40000-49999;
  - re-allocating an unchanged service returns the remembered port;
  - `save`/`load` preserves every assignment and leaves no `ports.json.tmp`;
  - a reserved port is never handed out, and a pinned service keeps its port.
- `crates/burrow-data/tests/properties.rs` - 1 property x 48 cases:
  - generated file trees round-trip through `create_snapshot` and
    `restore_snapshot`;
  - manifest file and byte counts match the generated tree;
  - a file added after the snapshot does not survive the restore;
  - a service recorded as absent stays absent;
  - no `.burrow-restore-*`, `.burrow-backup-*`, or `.burrow-tmp-*` remains.
- `crates/burrow-config/tests/template_properties.rs` - 5 properties x 256
  cases:
  - text without markers is returned unchanged;
  - a known substitution keeps the surrounding text;
  - an unclosed marker is an error;
  - an unknown variable is an error;
  - `resolve_templates` preserves template-free values.

**Generator bug found**: the first snapshot run failed with
`create parent directory: Os { code: 183, ... }`. The path strategy could
generate `f` and `f/d` in the same map, so a file was also its own parent
directory. This was invalid test input, not a product failure. The strategy now
uses a flat namespace plus one fixed `nested/` prefix; the rerun passes all 48
cases.

**Commands and real results**:

- `cargo test -p burrow-net --test properties -- --test-threads=1` -
  2 passed in 3.38 s.
- `cargo test -p burrow-data --test properties -- --test-threads=1` -
  1 passed in 2.80 s after the generator fix.
- `cargo test -p burrow-config --test template_properties -- --test-threads=1`
  - 5 passed in 0.16 s.
- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **187 passed, 0 failed across 23 suites** (8 new property tests).
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green, cached in 0.41 s (only
  dev-dependencies changed; the previous full build was 53.45 s).

**Not verified**: concurrent allocator processes, real crashes during the
property runs (crash recovery has separate simulated tests), and non-Windows
filesystems.

**Next**: A6 `doctor --report` (redacted bundle and structured local crash
notes), then the Phase 1 compatibility matrix and clean-machine pass.

## 2026-10-26 - cycle 11: local redacted diagnostic bundles (A6)

**Task**: Close Phase 0 with `burrow doctor --report` and structured local crash
notes, without telemetry or uploads.

**Outcome**: `doctor --report` writes a zip (default
`<project>/.burrow/reports/burrow-report-<stamp>-<pid>.zip`;
`--report-dir <DIR>` overrides and implies `--report`) and prints its path.
The CLI installs a panic hook that writes a structured note before chaining to
the previous hook. Nothing leaves the machine.

**Bundle contents**: `README.txt`, `report.json`, `environment.json`, redacted
`config.redacted.json`, redacted `state.redacted.json` when state exists, a
256 KiB tail of `.burrow/daemon.log`, and up to ten crash notes matching the
current project plus notes with no project. Service logs are excluded because
Burrow cannot vouch for arbitrary application output; `environment.json` lists
their names and sizes.

**Redaction**: known values from `.burrow/secrets`, the state token, sensitive
environment values, URL passwords, Bearer tokens, and common
`password`/`token`/`secret` assignments become `<redacted>`. The project
directory, `%USERPROFILE%`, and `%LOCALAPPDATA%` become placeholders. Every
service `env` value and command argument in the copied config is replaced
regardless of its name.

**Two real bugs found by inspecting a manual release-binary bundle**:

1. Paths in JSON were escaped (`C:\\Users\\...`) and did not match the raw
   path needles, so `report.json` and crash notes leaked the project and cache
   paths. `push_path_variants` now also adds the JSON-escaped form.
2. The unquoted-assignment scanner consumed the closing `"` of a JSON string,
   producing `"message": "password=<redacted>,` and invalid JSON. It now stops
   at `"` and `'`; `redaction_keeps_json_valid` and the integration test parse
   every JSON entry.

**Manual evidence**: release binary with `LOCALAPPDATA` pointed at an isolated
directory; bundle 2,494 bytes; entries `README.txt`, `report.json`,
`environment.json`, `config.redacted.json`, `daemon-log.txt`,
`crashes/crash-1-42.json`; scan for raw and JSON-escaped secrets/paths found
none; all four JSON entries parsed. Crash note showed
`project_dir: "<project>"` and `message: "password=<redacted>"`.

**Commands and real results**:

- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **202 passed, 0 failed across 23 suites** (15 new A6 tests: 5 crash unit,
  7 bundle/redaction unit, 3 CLI integration).
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 53.33 s; a rebuild after the
  redaction fix took 54.56 s.

**Not verified**: the panic hook itself has no supported CLI panic path, so its
end-to-end invocation is untested (the writer, reader, filtering, atomic rename,
and redaction are tested); crash notes accumulate without automatic deletion;
redaction remains best-effort and cannot recognize an unknown secret that
matches no pattern.

**Next**: Phase 1 - compatibility matrix (A5), clean-machine/README pass,
time-to-first-success measurement, and error-message pass.

## 2026-10-26 - cycle 12: compatibility matrix and long-path fix (A5)

**Task**: Start Phase 1 with a real compatibility matrix, and fix whatever the
probe finds.

**Probe**: `scripts/compatibility.ps1` creates a package-free project under
`%TEMP%` with a PowerShell TCP listener on `127.0.0.1:$env:PORT` that returns a
minimal HTTP 200. For each directory shape it runs `up --detach --yes`, polls
`status --json` until healthy, makes a direct HTTP request, runs `doctor
--json`, runs `down`, and checks that `.burrow/state.json` is gone. It downloads
nothing, needs no administrator rights, binds only loopback ephemeral ports, and
removes its work root on success.

**Real bug found**: a 279-character project path failed at daemon spawn with
`could not start the Burrow daemon (The directory name is invalid. (os error
267))`. `CreateProcessW` rejects `lpCurrentDirectory` at or beyond `MAX_PATH`,
rejects the `\\?\` prefix in that argument, and `cmd.exe` treats a verbatim
directory as a UNC path and falls back to `C:\Windows`.

**Fix**: `burrow_supervisor::process_working_directory` removes verbatim
prefixes from short paths and converts long paths to their existing 8.3 short
name, resolving it with `GetShortPathNameW` through a verbatim input. A long
`lpApplicationName` is shortened the same way. The helper is applied to
supervised service creation, `cmd` health checks, the detached daemon, `burrow
exec`, the browser launcher used by `burrow open`, and package init/seed steps.
If the short form is unavailable or still too long, the error names the
directory and tells the user to move the project to a path under 260 characters.
ADR 0010.

**Evidence**:

- PowerShell 7.6.7: 4/4 passed. ASCII 64 chars, spaces 81 chars, non-ASCII 68
  chars, long 279 chars. Each: `healthy`, HTTP 200, `doctor 9 ok / 0
  warnings`, `down` removed `.burrow/state.json`.
- Windows PowerShell 5.1.26100.9711: the same 4/4 passed with the same path
  lengths.
- The machine is Windows 11 Home Single Language 10.0.26200 build 26200,
  x86-64, AMD Ryzen 7 4800H, 15.4 GB RAM, non-elevated, `LongPathsEnabled=1`,
  locale `en-NZ`.

**PowerShell 5.1 quirk**: Windows PowerShell reads a BOM-less script as ANSI.
The literal `café 测试 Δ` test path was mangled into smart quotes and the script
failed to parse. The probe now builds that path from code points and is pure
ASCII, so both editions parse the same file.

**Docs**: added `docs/compatibility.md` with verified and unverified rows and
the probe's limits; linked it from the README and getting-started; added a
long-path remedy to troubleshooting and a spawn note to architecture; updated
the roadmap's doctor suggestion; ADR 0010 records the decision.

**CI**: added a `Compatibility probe` step to `.github/workflows/ci.yml` after
the release build. It runs on `windows-latest`; no CI run was available here,
so this is configuration only.

**Commands and real results**:

- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **206 passed, 0 failed across 23 suites** (4 new supervisor unit tests:
  short path unchanged, verbatim prefixes removed, long path resolves to a
  short name that names the same directory, missing long directory errors
  clearly).
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 54.47 s.

**Not verified**: Windows 10, ARM64, a true standard-user account (the test
account is an administrator running filtered), OneDrive and UNC project
directories, FAT/exFAT, third-party antivirus, `LongPathsEnabled=0`, and a
volume with 8.3 names disabled. The probe does not cover packages, the proxy,
snapshots, the dashboard, or containers. A clean-machine pass is still needed.

**Next**: clean-machine/README pass, time-to-first-success measurement, and an
error-message pass.

## 2026-10-26 - cycle 13: time to first success and error-message pass

**Task**: Follow the README as a new user with empty caches, measure time to
first success, and fix misleading error messages.

**Time-to-first-success method**: `git archive HEAD examples/node-postgres`
into a fresh `%TEMP%` root; empty `CARGO_HOME`, `npm_config_cache`, and
`BURROW_CACHE_DIR`; follow the README commands exactly and time each step with
`Stopwatch`. The clock stopped when `status --json` reported both services
healthy **and** `http://127.0.0.1:<api-port>/api/todos` returned a seeded HTTP
200.

**Real results**:

- `cargo install --path crates\burrow-cli --locked` (empty `CARGO_HOME`):
  **162.62 s**, 0.27 GB of crates downloaded, installed `burrow 0.1.0` runs.
- `npm install` (empty npm cache): **3.19 s**, 14 packages.
- `burrow up --detach --yes` (empty Burrow package cache): **79.43 s** to both
  healthy and a seeded HTTP 200. The 356.0 MiB PostgreSQL download, SHA-256
  verification, extraction, `initdb`, seed, and both health checks are inside
  that time.
- **Checkout to first successful request: 245.24 s (4 min 5 s).**
- `burrow down`, then warm `up --detach --yes`: **1.87 s**; the same endpoint
  returned `[{"id":1,"title":"Open http://api.localhost"}]`; `doctor --json`
  reported **11 ok / 0 warnings / 0 failures**.
- `api.localhost` did not resolve on this machine (`DNS name does not exist`).
  The direct loopback URL was used, which is the documented fallback.

Recorded in `docs/benchmarks.md` with caveats (one sample, dev machine, warm OS
and toolchain, real network time) and linked from the README.

**Error-message pass**: probed missing/invalid configs, unknown services,
missing snapshots, a non-existent command, a health timeout, and a fixed-port
conflict. Fixed three real issues:

1. Every config error appended `note: expected burrow.toml ... help: run burrow
   init`, even when the file existed and had a syntax error, unknown field,
   dependency cycle, or reserved name. `project::config_error` now adds the hint
   only for `ConfigError::Io` with `NotFound`; 3 unit tests cover missing,
   invalid, and permission-denied files.
2. `restore nope` now adds `help: run burrow snapshots to list existing
   snapshots`; the existing `snapshots_restore_data_atomically_and_reset_removes_it`
   integration test asserts both lines.
3. `exec nope`, `env nope`, and `up nope` used to report `invalid configuration
   at services: unknown service nope`; `project::ensure_known_services` now
   reports `unknown service nope` plus `help: choose one of: ...`; 2 unit tests.

The three runtime probes were already good and were left alone:

- Bad command: exit 1, `could not start service web: could not start
  definitely-not-a-command-xyz: The system cannot find the file specified.
  (0x80070002)`, no state file, no process.
- Health timeout (2 s HTTP check): exit 1, names the service, 2 attempts,
  2.0157975 s, and both log paths; no state file, no process.
- Fixed port held by a real listener: exit 1, `could not assign a port to web:
  port 45679 is already in use; choose another port or use port = "auto"`; no
  state file, no process.

**Commands and real results**:

- `cargo fmt --all --check` - green.
- `cargo clippy --workspace --all-targets -- -D warnings` - green.
- `cargo test --workspace --all-targets -- --test-threads=1` -
  **211 passed, 0 failed across 23 suites** (5 `project::tests`: 3 config-hint,
  2 service-selection).
- `cargo audit` - green, 3 allowed transitive warnings (`paste`, `lru` x2).
- `cargo build --workspace --release` - green in 127.94 s.
- `scripts/compatibility.ps1` - 4/4 under PowerShell 7.6.7 and 4/4 under
  Windows PowerShell 5.1.26100.9711; ASCII 64, spaces 81, non-ASCII 68, long
  279 characters; each `doctor 9 ok / 0 warnings` and a clean `down`.
- Temp roots removed; zero `burrow` processes remained after the probes.

**Not verified**: this was not a clean VM or Windows Sandbox; the OS, Rust
toolchain, MSVC build tools, Node, and npm were pre-installed. The numbers are
one sample and include network time. A true clean-machine pass remains, as does
a CI execution of the compatibility probe. The health-timeout message says
`2 attempt(s)` and prints a 7-decimal duration; both are cosmetic.

**Next**: close the remaining Phase 1 evidence gaps where possible, then move
to the sourced landscape/comparisons and launch drafts. Do not write
`LAUNCH_READINESS.md` until the blocked rows are resolved.
