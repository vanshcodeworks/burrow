# Burrow

**Docker Compose's reproducibility, at a fraction of the resources, native on Windows.**

Burrow is a Windows-native, ultra-lightweight local development environment
manager written in Rust. You describe a full-stack project in one `burrow.toml`.
The goal is for `burrow up` to start every service as a native Windows process,
with no Docker, VM, or WSL required: automatic ports, generated environment
variables, health-checked startup order, per-service resource limits, clean
teardown, and a live dashboard.

## Status

**M1 (foundation) — not usable end-to-end yet.** The repository contains the
workspace, the complete CLI command surface, the CI pipeline, and the first
supervisor primitives. `burrow --help` and `burrow --version` work. Every planned
subcommand exits with code 1 and prints a clear "not implemented yet" message
naming the milestone that will deliver it.

## What works now

- The Cargo workspace with the seven product crates from the design, plus a
  test-only `burrow-testbin` binary target inside `burrow-supervisor`.
- The `burrow` CLI skeleton. It parses and documents the full command surface,
  including the global `-v/--verbose` and `--json` flags.
- The Windows supervisor library: a child process runs inside a Job Object with a
  memory cap, kill-on-close, and a graceful Ctrl+Break shutdown path.
- Tests and checks: CLI integration tests, supervisor integration tests, and a
  `windows-latest` CI workflow that runs rustfmt, clippy, tests, and a release
  build.

## Quick start (contributors, Windows)

### Prerequisites

- Windows 10 or 11, x86_64.
- Rust stable through rustup (edition 2021, MSRV 1.82).
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

`burrow --help` and `burrow --version` work today. Every command in the table
below is parsed but not implemented yet; it exits with code 1 and names its
planned milestone.

| Command | Purpose | Status |
| --- | --- | --- |
| `burrow init [--template <name>]` | Scaffold `burrow.toml` and detect the stack | Not implemented (M5) |
| `burrow up [service...] [--detach] [--yes]` | Start all or selected services | Not implemented (M2) |
| `burrow down` | Stop everything for this project | Not implemented (M2) |
| `burrow restart <service>` | Restart one service | Not implemented (M2) |
| `burrow status` | Show service state and resource use | Not implemented (M2) |
| `burrow logs [service] [-f] [--since <duration>]` | Print or follow service logs | Not implemented (M2) |
| `burrow open [service]` | Open a routed URL in the browser | Not implemented (M4) |
| `burrow env [--format dotenv\|json\|powershell]` | Print the resolved environment | Not implemented (M4) |
| `burrow exec <service> -- <cmd>...` | Run a command with a service's environment | Not implemented (M4) |
| `burrow doctor` | Diagnose local environment problems | Not implemented (M5) |
| `burrow top` | Open the terminal dashboard | Not implemented (M5) |
| `burrow pkg list\|add\|remove\|gc\|verify` | Manage packages and the cache | Not implemented (M3) |
| `burrow snapshot <name>` | Snapshot a service's data | Not implemented (M6) |
| `burrow restore <name>` | Restore a named snapshot | Not implemented (M6) |
| `burrow snapshots` | List snapshots | Not implemented (M6) |
| `burrow reset <service>` | Reset a service's data directory | Not implemented (M6) |
| `burrow schema` | Emit the `burrow.toml` JSON Schema | Not implemented (M5) |

## Safety and trust

A checked-in `burrow.toml` can run arbitrary commands, just like a `Makefile` or
`package.json` scripts. Burrow's planned behavior is to show every command and
every download before the first start, require explicit confirmation, and
re-prompt when those entries change. `--yes` is for CI; nothing is auto-approved
silently.

Resource limits and process cleanup are operational safeguards, **not a security
boundary**. Services run as your Windows user with the same filesystem and
network access you have. See [`docs/architecture.md`](docs/architecture.md) for
the M1 safety rules. The full security and trust-model document is planned for M6
and is tracked in [`docs/roadmap.md`](docs/roadmap.md).

## Limitations

- Windows 10/11 only. There is no Linux or macOS support.
- M1 has no configuration parser and no working `up` or `down`; the environment
  is not usable end-to-end yet.
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
