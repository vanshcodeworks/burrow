# CLI reference

`burrow` is one executable. Run it from a project directory, or pass
`--project <DIR>` from anywhere.

```text
burrow [--project <DIR>] [--json] [-v]... <COMMAND>
```

| Global flag | Meaning |
| --- | --- |
| `--project <DIR>` | Directory containing `burrow.toml`. Defaults to the current directory. |
| `--json` | Machine-readable output where the command supports it. Ignored by commands that do not. |
| `-v`, `-vv` | Increase tracing detail (`debug`, then `trace`). Tracing goes to stderr. |
| `--version` | Print the Burrow version. |

`NO_COLOR` disables colored status output. `BURROW_NO_BROWSER` makes
`burrow open` print the URL instead of starting a browser.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The command completed successfully. |
| `1` | The command failed: invalid configuration, unreachable daemon, download or hash failure, a failing `doctor` check, or `pkg verify` problems. |
| `2` | Command-line usage error from clap: unknown command or flag, missing required argument. |
| child code | `burrow exec` propagates the exact exit code of the child process. If the child has no numeric code (rare on Windows), Burrow uses `1`. |

Errors are written to stderr in the form `error: ...`, often followed by a
`help:` line. Human output goes to stdout.

## Commands

| Command | Purpose | JSON |
| --- | --- | --- |
| `init` | Scaffold `burrow.toml`. | yes |
| `up [SERVICE...]` | Start services. | `--detach` only |
| `down` | Stop the project. | yes |
| `restart SERVICE` | Restart a service and dependents. | yes |
| `status` | Show runtime state and metrics. | yes |
| `logs [SERVICE]` | Print or follow logs. | yes |
| `open [SERVICE]` | Open a routed URL. | no |
| `env [SERVICE]` | Print resolved environment. | `--format json` |
| `exec SERVICE -- CMD...` | Run a command in a service environment. | no |
| `doctor` | Diagnose the machine and project. | yes |
| `top` | Live terminal dashboard. | no |
| `pkg` | Inspect and manage the package cache. | most |
| `snapshot NAME` | Copy service data. | yes |
| `restore NAME` | Restore service data. | yes |
| `snapshots` | List snapshots. | yes |
| `reset SERVICE` | Delete one service's data. | yes |
| `schema` | Print the `burrow.toml` JSON Schema. | JSON by nature |

## Project setup

### `burrow init [--template NAME]`

Writes `burrow.toml` in the project directory. It never overwrites an existing
file and creates no extra files that would overwrite yours. Without
`--template`, it detects, in order: Node.js (`package.json` with a dev or start
script), Python (`main.py` or `app.py`), Cargo (`Cargo.toml` + `src/main.rs`),
Go (`go.mod`). Named templates are `node`, `python`, `postgres`, `cargo`, and
`go`.

The generated configuration is parsed and validated before any file is written.
Human output lists the services and the next commands; `--json` returns the
template, summary, project name, directory, config path, services, and created
files.

### `burrow schema`

Prints the JSON Schema for `burrow.toml` to stdout. The checked-in copy is
[`burrow.schema.json`](burrow.schema.json). Regenerate it with:

```powershell
burrow schema > docs\burrow.schema.json
```

## Running services

### `burrow up [SERVICE...] [--detach] [--yes]`

Loads and validates the configuration, requires trust approval, warms the
package cache, and starts the daemon.

- With no service names, starts every service.
- With names, starts those services plus their transitive `needs`
  dependencies, and nothing else.
- In the foreground, streams logs and stops services on `Ctrl+C`.
- `--detach` starts a background daemon, waits until it is reachable, and
  prints the service list. Use `--json` with `--detach` for JSON output.
- `--yes` approves the current executable configuration without prompting. Use
  it only in an environment where you have already reviewed the file: CI, a
  disposable container, or a scripted demo.

Startup is layered. Each layer starts in parallel; the next layer waits until
every configured health check in the previous layer passes. A service without a
health block counts as ready when its process starts.

While the daemon runs, starting the same project again fails with a message
pointing at `burrow status` or `burrow down`. Different projects can run at the
same time; their ports and runtime state are independent.

### `burrow down`

Asks the daemon to stop. Services are stopped in reverse dependency order. A
managed package with a `stop` command gets that command first (for PostgreSQL,
`pg_ctl stop -m fast`), then the process tree is shut down with the configured
grace period and the Job Object is closed. The command waits up to 120 seconds
for the daemon to exit and removes runtime state.

If no daemon is running, `down` succeeds and removes stale state if a previous
daemon left some behind. With `--json`, the result says whether the project was
running.

### `burrow restart SERVICE`

Restarts one service and its dependents through the running daemon, in
dependency order. The manual restart resets that service's crash-loop breaker.
The command waits up to 120 seconds for the daemon to acknowledge.

### `burrow status`

Prints the project, daemon pid, control port, and one row per service: state,
port, working-set RAM, CPU, uptime, and URL. A failed service also prints its
last error.

`--json` prints the public state document:

