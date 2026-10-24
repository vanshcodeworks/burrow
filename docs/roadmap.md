# Roadmap

Burrow is built milestone by milestone. Each milestone must pass the full check
suite before the next one starts. Checked boxes mean the code exists in the
repository; CI results are the authority on whether a revision is green.

## M1 - Foundation

- [x] Cargo workspace with the seven product crates plus a test-only
      `burrow-testbin` binary target inside `burrow-supervisor`
- [x] `burrow` CLI skeleton covering the full command surface, with clean
      "not implemented yet" errors for later milestones
- [x] CI on `windows-latest`: rustfmt, clippy (`-D warnings`), tests, release
      build artifact, and `cargo audit`
- [x] Supervisor core: one process in a Job Object with a memory cap,
      kill-on-close, and a graceful Ctrl+Break shutdown path
- [x] Orphan-free shutdown integration tests using a real child process
- [x] M1 documentation: README, architecture, ADRs, and contributing guide

M1 is a foundation only. Burrow is not usable end-to-end until M2 ships.

## M2 - Config and graph

- [ ] Parse and validate `burrow.toml`; merge `burrow.local.toml` overrides
- [ ] Template variables such as `{{service.port}}` and generated passwords;
      unknown variables are errors
- [ ] Dependency graph validation with the cycle path in the error
- [ ] Start services in topological order, parallelizing independent branches
- [ ] Health checks with configurable timeout, retries, and interval
- [ ] Restart policy with exponential backoff and a crash-loop breaker
- [ ] Per-service log capture, switching, and `burrow logs`
- [ ] Commands: `up`, `down`, `restart`, `status`, `logs`

## M3 - Package manager

- [ ] Registry TOML format, one definition file per service
- [ ] SHA-256-verified, resumable downloads into a shared content-addressed cache
- [ ] Hardened archive extraction that rejects zip-slip and path traversal
- [ ] PostgreSQL registry entry with a verified upstream source and hash
- [ ] Database initialization and one-time seeding
- [ ] Commands: `pkg list`, `pkg add`, `pkg remove`, `pkg gc`, `pkg verify`

## M4 - Networking

- [ ] Stable port allocation across restarts, with collision avoidance
- [ ] Generated credentials stored in `.burrow/secrets` and never logged
- [ ] Environment templating and URL generation
- [ ] Loopback-only reverse proxy for `*.localhost` with WebSocket passthrough
- [ ] Commands: `open`, `env`, `exec`

## M5 - Experience

- [ ] ratatui dashboard with live state, resource use, and a log pane (`top`)
- [ ] `doctor` checks with a concrete remedy for every failure
- [ ] `init` with stack detection
- [ ] Trust prompt with approval keyed by a hash of the config
- [ ] `burrow schema` emitting the `burrow.toml` JSON Schema
- [ ] JSON output where supported

## M6 - Data and polish

- [ ] Snapshot, restore, list, and reset using atomic directory operations
- [ ] Benchmarks with real measurements and recorded machine specs
- [ ] Two runnable examples: Node/React + Postgres, and Python + Postgres + Redis
- [ ] Full user documentation: getting started, config reference, registry
      authoring, security and trust model, troubleshooting, FAQ
- [ ] Release pipeline, `cargo install` support, and winget/Scoop manifest drafts
- [ ] Final README with only measured claims

## Phase 2

Only after M6:

- [ ] Idle stop with lazy wake-up on the next connection
- [ ] File watching and restart on change
- [ ] More registry services
- [ ] Optional container fallback for services without a trustworthy native
      Windows build
