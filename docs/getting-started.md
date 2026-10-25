# Getting started

Burrow runs a full local development stack as native Windows processes. One
`burrow.toml` describes the services; `burrow up` allocates ports, downloads and
verifies managed packages, starts services in dependency order, waits for health
checks, and streams their logs. There is no Docker daemon, VM, or WSL
distribution in the loop.

## Requirements

- Windows 10 or Windows 11, x86-64.
- A normal user account. Do not run Burrow from an elevated terminal:
  PostgreSQL's `initdb` refuses to initialize a cluster with an administrator
  token.
- The tools your services need (`node`, `npm`, `python`, `cargo`, `go`, ...) on
  `PATH`. Burrow starts your command; it does not install language runtimes.
- To build from source: Rust 1.85 or newer and the MSVC toolchain.

## Install

This repository does not yet publish crates.io or GitHub release artifacts.
Build the only supported artifact from a checkout:

```powershell
git clone <repository-url>
cd Burrow
cargo install --path crates\burrow-cli --locked
```

`cargo install` places `burrow.exe` in `%USERPROFILE%\.cargo\bin`, which is
normally already on `PATH`. Confirm it:

```powershell
burrow --version
```

If you prefer not to install, `cargo run -p burrow-cli -- <command>` works from
the checkout. Do not copy a debug `target\debug\burrow.exe` over an installed
release build.

Future release archives, winget, and Scoop manifests are described in
[`packaging/`](../packaging/). Until a release exists, those files contain
explicit placeholders and are drafts, not working installers.

## Your first project

### Option A: scaffold in an existing app

Change into the project directory and let Burrow inspect it:

```powershell
cd C:\dev\my-app
burrow init
```

`burrow init` detects Node.js (`package.json`), Python (`main.py` or `app.py`),
Cargo (`Cargo.toml` + `src/main.rs`), and Go (`go.mod`), in that order. It writes
`burrow.toml` and refuses to overwrite an existing one. To choose explicitly:

```powershell
burrow init --template node       # or python, postgres, cargo, go
```

Every template is validated before anything is written. Review the generated
commands; they are ordinary shell command lines that Burrow will execute.

### Option B: start from an example

The two runnable examples use the same verified PostgreSQL package:

```powershell
cd examples\node-postgres
npm install
burrow up
```

```powershell
cd examples\python-postgres
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
burrow up
```

The Python example must be started from the activated shell because managed
services inherit the daemon's environment. The first start downloads the
PostgreSQL archive (373,254,386 bytes) into the shared cache and verifies its
SHA-256 before extracting it. Later projects reuse the extracted copy.

### Option C: write the file yourself

```toml
[project]
name = "shop"

[services.db]
use = "postgres@16"
seed = "db/seed.sql"

[services.api]
run = "node server.js"
needs = ["db"]
env = { DATABASE_URL = "{{db.url}}", PORT = "{{api.port}}" }
health = { http = "/health", timeout = "120s" }
route = "api.localhost"
```

The full field list is in the [configuration reference](config-reference.md).
`use = "postgres@16"` selects the newest 16.x entry in the built-in registry;
`postgres@16.15` would pin the exact version. See the [registry guide](registry.md).

## Check the machine

```powershell
burrow doctor
```

`doctor` is read-only. It verifies the platform, cache location, project
configuration, trust state, secrets, cached packages, port availability, service
commands, and the proxy. Every failure includes a suggested fix. Add `--json`
for a machine-readable report. The command exits with status 1 if any check
fails, so CI can gate on it.

## First start

```powershell
burrow up
```

The first run in a project prints a trust summary before executing anything:

```text
Burrow will run commands from C:\dev\shop\burrow.toml in C:\dev\shop

  api: node server.js
  db: download and run `postgres@16` from the Burrow registry

Treat this like a Makefile or package.json script: it can run arbitrary code.
Approve and start? [y/N]
```

Type `y` to approve. The approval is stored in `.burrow/trust.json` and keyed by
a hash of the project name and each service's executable fields (`use`, `run`,
`cwd`). Changing a command or adding a service prompts again; changing ports or
memory limits does not. In CI, pass `--yes` explicitly. Without a terminal and
without `--yes`, Burrow refuses to start.

