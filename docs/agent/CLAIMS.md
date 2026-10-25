# Claims audit (Tier A1)

Every checked box in `roadmap.md` and every quantitative claim in `README.md`,
mapped to the test, file, or measurement that backs it. Status legend:
**verified** (automated test or recorded measurement), **observed** (verified
manually this run, not automated), **gap** (no backing — queued in QUEUE.md).

## M1 — Foundation

| Claim | Evidence | Status |
| --- | --- | --- |
| Workspace with the seven product crates plus test-only `burrow-testbin` | `Cargo.toml` members; every workspace build compiles it (CI `cargo build --workspace --release`) | verified |
| CLI skeleton covering the full command surface | `crates/burrow-cli/tests/cli.rs::help_lists_the_full_command_surface` (all commands asserted) | verified |
| CI on windows-latest: fmt, clippy `-D warnings`, tests, release artifact, `cargo audit` | `.github/workflows/ci.yml` (verified line-by-line this run); audit job installs cargo-audit | verified |
| One process in a Job Object with memory cap | `job_lifecycle.rs::enforces_a_job_memory_cap` | verified |
| Kill-on-close | `job_lifecycle.rs::dropping_a_process_kills_the_whole_tree` | verified |
| Graceful Ctrl+Break shutdown path | `job_lifecycle.rs::ctrl_break_allows_a_graceful_exit_when_a_console_is_available`, `shutdown_forces_an_unresponsive_process_after_the_grace_period`, `shutdown_terminates_descendants_and_empties_the_job` | verified |
| Orphan-free shutdown with a real child process | `job_lifecycle.rs::job_close_on_host_death_leaves_no_orphans` (separate host process killed with the testbin tree); plus a real-world observation this run: a daemon killed with `taskkill /F` left zero service processes | verified |
| M1 docs (README, architecture, ADRs, contributing) | `README.md`, `docs/architecture.md`, `docs/decisions/0001..0002`, `CONTRIBUTING.md` | verified |

## M2 — Config and graph

| Claim | Evidence | Status |
| --- | --- | --- |
| Parse/validate `burrow.toml`; merge `burrow.local.toml` | `tests/config.rs` (21 tests) incl. `parses_a_minimal_project`, `local_overrides_merge_over_the_base`; `merge.rs::tables_merge_and_scalars_replace` | verified |
| Template variables; generated passwords; unknown variables are errors | `template.rs` tests (short/long forms, password-when-present, unknown-variable, unclosed braces); CLI `env` tests assert the password exists and appears only in service env | verified |
| Dependency graph validation with cycle path | `tests/config.rs::reports_the_cycle_path`, `suggests_a_dependency_typo`; property tests over random graphs | verified |
| Topological start, parallelizing independent branches | `tests/config.rs::startup_plan_groups_independent_services`; daemon starts layer-by-layer (`daemon.rs::start_all`) | verified |
| Health checks with timeout, retries, interval | `health.rs` tests incl. `timeout_is_reported_with_the_last_error`; CLI `a_failed_health_check_stops_the_daemon_and_leaves_no_state` | verified |
| Restart policy with exponential backoff and crash-loop breaker | `restart.rs` tests (exponential cap, breaker trips, stable-run reset); CLI `a_crashing_service_is_restarted_with_backoff` | verified |
| Per-service log capture, switching, `burrow logs` | `logs.rs` tests (ring keeps newest + seq, writer rotates); CLI `daemon_lifecycle_covers_status_logs_auth_restart_and_down` (log lines fetched through the daemon) | verified |
| Commands `up`, `down`, `restart`, `status`, `logs` | CLI tests: `daemon_lifecycle_...`, `down_removes_state_left_by_a_dead_daemon`, `unknown_service_fails_before_starting_anything` | verified |

## M3 — Package manager

| Claim | Evidence | Status |
| --- | --- | --- |
| Registry TOML, one file per service | `registry.rs` tests (prefix selection, actionable errors, hash normalization, invalid documents rejected, `unsupported_native` fallback) | verified |
| SHA-256-verified, resumable downloads into shared content-addressed cache | Implementation: `cache.rs` (`.part` files kept at line 433, resume via Range per module docs, hash-before-use). Automated tests cover hashing, gc, and corrupt-archive verification. **No automated test drives an interrupted-then-resumed download** | verified (code) / gap (resume path untested — queued A2.8) |
| Hardened extraction rejecting zip-slip and traversal | `extract.rs` tests: zip-slip, absolute/reserved names, tar traversal, tar symlinks, non-empty target refusal | verified |
| PostgreSQL entry with verified upstream source and hash | `builtin.rs` test pins URL/size/hash; live `pkg add`/`pkg verify`/`burrow up` reproduced this run (docs/agent/LOG.md cycle 1) | verified |
| Database initialization and one-time seeding | `burrow-data` tests (init marker required, seeding once + script hash, invalid markers treated as missing); examples seed for real | verified |
| `pkg list/add/remove/gc/verify` | CLI tests for all five incl. empty-cache paths and no-download-on-failure assertions | verified |

## M4 — Networking

| Claim | Evidence | Status |
| --- | --- | --- |
| Stable port allocation across restarts, collision avoidance | `burrow-net/src/lib.rs` tests: remembered across loads, fixed ports must be free, unique per run, busy remembered port replaced, pinned survives probe | verified |
| Credentials in `.burrow/secrets`, never logged | Password generation and file handling tested via `env` tests; **no test scans logs/state for secret bytes** | gap (queued A2.5) |
| Environment templating and URL generation | CLI `env_prints_project_endpoints_and_the_selected_service_environment` (incl. `postgres://` package URL template and route URLs) | verified |
| Loopback proxy for `*.localhost` with WebSocket passthrough | `proxy.rs` tests: host routing + 404, websocket upgrade + byte tunnel, idle-route wake | verified |
| Commands `open`, `env`, `exec` | CLI tests (`open` prints route URL under `BURROW_NO_BROWSER`, exec env/cwd/exit-code/.cmd handling) | verified |

