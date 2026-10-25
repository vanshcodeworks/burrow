# Agent state (overwrite each cycle)

## Current focus

Tier A1: independent audit of every roadmap claim into `CLAIMS.md`.

## Last cycle summary (2026-10-25)

Phase 2 landed on `agent/work` (commits `aec71e2..73786d7`):
idle-stop + lazy wake-up (proxy channel and TCP wake listener),
watch-based restarts with debounce, container fallback
(`use = "container:IMAGE"` via Docker/Podman), and verified registry
entries `nats@2.15.1` and `caddy@2.11.7` (real downloads, hashes,
`pkg add`, `pkg verify`, and a live `burrow up` — both healthy).
ADRs 0003-0005 written; roadmap, architecture, config reference,
registry guide, security, FAQ, README updated.

## Check suite status (last full run)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green**
  (151 tests across 14 suites; one prior flake of
  `a_failed_health_check_stops_the_daemon_and_leaves_no_state` under
  compile load — passed on rerun and in isolation; see QUEUE A-flake-1)
- `cargo audit`: **not run locally** — install failed
  (`cargo install cargo-audit` build error, os error 3; retry queued).
  CI runs it on windows-latest.
- Last green commit: `73786d7` (all work committed; tree clean).

## Environment notes

- Windows 10 x64, Rust stable via `C:\Users\Asus\.cargo\bin` (cargo not
  on the Git Bash PATH by default — export it in every shell).
- Docker Desktop 24.0.7 present and running (engine verified live for
  the container fallback test).
- CI: GitHub Actions windows-latest; local mirror is
  `scripts/check.ps1`.

## Known bugs / follow-ups

See `QUEUE.md`. Top: audit claims (A1), adversarial suite (A2), the
timing-sensitive health-timeout test.