Normal startup output shows package verification, service state changes, health
checks, and then the live log stream. Once every service is healthy, open the
URLs:

```text
api  healthy  http://api.localhost
db   healthy  postgres://postgres@127.0.0.1:40001/postgres
```

Press `Ctrl+C` to stop every service gracefully, or run `burrow down` from a
second terminal.

### If `*.localhost` does not resolve

Some Windows configurations do not resolve `api.localhost` through the OS
resolver. Use the direct loopback URL instead; `burrow status` prints the
allocated port:

```text
http://127.0.0.1:40002
```

Or add one line to `C:\Windows\System32\drivers\etc\hosts` from an elevated
editor:

```text
127.0.0.1 api.localhost
```

Burrow cannot edit that file for you without elevation. The reverse proxy
itself works either way; the hosts entry only teaches the resolver the name.

## Everyday commands

| Command | What it does |
| --- | --- |
| `burrow up` | Start and stream logs in the foreground. |
| `burrow up --detach` | Start in the background and return to the prompt. |
| `burrow status` | Show state, ports, memory, CPU, and uptime. |
| `burrow logs api -f` | Follow one service's logs (`--since 10m` filters). |
| `burrow restart api` | Restart one service and its dependents. |
| `burrow open` | Open the first routed URL in the default browser. |
| `burrow down` | Stop everything and clean up runtime state. |
| `burrow top` | Open the live terminal dashboard. |
| `burrow env` | Print namespaced endpoint variables. |
| `burrow env api` | Print one service's complete environment. |
| `burrow exec api -- npm test` | Run a command with one service's environment and working directory. |

`burrow env` resolves planned or live ports and package paths without starting
anything or downloading a package. `burrow exec` ensures the selected service's
managed package is installed before running your command, so it can download on
first use.

## Data, snapshots, and reset

A managed service keeps its data under `.burrow/data/<service>/`. Initialization
runs once; the seed script also runs once, after the first successful health
check.

```powershell
burrow down
burrow snapshot before-migration
burrow up
# ... run a migration, find a problem ...
burrow down
burrow restore before-migration
burrow up
```

Snapshots are full copies of every configured service data directory. Data
commands require the daemon to be stopped so no file is being written. See the
[CLI reference](cli.md#data-commands) for the exact behaviour.

```powershell
burrow snapshots
burrow reset db
```

`burrow reset db` removes that service's data directory and its initialization
and seed markers. The next `burrow up` initializes a fresh cluster and runs the
seed script again.

## The dashboard

`burrow top` requires an interactive terminal. It refreshes once per second and
shows service state, CPU, memory, ports, uptime, and recent logs.

| Key | Action |
| --- | --- |
| `q`, `Esc`, `Ctrl+C` | Close the dashboard. Services keep running. |
| `Up` / `Down`, `j` / `k` | Move the selection. |
| `r` | Restart the selected service and its dependents. |

`burrow status` and `burrow logs` are the script-friendly alternatives; they do
not require a TTY.

## Per-machine overrides

Put local changes in `burrow.local.toml` next to `burrow.toml`:

```toml
[services.db]
memory = "1GB"

[services.api.env]
RUST_LOG = "debug"
```

Tables merge, scalars and arrays replace, and validation runs on the merged
result. Both `.burrow/` and `burrow.local.toml` belong in `.gitignore`.

## Clean up

```powershell
burrow down
```

Stopping removes runtime state but keeps `.burrow/data/` and snapshots. To remove
everything for one project, delete its `.burrow/` directory. Installed packages
live in a shared cache at `%LOCALAPPDATA%\burrow\cache` (or `BURROW_CACHE_DIR`).
Remove packages nothing references with:

```powershell
burrow pkg gc
```

Uninstall the binary with `cargo uninstall burrow-cli` if it was installed with
`cargo install`.

## Next

- [CLI reference](cli.md) — every command, flag, JSON shape, and exit code.
- [Configuration reference](config-reference.md) — every field and default.
- [Registry guide](registry.md) — how packages are verified and added.
- [Security and trust](security.md) — what the prompt does and does not protect.
- [Troubleshooting](troubleshooting.md) and [FAQ](faq.md).
- [Architecture](architecture.md) — how the pieces fit together.
