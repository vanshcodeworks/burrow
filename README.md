# Burrow

**Docker Compose's reproducibility, at a fraction of the resources, native on
Windows.**

Burrow is a Windows-native local development environment manager written in
Rust. You describe a full-stack project in one `burrow.toml`, and `burrow up`
starts every service as a native Windows process: no Docker daemon, virtual
machine, or WSL distribution in the loop. Burrow allocates stable ports,
downloads and verifies managed packages, generates environment variables and
passwords, waits for health checks, captures logs, and shows a live dashboard.

## Status

**M1-M6 and Phase 2 complete.** Burrow can load and validate a `burrow.toml`,
start services in dependency order, manage verified package downloads, expose
routed `*.localhost` URLs through a loopback proxy, print or inject resolved
environments, scaffold and diagnose projects, show a live terminal dashboard,
snapshot and restore data, and run the full command surface with JSON output
where documented. Phase 2 adds idle-stop with lazy wake-up (`idle_stop`),
restart-on-change file watching (`watch`), two more verified registry packages
(`nats`, `caddy`), and an optional container fallback
(`use = "container:IMAGE"` through Docker or Podman) for services without a
trustworthy native Windows build.

There is no published release yet. The supported install path is a source
checkout; release archives, winget, and Scoop manifests are drafts in
[`packaging/`](packaging/).

## Quick start

Requirements:

- Windows 10 or 11, x86_64.
- A normal user terminal. Do not run Burrow elevated; PostgreSQL's Windows build
  refuses to initialize a cluster with an administrator token.
- The runtimes your services need (`node`, `npm`, `python`, `cargo`, `go`, ...)
  on `PATH`. Burrow does not install language runtimes.
- To build from source: Rust 1.85 or newer and the MSVC build tools with the
  Windows SDK.

From a checkout, this one command builds the release binary and installs it:

```powershell
cargo install --path crates\burrow-cli --locked
```

It places `burrow.exe` in `%USERPROFILE%\.cargo\bin` (normally already on
`PATH`). It does not install a Windows service or write machine-wide state.
Burrow downloads packages only from its embedded registry, and only when a
service that uses one starts.

Run the Node.js + PostgreSQL example:

```powershell
cd examples\node-postgres
npm install
burrow up
```

The first start prints a trust summary, downloads the verified PostgreSQL
archive (373,254,386 bytes), initializes the cluster, seeds it, and starts both
services. Later starts that reuse both the shared cache and the initialized
cluster take about two seconds on the benchmark machine. Open
<http://api.localhost> or the direct loopback URL from `burrow status`. Press
`Ctrl+C` or run `burrow down` to stop.

For the Python FastAPI example, activate a virtual environment first because
managed services inherit the daemon's environment:

```powershell
cd examples\python-postgres
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
burrow up
```

See [`docs/getting-started.md`](docs/getting-started.md) for the full walkthrough,
including what to do when `api.localhost` does not resolve.

## Demo

[`docs/demo.md`](docs/demo.md) is a five-minute recording script with the exact
commands and representative output for the Node.js + PostgreSQL example. No
video or GIF is published yet; the script is the source of truth.

## Measured performance

These are real measurements from `scripts/benchmark.ps1`, not estimates. They
were recorded on an AMD Ryzen 7 4800H (8 cores / 16 threads), 15.4 GB RAM,
Windows 11 10.0.26200, with a release build of Burrow 0.1.0 on 2026-10-24.

| Scenario | Burrow ready after | Burrow daemon RSS | Whole environment RSS |
| --- | ---: | ---: | ---: |
| Cold start, empty cache | 61.35 s | 12.6 MB | 178.6 MB |
| Warm start, populated cache | 1.87 s | 10.6 MB | 172.1 MB |

