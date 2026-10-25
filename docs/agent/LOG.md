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
