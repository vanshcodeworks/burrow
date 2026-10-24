# Burrow

**Docker Compose's reproducibility, at a fraction of the resources, native on Windows.**

Burrow is a Windows-native, ultra-lightweight local development environment
manager written in Rust. You describe a full-stack project in one `burrow.toml`.
The goal is for `burrow up` to start every service as a native Windows process,
with no Docker, VM, or WSL required: automatic ports, generated environment
variables, health-checked startup order, per-service resource limits, clean
teardown, and a live dashboard.

## Status

**M1-M5 complete.** Burrow can load a validated `burrow.toml`, start services in
dependency order as native Windows processes, manage verified package downloads,
expose routed `*.localhost` URLs through a loopback proxy, print or inject
resolved environments, scaffold and diagnose projects, and show a live terminal
dashboard. Snapshots and the full user documentation are still being built (M6).

## What works now

- The seven-crate Cargo workspace plus a test-only `burrow-testbin` binary
  target inside `burrow-supervisor`.
- Strict configuration parsing, local overrides, template expansion, dependency
  graphs, cycle reports, generated per-project secrets, and a JSON Schema.
- A supervisor that runs each service in a Job Object with memory and CPU limits,
  health-gated startup, restart backoff, a crash-loop breaker, and per-service
  log files.
- A content-addressed package cache with SHA-256 verification, resumable
  downloads, hardened extraction, and one-time database initialization/seeding.
- Stable per-project ports, a `*.localhost` reverse proxy with WebSocket
  passthrough, and the `open`, `env`, and `exec` commands.
- Stack detection and scaffolding through `burrow init`, including Node
  (Vite/Next/Express), Python (FastAPI/Flask), Cargo, Go, and PostgreSQL.
- A read-only `burrow doctor` that checks the platform, configuration, cache,
  trust, secrets, packages, fixed ports, commands, daemon, and proxy, with a
  concrete remedy for every problem.
- A ratatui dashboard (`burrow top`) with live service state, ports, CPU,
  memory, uptime, and a scrolling log pane.
- Tests and checks: unit and integration tests across the workspace and a
  `windows-latest` CI workflow that runs rustfmt, clippy, tests, and a release
  build.

## Quick start (contributors, Windows)

### Prerequisites

- Windows 10 or 11, x86_64.
- Rust stable through rustup (edition 2021, MSRV 1.85).
- MSVC build tools with the Windows SDK. In the Visual Studio Build Tools
  installer, select "Desktop development with C++".

### Check, build, and run

```powershell
# rustfmt, clippy, and the full workspace test suite
.\scripts\check.ps1

# inspect the command surface
cargo run -p burrow-cli -- --help
cargo run -p burrow-cli -- --version
```

Run the supervisor example, which starts a child process in a Job Object:

```powershell
cargo run -p burrow-supervisor --example supervise -- cmd /C echo hello
```

## Commands

`burrow --help` and `burrow --version` work today. The lifecycle, package,
environment, networking, and experience commands below are implemented; the
snapshot commands exit with code 1 and name their planned milestone.

| Command | Purpose | Status |
| --- | --- | --- |
| `burrow init [--template <name>]` | Scaffold `burrow.toml` and detect the stack | Implemented |
| `burrow up [service...] [--detach] [--yes]` | Start all or selected services | Implemented |
| `burrow down` | Stop everything for this project | Implemented |
| `burrow restart <service>` | Restart one service | Implemented |
| `burrow status` | Show service state and resource use | Implemented |
| `burrow logs [service] [-f] [--since <duration>]` | Print or follow service logs | Implemented |
| `burrow open [service]` | Open a routed URL in the browser | Implemented |
| `burrow env [service] [--format dotenv\|json\|powershell]` | Print endpoints, or one service's resolved environment | Implemented |
| `burrow exec <service> -- <cmd>...` | Run a command with a service's environment | Implemented |
| `burrow doctor` | Diagnose local environment problems | Implemented |
| `burrow top` | Open the terminal dashboard | Implemented |
| `burrow pkg list\|add\|remove\|gc\|verify` | Manage packages and the cache | Implemented |
| `burrow snapshot <name>` | Snapshot a service's data | Not implemented (M6) |
| `burrow restore <name>` | Restore a named snapshot | Not implemented (M6) |
| `burrow snapshots` | List snapshots | Not implemented (M6) |
| `burrow reset <service>` | Reset a service's data directory | Not implemented (M6) |
| `burrow schema` | Emit the `burrow.toml` JSON Schema | Implemented |

## Safety and trust

A checked-in `burrow.toml` can run arbitrary commands, just like a `Makefile` or
`package.json` scripts. Burrow shows every command and every download before the
first start, requires explicit confirmation, and re-prompts when those entries
change. Approval is keyed by a hash of the executable configuration; `--yes` is
for CI. Nothing is auto-approved silently.

Generated credentials live in `.burrow/secrets` and are excluded from normal
logs and status output. `burrow env <service>` prints them only because that
command exists to export a service environment.

The reverse proxy binds `127.0.0.1` by default. A route is reachable from other
machines only when its service sets `expose = true`, and Burrow prints a warning
when that happens.

Resource limits and process cleanup are operational safeguards, **not a security
boundary**. Services run as your Windows user with the same filesystem and
network access you have. See [`docs/architecture.md`](docs/architecture.md) for
the safety rules. The full security and trust-model document is planned for M6
and is tracked in [`docs/roadmap.md`](docs/roadmap.md).

## Limitations

- Windows 10/11 only. There is no Linux or macOS support.
- Data snapshots, restore, and reset are not implemented yet (M6).
- Resource limits and process cleanup are not a security boundary and do not
  sandbox services.
- Burrow is a local development tool, not a production deployment tool, and does
  not promise production parity.
- No performance numbers are published yet. Benchmarks with real measurements
  are planned for M6.

## Documentation

- [Architecture and M1 supervisor design](docs/architecture.md)
- [Roadmap](docs/roadmap.md)
- [ADR 0001: workspace layout](docs/decisions/0001-workspace-layout.md)
- [ADR 0002: Job Object lifecycle](docs/decisions/0002-job-object-lifecycle.md)
- [Contributing](CONTRIBUTING.md)

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