A second warm run measured 1.95 s and 10.7 MB daemon RSS. The cold start includes
the 356 MiB download, SHA-256 verification, extraction, `initdb`, both service
starts, and health checks. Whole-environment RSS sums the daemon and every
descendant process; PostgreSQL accounts for roughly 100 MB and Node for roughly
55 MB, so Burrow's own share is the daemon figure.

Both published targets were met: the daemon stays below 20 MB idle, and a warm
`burrow up` is well under 10 seconds. The method and machine details are in
[`docs/benchmarks.md`](docs/benchmarks.md). Docker CLI 24.0.7 was installed, but
its daemon was not running during the measurement, so no Docker comparison is
reported. The benchmark script skips that comparison rather than inventing a
number.

## Fair comparison

Burrow is not a drop-in Docker Compose replacement. It targets native Windows
services that already have trustworthy builds.

| Property | Burrow | Docker Desktop + Compose on Windows |
| --- | --- | --- |
| Process model | Native Windows processes, one Job Object per service | Linux containers in a managed VM / WSL 2 |
| Resource overhead | 10.6-12.6 MB daemon RSS measured here | Not measured here; Docker daemon was not running |
| Startup | 1.87-1.95 s warm for this example | Not measured here |
| Isolation | None; services share your user account | Namespaces, cgroups, and a VM boundary |
| Linux binaries and images | Not supported | Supported |
| Reproducible inputs | Registry entries pin URL, size, SHA-256, and version | Image tags and digests |
| Platform | Windows 10/11 x86_64 only | Cross-platform |
| Best fit | Native Windows services, small local stacks | Linux services, isolation, and image-based workflows |

Containers provide filesystem and network isolation, reproducible base images,
and Linux userspace. Burrow provides none of those. Read
[`docs/security.md`](docs/security.md) before deciding whether that trade-off is
appropriate for a project.

## What works now

- One `burrow.toml` with strict validation, local overrides, template expansion,
  dependency graphs, cycle reports, generated per-project secrets, and a checked
  JSON Schema.
- A supervisor that runs each service in a Job Object with memory and CPU limits,
  health-gated startup, restart backoff, a crash-loop breaker, graceful shutdown,
  and per-service rotating logs.
- A content-addressed package cache with SHA-256 verification, resumable
  downloads, hardened extraction, and one-time database initialization/seeding.
- Stable per-project ports, a `*.localhost` reverse proxy with WebSocket
  passthrough, and `open`, `env`, and `exec`.
- Stack detection and scaffolding with `burrow init` for Node (Vite/Next/Express),
  Python (FastAPI/Flask), Cargo, Go, and PostgreSQL.
- A read-only `burrow doctor` with a remedy for every failure, plus `--json`.
- A ratatui dashboard (`burrow top`) with live state, ports, CPU, memory, uptime,
  and logs.
- Full-directory snapshots, restore with rollback, snapshot listing, and reset.
- Unit and integration tests across the workspace, plus a `windows-latest` CI
  workflow for rustfmt, clippy, tests, release builds, and `cargo audit`.

## Commands

| Command | Purpose |
| --- | --- |
| `burrow init [--template <name>]` | Scaffold `burrow.toml` and detect the stack |
| `burrow up [service...] [--detach] [--yes]` | Start all or selected services |
| `burrow down` | Stop everything for this project |
| `burrow restart <service>` | Restart one service and its dependents |
| `burrow status` | Show service state, ports, and resource use |
| `burrow logs [service] [-f] [--since <duration>]` | Print or follow logs |
| `burrow open [service]` | Open a routed URL in the browser |
| `burrow env [service] [--format dotenv\|json\|powershell]` | Print endpoints or a service environment |
| `burrow exec <service> -- <cmd>...` | Run a command with a service environment |
| `burrow doctor` | Diagnose local environment problems |
| `burrow top` | Open the terminal dashboard |
| `burrow pkg list\|add\|remove\|gc\|verify` | Manage packages and the cache |
| `burrow snapshot <name>` | Copy service data |
| `burrow restore <name>` | Restore a snapshot |
| `burrow snapshots` | List snapshots |
| `burrow reset <service>` | Delete one service's data directory |
| `burrow schema` | Emit the `burrow.toml` JSON Schema |