```json
{
  "version": 1,
  "project": "shop",
  "project_dir": "C:\\dev\\shop",
  "pid": 12345,
  "control_port": 40000,
  "proxy_port": 80,
  "started_at": 1760000000,
  "config_hash": "…",
  "ready": true,
  "services": {
    "api": {
      "status": "healthy",
      "pid": 12346,
      "port": 40002,
      "url": "http://127.0.0.1:40002",
      "route": "api.localhost",
      "route_url": "http://api.localhost",
      "restarts": 0,
      "started_at": 1760000001,
      "exit_code": null,
      "memory_bytes": 73400320,
      "cpu_percent": 0.4,
      "last_error": null
    }
  }
}
```

Service states are `starting`, `healthy`, `restarting`, `stopped`, and `failed`.
The IPC bearer token is never included.

When the daemon is not running, `status` exits 1 with a hint to run `burrow up`.
With `--json` and no state file, it prints `{"running": false}` and exits 0; with
stale state it prints `{"running": false, "stale": true, ...}` and also exits 0.

### `burrow logs [SERVICE] [-f|--follow] [--since DURATION]`

Prints buffered log lines captured by the daemon. `SERVICE` restricts the output;
`--since 10m`, `--since 1h`, and other `humantime` durations filter by timestamp.

`--follow` keeps the connection open until `Ctrl+C`. Lines are printed as
`service` followed by the text; stderr lines are red. With `--json`, the
initial batch is a JSON array and, when following, each new line is printed as
one JSON object per line.

Logs are held in memory by the daemon's bounded ring buffer and are lost when
the daemon stops. `burrow up --detach` also writes daemon tracing to
`.burrow/daemon.log`; service output is available through `burrow logs` while
the daemon is alive.

### `burrow open [SERVICE]`

Opens a routed URL in the default browser. With a service name, opens that
service's route, falling back to its HTTP endpoint if it has no route. Without a
name, opens the first service (alphabetically) that has a route.

If the project is not running, or no service has a route, the command fails and
explains what to add. Set `BURROW_NO_BROWSER=1` to print the URL instead.

### `burrow env [SERVICE] [--format dotenv|json|powershell]`

Without a service, prints the project endpoint view:

```text
BURROW_PROJECT=shop
BURROW_PROJECT_DIR=C:\dev\shop
BURROW_API_HOST=127.0.0.1
BURROW_API_PORT=40002
BURROW_API_URL=http://127.0.0.1:40002
BURROW_API_ROUTE_URL=http://api.localhost
```

Service names are uppercased and every non-alphanumeric character becomes `_`.
The route URL is present only when the service has a route and the proxy port is
assigned.

With a service, prints that service's complete environment: configured `env`,
generated `BURROW_*` variables, `PATH` with package binaries prepended, and the
generated secret in `BURROW_SERVICE_PASSWORD`. This is the one command that
prints secrets, and only because you asked for a specific service.

`--format` controls the syntax. `dotenv` (default) prints `KEY=value` and quotes
only values that need it. `json` prints one JSON object. `powershell` prints
`$env:KEY = 'value'` assignments with single quotes escaped.

`burrow env` resolves planned or live ports and package paths but never
downloads a package. If a managed package has never been installed, its path is
reported as it will be after installation.

### `burrow exec SERVICE -- COMMAND [ARG...]`

Runs a command with the service's resolved environment, `PATH`, and working
directory. Everything after `--` belongs to the child. The child inherits
Burrow's standard handles and current environment, with the service variables
layered on top.

```powershell
burrow exec api -- node --version
burrow exec db -- psql --version
```

`burrow exec` ensures the service's managed package is installed, so it may
download a package on first use. It propagates the child's exit code. Use it for
one-off tasks that need the same environment as a service, such as migrations or
tests.

## Diagnostics and monitoring

### `burrow doctor`

Runs read-only checks and exits nonzero if any check fails. It never downloads,
generates a secret, or binds a port. Checks include:

- `platform`: Windows and x86-64.
- `session`: warns when the terminal is elevated, because PostgreSQL refuses to
  initialize as administrator.
- `config`: the project file parses and validates.
- `cache`: the shared cache is discoverable and writable.
- `daemon`: a running daemon answers, or stale state is reported.
- `trust`, `secrets`, `packages`, `ports`, `commands`, `proxy`, `listeners`:
  project-specific readiness checks. The `listeners` check inspects the TCP
  sockets owned by supervised process trees and warns when a service listens on
  a non-loopback address without `expose = true`.

`--json` emits `{ "ok": bool, "project", "project_dir", "summary": {"ok",
"warnings", "failures"}, "checks": [...] }`. Each check has `name`, `status`
(`ok`, `warn`, `fail`), `detail`, and an optional `remedy`.

### `burrow top`

Opens the dashboard. Requires an interactive terminal; in a pipe or CI it fails
with a hint to use `status` or `logs -f` instead. It refreshes once per second.