## M5 — Experience

| Claim | Evidence | Status |
| --- | --- | --- |
| ratatui dashboard with live state, resource use, log pane | `burrow-tui` tests (render services/logs/hints, selection bounds, empty project, formatting); CLI `top_requires_an_interactive_terminal`. Live rendering verified manually during development; **no automated end-to-end TUI session** | verified (unit) / gap (e2e — queued E5) |
| `doctor` with a concrete remedy for every failure | CLI doctor tests (valid project ok + JSON shape, missing config fails with remedy); every `Check::new(...Fail...)` in `doctor.rs` carries a remedy | verified |
| `init` with stack detection | CLI init tests: Vite detection + refusal to overwrite, postgres template + seed file, no-stack guidance | verified |
| Trust prompt keyed by hash of the config | `trust_is_required_without_yes`; `executable_hash_tracks_commands_but_not_limits`; hash re-prompt logic in `project.rs::ensure_trust` | verified |
| `burrow schema` | `schema_mentions_the_main_fields` (+ generated `docs/burrow.schema.json`) | verified |
| JSON output where supported | `--json` asserted across status/logs/env/doctor/snapshots/pkg commands | verified |

## M6 — Data and polish

| Claim | Evidence | Status |
| --- | --- | --- |
| Snapshot/restore/list/reset with atomic directory operations | `snapshot.rs` tests (name escape rejected, round trip incl. absent services, duplicates/unknown errors, temp dirs not listed); CLI `snapshots_restore_data_atomically_and_reset_removes_it` | verified |
| Benchmarks with real measurements and machine specs | `docs/benchmarks.md`: real table (cold 61.35 s / warm 1.87 s, RSS), machine spec, reproduction script; Docker comparison honestly skipped with reason | verified |
| Two runnable examples | `examples/node-postgres`, `examples/python-postgres` with READMEs; node example exercised during phase-2 registry work (healthy); **no automated example smoke test in CI** | observed (gap queued E1) |
| Full user documentation | `docs/`: getting-started, config-reference, registry, security, troubleshooting, FAQ, CLI, demo — updated this run for phase 2 | verified |
| Release pipeline, `cargo install` support, winget/Scoop drafts | `.github/workflows/release.yml` + `packaging/`; README documents `cargo install --path ... --locked` (used to build locally during this run, equivalent source build); winget manifest is a draft placeholder by design | verified (drafts marked as drafts) |
| README with only measured claims | "about two seconds" warm start ↔ 1.87 s measured; 373,254,386 bytes ↔ builtin test. The "fraction of the resources" headline has **no Docker comparison measurement yet** — the daemon is now running locally, comparison queued (E3) | partial (queued E3) |

## Phase 2

| Claim | Evidence | Status |
| --- | --- | --- |
| Idle stop with lazy wake-up | `tests/phase2.rs::idle_stop_parks_a_service_and_wakes_it_on_the_next_connection` (held connection keeps awake; idle status with no PID; wake on next connection; tunnelled greeting + echo; new PID); `proxy.rs::an_idle_route_is_woken_before_the_proxy_connects`; ADR 0003 | verified |
| File watching and restart on change | `tests/phase2.rs::watch_restarts_a_service_only_for_watched_paths` (restart on watched write, no restart for `.burrow`); `watch.rs` unit tests (create/modify/remove events, missing root rejected) | verified |
| More registry services | `builtin.rs` tests pin nats/caddy URL+size+hash; live `pkg add` + `pkg verify` + `burrow up` with both healthy this run (LOG cycle 1) | verified |
| Optional container fallback | `tests/phase2.rs`: missing-engine failure with remedy; live Docker 24.0.7 round trip (healthy, `container:...` label, HTTP 200, no leftover container). Config validation tests for `container:`/`container_port`/seed rejection. Podman path untested (no podman here) | verified (docker) / gap (podman — queued C13) |

## Post-audit hardening (added after the audit)

| Claim | Evidence | Status |
| --- | --- | --- |
| Orphan-free under daemon force-kill (M1, end-to-end) | `tests/adversarial.rs::a_force_killed_daemon_leaves_no_orphans_and_the_project_recovers` (3/3 runs) | verified |
| Junctions in data directories are refused (M6) | `snapshot.rs::a_junction_inside_a_data_directory_is_rejected` (real mklink /J) | verified |
| Dependency audit green (CI claim) | `cargo audit` locally green after zip 9.1.0 (`fbd6459`); CI job unchanged | verified |

## Gaps found by this audit (queued)

1. ~~A2.5 secrets-scanning test~~ **closed**: `tests/secrets.rs` scans
   daemon.log, service logs, state.json, status/logs/env output, and a
   snapshot; positive control via `burrow env <service>`.
2. A2.8 interrupted-download resume test (M3) — still open.
3. E1 automated example smoke tests in CI (M6).
4. E3 Docker Compose comparison measurement for the README headline (now
   possible; engine running).
5. E5 end-to-end TUI session test (M5 covered by unit tests only).
6. C13 podman verification of the container fallback.
7. A-flake-1 timing-sensitive health-timeout test (observed once).