`burrow exec` propagates the child process exit code. Other commands exit `0` on
success and `1` on failure; command-line usage errors exit `2`. See
[`docs/cli.md`](docs/cli.md) for flags, JSON shapes, and exact behavior.

## Safety and trust

A checked-in `burrow.toml` can run arbitrary commands, just like a `Makefile` or
`package.json` scripts block. The first `burrow up` shows every command and every
download, then requires explicit approval. The approval is keyed by a hash of the
project name, service names, and each service's `run`, `use`, and `cwd` values.
Changing `env`, health checks, ports, routes, or the contents of scripts does not
re-prompt. `--yes` approves non-interactively for reviewed CI configurations;
without a terminal and without `--yes`, Burrow refuses to start.

Generated credentials live in `.burrow/secrets` as plaintext JSON readable by
your Windows account. They are excluded from normal logs and status output, but
`burrow env <service>` prints them because that command exists to export a
service environment. Do not commit `.burrow/` or paste its contents into an issue.

The reverse proxy binds `127.0.0.1` by default. A route becomes reachable from
other machines only when its service sets `expose = true`, and Burrow warns when
it binds all interfaces.

Resource limits and process cleanup are operational safeguards, **not a security
boundary**. Services run as your Windows user with the same filesystem and
network access you have. The full trust model is in
[`docs/security.md`](docs/security.md); report vulnerabilities as described in
[`SECURITY.md`](SECURITY.md).

## Limitations

- Windows 10/11 x86_64 only. There is no Linux or macOS backend.
- No sandbox, privilege separation, or production support. Services run with your
  full user rights.
- The built-in registry contains PostgreSQL 16.15, NATS 2.15.1, and Caddy
  2.11.7. There is no managed Redis or MySQL entry; adding one requires a
  verified native Windows build and a registry change described in
  [`docs/registry.md`](docs/registry.md). Unsupported services can use
  `use = "container:IMAGE"` with a separately installed Docker or Podman.
- Snapshots are full directory copies, not database dumps. They require the
  daemon to be stopped, are not incremental, and do not understand database
  formats.
- File watching is opt-in per service and only watches the directories listed in
  `watch`; it is not a whole-project watcher. Framework watchers such as Vite,
  `uvicorn --reload`, and `cargo watch` still work inside a service.
- `*.localhost` resolution depends on the machine. Use the direct loopback URL or
  add a hosts entry; the proxy works either way.
- No GUI, cloud integration, telemetry, or auto-updater. The optional container
  fallback requires Docker or Podman and its containers have ephemeral
  filesystems that snapshots do not cover.
- No published release or signing certificate yet. Winget and Scoop manifests are
  drafts with explicit placeholders, not working installers.

## Documentation

- [Getting started](docs/getting-started.md)
- [CLI reference](docs/cli.md)
- [Configuration reference](docs/config-reference.md) and [JSON Schema](docs/burrow.schema.json)
- [Registry guide](docs/registry.md)
- [Security and trust](docs/security.md)
- [Troubleshooting](docs/troubleshooting.md) and [FAQ](docs/faq.md)
- [Architecture](docs/architecture.md)
- [Benchmarks](docs/benchmarks.md)
- [Demo script](docs/demo.md)
- [Roadmap and good first issues](docs/roadmap.md)
- [Packaging drafts](packaging/README.md)
- [Contributing](CONTRIBUTING.md) and [Code of Conduct](CODE_OF_CONDUCT.md)

## Development

Run the same checks as CI:

```powershell
.\scripts\check.ps1
```

Or individually:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets -- --test-threads=1
```

The `--test-threads=1` setting is required because supervisor tests exercise
process-wide console control events. CI runs on `windows-latest` and uploads a
release `burrow.exe` artifact.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