| Key | Action |
| --- | --- |
| `q`, `Esc`, `Ctrl+C` | Close the dashboard without stopping services. |
| `Up` / `Down`, `j` / `k` | Change the selected service. |
| `r` | Restart the selected service and its dependents. |

## Package cache

The cache lives at `%LOCALAPPDATA%\burrow\cache` unless `BURROW_CACHE_DIR` is
set. It is shared by every project. See the [registry guide](registry.md) for the
layout and verification rules.

### `burrow pkg list`

Lists installed packages, versions, archive sizes, and short hashes. `--json`
adds the cache root, full SHA-256 values, install times, paths, and total bytes
on disk.

### `burrow pkg add NAME[@VERSION] [--service NAME] [--seed PATH]`

Resolves the reference, downloads and verifies the package, then adds a service
table to `burrow.toml` with `use = "NAME@VERSION"` and the optional seed path.
The service name defaults to the package name. It refuses to replace an existing
service and validates that the seed file is inside the project and exists.
`--json` prints the service name, canonical package key, and config path.

The command edits `burrow.toml` as TOML; comments and formatting outside the
inserted table are preserved by `toml_edit`.

### `burrow pkg remove NAME`

Removes a package-backed service from `burrow.toml`. `NAME` can be the service
name or the package name. It refuses while the daemon is running, refuses for a
command-based service, and refuses to remove the only service. The data
directory and cached files remain. Use `burrow reset SERVICE` for data and
`burrow pkg gc` for cache entries.

### `burrow pkg gc`

Deletes cached archives and extracted packages that no known project references.
It also drops stale project reference files for directories that no longer
exist. `--json` reports packages removed, archives removed, stale references
removed, and bytes freed.

### `burrow pkg verify`

Re-hashes every cached archive and checks every extracted package marker. It
prints each problem and exits 1 if anything is wrong; `--json` reports `ok`,
counts, and the problem list.

## Data commands

All four data commands require the project daemon to be stopped. A reliable
control-socket handshake decides whether it is running; a stale state file does
not block the command.

### `burrow snapshot NAME`

Copies every configured service's data directory into
`.burrow/snapshots/NAME/` and writes `manifest.json`. A service with no data is
recorded as `present = false`. The copy is built in a temporary directory on the
same volume and renamed into place only after it succeeds, so a failed snapshot
never appears in `burrow snapshots`. Snapshots are full copies, not incremental
or deduplicated, and the name must be 1-64 characters of lowercase letters,
digits, `-`, `_`, or `.`, starting with a letter or digit. Reserved Windows
device names such as `con` and `nul` are rejected.

`--json` prints the manifest: `version`, `name`, `created_at`, and a `services`
array with `name`, `present`, `files`, and `bytes`.

### `burrow snapshots`

Lists snapshots newest first. The table shows name, UTC creation time, size, and
which services had data. `--json` prints the manifest array.

### `burrow restore NAME`

Restores every service recorded in the snapshot manifest. Services added to
`burrow.toml` after the snapshot are left untouched. A service recorded as
`present = false` has its current data removed. Restore stages all data first,
then swaps one service directory at a time and rolls back already-swapped
directories if a later swap fails.

A journal written before the first swap makes the operation crash-safe. If the
process is killed mid-restore, the next `burrow up`, `snapshot`, `restore`, or
`reset` rolls the uncommitted restore back to the pre-restore data (or finishes
cleanup for a committed restore) and prints a `note:` line. A staging directory
without a journal is removed; a backup directory without a journal is refused
and left for inspection. Do not delete `.burrow/data/.burrow-backup-*` or
`.burrow-restore-journal.json` by hand while diagnosing a crash.

`--json` prints `{ "name": "...", "services": ["..."] }`. Run `burrow up`
afterwards to start from the restored data.

Limitation: snapshots are plain directory copies. They do not understand
database formats. A stopped PostgreSQL cluster is safe to copy on Windows, but a
running one is not, which is why the daemon must be stopped. Snapshot integrity
depends on no other program writing to `.burrow/data/` at the same time.

### `burrow reset SERVICE`

Removes `.burrow/data/SERVICE/`, including the initialization and seed markers.
The next `burrow up` initializes a fresh data directory and runs the seed script
again. `SERVICE` may be any configured service; command-based services get an
empty data directory too. `--json` prints `ok`, `service`, `data_dir`, and
`reset`.

## Internal command

`burrow __daemon [SERVICE...]` is hidden and used by `burrow up --detach`. It is
not a supported user entry point. It refuses to start if the project is not
already trusted.

## Environment variables read by Burrow

| Variable | Effect |
| --- | --- |
| `BURROW_CACHE_DIR` | Override the package cache location. |
| `BURROW_NO_BROWSER` | `burrow open` prints the URL instead of opening a browser. |
| `NO_COLOR` | Disable colored `status` output. |
| `RUST_LOG` | Override tracing filters. `-v` sets the default when this is unset. |

Managed services inherit the environment of the process that started the daemon,
plus their configured and generated variables. Launch Burrow from the shell whose
runtime you want services to use, such as an activated Python virtual
environment.
