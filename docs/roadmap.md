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
      build artifact, `cargo audit`, and `cargo deny`
- [x] Supervisor core: one process in a Job Object with a memory cap,
      kill-on-close, and a graceful Ctrl+Break shutdown path
- [x] Orphan-free shutdown integration tests using a real child process
- [x] M1 documentation: README, architecture, ADRs, and contributing guide

M1 is a foundation only. Burrow is not usable end-to-end until M2 ships.

## M2 - Config and graph

- [x] Parse and validate `burrow.toml`; merge `burrow.local.toml` overrides
- [x] Template variables such as `{{service.port}}` and generated passwords;
      unknown variables are errors
- [x] Dependency graph validation with the cycle path in the error
- [x] Start services in topological order, parallelizing independent branches
- [x] Health checks with configurable timeout, retries, and interval
- [x] Restart policy with exponential backoff and a crash-loop breaker
- [x] Per-service log capture, switching, and `burrow logs`
- [x] Commands: `up`, `down`, `restart`, `status`, `logs`

## M3 - Package manager

- [x] Registry TOML format, one definition file per service
- [x] SHA-256-verified, resumable downloads into a shared content-addressed cache
- [x] Hardened archive extraction that rejects zip-slip and path traversal
- [x] PostgreSQL registry entry with a verified upstream source and hash
- [x] Database initialization and one-time seeding
- [x] Commands: `pkg list`, `pkg add`, `pkg remove`, `pkg gc`, `pkg verify`

## M4 - Networking

- [x] Stable port allocation across restarts, with collision avoidance
- [x] Generated credentials stored in `.burrow/secrets` and never logged
- [x] Environment templating and URL generation
- [x] Loopback-only reverse proxy for `*.localhost` with WebSocket passthrough
- [x] Commands: `open`, `env`, `exec`

## M5 - Experience

- [x] ratatui dashboard with live state, resource use, and a log pane (`top`)
- [x] `doctor` checks with a concrete remedy for every failure
- [x] `init` with stack detection
- [x] Trust prompt with approval keyed by a hash of the config
- [x] `burrow schema` emitting the `burrow.toml` JSON Schema
- [x] JSON output where supported

## M6 - Data and polish

- [x] Snapshot, restore, list, and reset using atomic directory operations
- [x] Benchmarks with real measurements and recorded machine specs
- [x] Two runnable examples: Node.js API + PostgreSQL, and Python FastAPI +
      PostgreSQL (no Redis entry exists; the registry documents why)
- [x] Full user documentation: getting started, config reference, registry
      authoring, security and trust model, troubleshooting, FAQ, CLI reference,
      demo script
- [x] Release pipeline, `cargo install` support, and winget/Scoop manifest drafts
- [x] Final README with only measured claims

## Phase 2 (complete)

- [x] Idle stop with lazy wake-up on the next connection. `idle_stop` counts
      established connections to the service port, stops the service after the
      configured quiet period, holds the port with a wake listener, and wakes
      the service on the next connection; routed services wake through the
      proxy, which waits for a healthy upstream
      ([ADR 0003](decisions/0003-idle-stop-and-lazy-wake.md))
- [x] File watching and restart on change. `watch` roots are monitored with
      `ReadDirectoryChangesW`, changes are debounced per service, and only the
      watched service restarts ([ADR 0004](decisions/0004-file-watching.md))
- [x] More registry services: `nats@2.15.1` and `caddy@2.11.7`, both pinned to
      official upstream Windows artifacts with locally verified SHA-256 hashes
- [x] Optional container fallback: `use = "container:IMAGE"` delegates to
      Docker or Podman, loopback-only, labelled in `burrow status`, with
      leftover-container cleanup and an honest list of limitations
      ([ADR 0005](decisions/0005-container-fallback.md))

## Good first issue candidates

These are scoped suggestions, not commitments. Each one is self-contained, has a
clear test strategy, and does not weaken the trust model. Read
[CONTRIBUTING.md](../CONTRIBUTING.md) first.

- **Shell completions.** Add `burrow completions <shell>` so PowerShell users
  get tab completion. Evaluate a completion crate (such as `clap_complete`) and
  record the verified version before depending on it. Cover the command surface
  with a unit test on the generated output.
- **`burrow env --format cmd`.** Add a `cmd.exe` output format (`set KEY=value`)
  next to the existing dotenv, JSON, and PowerShell formats, with the same
  escaping tests the PowerShell format has.
- **`burrow doctor` long-path check.** Warn when the project directory is at or
  beyond `MAX_PATH` and its 8.3 short name cannot be resolved, since that is the
  case where process creation fails. Do not warn merely because the path is
  long; that case now works. Use the same remedy `up` reports.
- **Snapshot manifest tests.** Extend `burrow-data` tests for the manifest
  version field and for restoring a snapshot that contains a service which was
  later removed from `burrow.toml` (the untouched-service case).
- **`docs/registry.md` walkthrough.** Add a step-by-step annotated example of a
  registry entry using the PostgreSQL file as the reference, aimed at
  contributors adding a second verified package.
- **Example seed variations.** Add a second seed script to one example that
  demonstrates `burrow restore` returning a changed table to its snapshot state,
  with the expected query output in the example README.
