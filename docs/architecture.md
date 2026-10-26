# Architecture

This document describes the crate boundaries, the Windows supervisor design,
the package cache, and the networking and shell-environment layers. For current
status, see [`roadmap.md`](roadmap.md).

## Workspace layout

| Crate | Responsibility | Milestone |
| --- | --- | --- |
| `burrow-config` | Parse, validate, and template `burrow.toml`; merge local overrides; detect dependency cycles. | M2 |
| `burrow-pkg` | Registry entries, SHA-256-verified downloads, shared content-addressed cache, hardened archive extraction. | M3 |
| `burrow-supervisor` | Process graph, health checks, Windows Job Objects, restart policy, and log capture. | M1 core; M2 graph and policies |
| `burrow-net` | Stable port allocation, loopback reverse proxy, `*.localhost` routing with WebSocket passthrough. | M4 |
| `burrow-data` | Per-project data directories, one-time initialization and seeding, snapshots and restore. | M6 |
| `burrow-cli` | The `burrow` binary and its command surface. | M1 skeleton; handlers M2-M6 |
| `burrow-tui` | ratatui dashboard for live service state and logs. | M5 |

Only `burrow-cli` produces a user-facing binary. The `burrow-supervisor` package
also builds a test-only `burrow-testbin` binary target (not a product crate and
not shipped), which the supervisor integration tests spawn as a real child
process. Each library crate owns a narrow layer and is consumed by the CLI; the
crate-level docs describe the contracts.

## Supervisor design (M1)

One supervised service maps to one Windows Job Object.

### Job creation and race-free assignment

The supervisor creates a job with `CreateJobObjectW`, then configures a
`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` structure with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, a per-process memory limit, and a job-wide
memory limit. The job is assigned to the child at process creation time: the
supervisor builds a `STARTUPINFOEX` attribute list containing
`PROC_THREAD_ATTRIBUTE_JOB_LIST` and calls `CreateProcessW` with
`EXTENDED_STARTUPINFO_PRESENT`. Post-spawn `AssignProcessToJobObject` is not
used, because a child can spawn descendants before a separate assignment call
completes, leaving those descendants outside the job.

The child is also created with `CREATE_NEW_PROCESS_GROUP` so that console
control events can be addressed to its process group.

Bare program names such as `node` or `cmd` are resolved against `PATH` before
creation (using an environment override for `PATH` when one is configured).
`CreateProcessW` does not search `PATH` when `lpApplicationName` is set, so the
supervisor passes a resolved executable path. Relative program paths that
contain a directory are resolved against the configured working directory, or
against Burrow's current directory when no working directory is set.

A working directory of 260 UTF-16 units or more cannot be passed to
`CreateProcessW` directly, and the `\\?\` prefix is rejected in that argument
(and makes `cmd.exe` fall back to `C:\Windows`). Before creation, Burrow
converts such a directory to its 8.3 short name, removes the prefix, and uses
the short form for the process working directory and application path. The
project path in configuration, state, and output is unchanged. The same helper
is applied to `cmd` health checks, the detached daemon, `burrow exec`, the
browser launcher used by `burrow open`, and package steps. See
[ADR 0010](decisions/0010-long-path-process-working-directories.md).

### Limits

- `JobMemoryLimit` caps committed memory for the job as a whole.
- `ProcessMemoryLimit` adds a per-process cap.
- Optional CPU rate control can enforce a hard CPU cap when a service sets one.
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes Windows terminate any remaining
  processes when the last handle to the job closes, including when Burrow exits
  unexpectedly.

### Shutdown sequence

1. Send `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)` to the child's process
   group.
2. Wait for the configured grace period while the service exits on its own.
3. If the process tree is still alive, call `TerminateJobObject`.
4. Wait until the job is empty, then close the job handle.

Closing the job handle earlier would trigger kill-on-close immediately and skip
the graceful path. The detailed rationale and known limitations are recorded in
[ADR 0002](decisions/0002-job-object-lifecycle.md).

### Log forwarding

Child stdout and stderr are redirected through pipes. Tokio reader tasks drain
both streams continuously, forward complete lines to the per-service ring
buffer, and append them to rotating files under `.burrow/logs/`, so a full pipe
buffer cannot block the child.

## Configuration, secrets, and trust (M2)

`burrow-config` owns parsing, validation, local-override merging, dependency
graphs, and template expansion. `burrow-cli` loads the result, generates one
random password per service into `.burrow/secrets`, and expands
`{{service.port}}`, `{{service.url}}`, `{{service.password}}`, and project
variables before anything starts.

The first `burrow up` prints every executable entry and requires confirmation.
The approval is stored in `.burrow/trust.json` and keyed by
`Config::executable_hash`, which covers the project name, service names, and
each service's `run`, `use`, and `cwd` values. It does not cover `env`, health
checks, ports, routes, memory limits, or the contents of scripts. `--yes`
performs the same approval non-interactively for CI.

## Package cache (M3)

`burrow-pkg` embeds `registry/*.toml` at compile time. Each entry pins the
upstream URL, archive size, SHA-256, archive type, binary layout, run/stop
commands, connection URL, and initialization steps. Downloads resume from a
partial file, are hashed before use, and are extracted into a shared,
content-addressed cache under `%LOCALAPPDATA%\burrow\cache` (or
`BURROW_CACHE_DIR`).

Archive extraction rejects absolute paths, `..` traversal, symlinks, duplicate
or reserved names, and oversized entries. Initialization and seeding markers
live inside each service's data directory, so seed scripts run exactly once per
data directory even across restarts.

## Snapshots, restore, and reset (M6)

`burrow-data` implements snapshots as full directory copies, not database dumps.
`burrow snapshot <name>` copies every configured service's data directory into
`.burrow/snapshots/<name>/` and writes `manifest.json` with each service's
presence, file count, and byte count. The copy is built in a temporary directory
on the same volume and renamed into place only after every file and the manifest
have been written, so an interrupted snapshot never appears in
`burrow snapshots`. Snapshot names are restricted to 1-64 lowercase letters,
digits, `-`, `_`, and `.`, and must start with a letter or digit. Reserved
Windows device names such as `con` and `nul` are rejected, because a snapshot
name becomes a directory name. Reparse points inside a data directory are
rejected instead of followed. Hard links are copied as independent files, so
the snapshot preserves their contents but not the link relationship. A file
with a non-default NTFS alternate data stream is refused rather than copied
without that stream: `fs::copy` copies only the default stream, and silently
dropping data would make the snapshot incomplete.

`burrow restore <name>` stages every service's snapshot data first, then swaps
one service directory at a time with `rename`. If a later swap fails, directories
already replaced are rolled back, so each service is left with either its old
directory or the restored one, never a partial copy. A journal is written to
`.burrow/data/.burrow-restore-journal.json` before the first swap. If the
process dies mid-restore, the next `burrow up`, `snapshot`, `restore`, or
`reset` rolls an uncommitted restore back to the pre-restore directories; a
committed journal only needs its backups and staging directory cleaned up. A
staging directory with no journal is removed. A backup directory with no journal
is refused and left in place for inspection rather than guessed about. Services
that did not exist when the snapshot was taken are left untouched; a service
recorded as absent has its current data removed.

`burrow reset <service>` removes that service's data directory and its
initialization and seed markers. The next start initializes a fresh directory and
runs the seed script again. All three data commands require the project daemon to
be stopped: copying a live database directory can capture a torn state. Stale
state from a crashed daemon is authenticated before being treated as running.
Snapshots are not aware of database formats and do not protect against another
program writing to `.burrow/data/` at the same time.

A killed registry `init` can leave a non-empty data directory without the
initialization marker. Before retrying, Burrow moves that directory to
`.burrow/data/.burrow-init-failed-<service>-<pid>-<unix>` instead of deleting
it, so `initdb` starts clean and the partial output remains available for
inspection. PostgreSQL's seed command runs inside a single transaction
(`psql --single-transaction`), so a killed seed rolls back instead of leaving
partial rows. Rationale and limits: [ADR 0007](decisions/0007-crash-safe-data-operations.md).


## Networking and reverse proxy (M4)

`burrow-net` remembers one port per project in `.burrow/ports.json`. Automatic
ports are chosen from 40000-49999 and probed on `127.0.0.1` before reuse, so
URLs stay stable but never collide with another running service. Fixed ports are
validated as free. When the CLI needs values for a live daemon, it pins the
ports recorded in `.burrow/state.json` instead of re-probing them.

`burrow-net::proxy` is a hyper HTTP/1 proxy. It matches the `Host` header
against `route` values, opens a fresh loopback connection to the service, copies
the request, and streams the response back. For HTTP/1.1 upgrades (WebSocket) it
forwards the upgrade handshake and then uses `copy_bidirectional` to tunnel
frames in both directions.

The listener prefers port 80 so `http://shop.localhost` needs no suffix, and
falls back to a stable random port when 80 is unavailable. It binds
`127.0.0.1` unless a routed service sets `expose = true`; even then, requests
arriving from other machines are restricted to exposed routes. Burrow prints a
warning whenever the proxy binds all interfaces.

## Shell environment commands (M4)

`burrow env` resolves ports without downloading packages. Without a service
argument it prints project-level endpoint variables
(`BURROW_<SERVICE>_URL`, `_HOST`, `_PORT`, and `_ROUTE_URL` when routed) in
dotenv, JSON, or PowerShell form. With a service argument it prints that
service's complete resolved environment.

`burrow exec` resolves packages, then layers the service environment over the
caller's environment and runs the command in the service working directory.
Program names are resolved against the service `PATH` (which includes managed
package `bin` directories) and `.cmd`/`.bat` shims are executed through
`cmd.exe`. The child inherits the terminal, and its exit code becomes Burrow's
exit code.

`burrow open` reads live state, prefers the proxy route URL, and launches the
default browser. Setting `BURROW_NO_BROWSER` makes it print the URL instead,
which is useful in tests and remote shells.

## Experience commands (M5)

`burrow init` inspects the project directory and writes a starter `burrow.toml`.
It refuses to overwrite an existing file, and it validates the generated TOML
before creating anything. Detection is deliberately conservative:

- Node: reads `package.json`, prefers the `dev` script over `start`, and
  recognizes Vite, Next.js, and Express. Only Vite and Next receive explicit
  port flags, because those are the frameworks whose flags Burrow can verify.
  The package manager comes from `packageManager` or the lockfile.
- Python: looks for FastAPI or Flask instances in files up to two directories
  deep and emits `uvicorn module:variable` or `flask --app module`. Module names
  are computed relative to the project root, so a root `main.py` becomes
  `main:app`.
- Cargo and Go: recognize `Cargo.toml` + `src/main.rs` and `go.mod` +
  `main.go`.
- PostgreSQL: `--template postgres` writes a `db/seed.sql` placeholder and a
  service that uses the verified registry entry.

Unrecognized dev servers still get `PORT` in their environment, but no route or
health check, plus a comment explaining what to add after confirming that the
server honors the variable.

`burrow doctor` checks are read-only. They never download a package, generate a
secret, or bind a port. They cover the platform and elevation state,
`burrow.toml`, cache location and writability, trust approval, secrets, package
completeness, fixed ports, command resolution, daemon liveness and config drift,
proxy exposure, and supervised TCP listeners. Every failure includes a remedy;
`--json` emits the same report for scripts, and the command exits nonzero when
any check fails. Running elevated is reported as a warning because PostgreSQL's
Windows build refuses to initialize with an administrator token.

`--report` additionally writes a local, redacted diagnostic zip and prints its
path; nothing is uploaded. `--report-dir <DIR>` overrides the default
`.burrow/reports/` location and implies `--report`. The bundle contains the
report, an environment summary, redacted configuration and state, the daemon-log
tail, and up to ten recent crash notes. The CLI installs a panic hook that
writes a structured note under `%LOCALAPPDATA%\burrow\crashes` before chaining
to the previous hook; the hook only records, and `doctor` only reads. Redaction
and its limits are described in
[ADR 0009](decisions/0009-diagnostic-bundles.md).

`burrow top` requires an interactive terminal and refreshes once per second. It
reads the same authenticated control socket as `status` and `logs`, so it shows
live rather than stale state. The `burrow-tui` crate is rendering-only; all
terminal modes, key handling, and control traffic stay in the CLI. A guard
restores the console modes and exits the alternate screen on every return path,
including errors and panics.

Windows command lookup follows `PATHEXT` order before considering an
extensionless file. This matches `cmd.exe` and avoids selecting an npm Unix shell
shim instead of `npm.cmd`. `.cmd` and `.bat` targets run through `cmd.exe`.

## Idle stop and lazy wake-up (Phase 2)

A service with `idle_stop` is stopped after it has had no established TCP
connections for the configured duration, and started again on the next
connection. The design and its trade-offs are recorded in
[ADR 0003](decisions/0003-idle-stop-and-lazy-wake.md); the mechanics in brief:

- The daemon samples `GetExtendedTcpTable` (wrapped in `burrow-supervisor`'s
  `win` module) every two seconds, counting system-wide connections whose
  local port is the service's port. Sampling starts only after the project is
  fully up and the service is healthy.
- Stopping is the same graceful path `burrow down` uses (package stop command,
  CTRL_BREAK, terminate). The daemon then binds the port with a wake listener
  so no other process can take it while the service sleeps.
- A direct client that connects wakes the service: the listener accepts one
  client, drops the listen socket, and the daemon parks the client until the
  service is healthy, then splices it with `copy_bidirectional`. The client
  speaks to the real service; Burrow never interprets the protocol.
- Routed services wake through the proxy: an `ProxyWake` channel shared with
  the daemon lets the proxy hold a request until the upstream is healthy, so
  HTTP clients see a real response or an explicit error instead of a refused
  connection.
- `burrow status` shows idle services as `idle` with no PID; a wake resets
  crash backoff and is counted as a restart.

## File watching (Phase 2)

`watch` roots are monitored with one `ReadDirectoryChangesW` thread per root
(overlapped I/O plus a stop event, in the supervisor's `win` module). The
daemon polls events on its existing tick, maps them to services through the
configured roots, filters anything under a `.burrow` or `.git` component, and
restarts only the watched service after a debounce window (quiet for 500 ms,
at most 2 s). Rationale: [ADR 0004](decisions/0004-file-watching.md).

## Container fallback (Phase 2)

`use = "container:IMAGE"` delegates a service to Docker or Podman (found via
`BURROW_CONTAINER_ENGINE`, `docker`, `podman`). The supervised process is the
engine client; the port mapping is loopback-only; stops go through
`<engine> stop` and every start clears a leftover container first. Container
services are labelled in `burrow status` (RUNTIME column and JSON), cannot be
seeded or snapshotted, and run with an ephemeral filesystem. Design and known
limitations: [ADR 0005](decisions/0005-container-fallback.md).

## Safety rules

- Product `unsafe` is confined to each crate's private `win` module
  (`burrow-supervisor` and `burrow-cli`). No other module contains `unsafe`.
- Every `unsafe` block carries a `// SAFETY:` comment that explains why its
  preconditions hold.
- Win32 handles are owned by wrappers in the `win` modules and cannot escape as
  raw handles through the safe public API.
- Public functions validate inputs and translate Win32 failures into errors;
  expected failure paths do not panic.
- The supervisor's test-only support module uses a few Win32 query calls to
  inspect process liveness and job membership from the outside; those calls
  carry SAFETY comments too. This is a review-enforced rule, backed by clippy
  with `-D warnings` in CI.
- Generated secrets are written to `.burrow/secrets` as plaintext JSON readable
  by the Windows account. There is no custom ACL or encryption. Secrets are
  excluded from logs, status tables, and command summaries; only
  `burrow env <service>` prints them, because that command exists to export a
  service environment.
- Network listeners bind loopback by default. Remote proxy access requires an
  explicit per-service `expose = true` and is limited to those routes.

## Testing strategy

- Unit tests cover pure logic: configuration validation, dependency ordering,
  port allocation, proxy host routing and upgrade tunneling, and extraction path
  checks.
- CLI integration tests spawn the built `burrow` binary with
  `std::process::Command` and assert exit codes and stderr.
- Windows supervisor integration tests spawn real children through
  `burrow-testbin` inside Job Objects. They exercise memory-cap enforcement,
  kill-on-close, graceful shutdown ordering, and the forced-termination fallback.
- Tests that exercise console control events run with `--test-threads=1` because
  console events are process-wide. CI enforces this.
- Phase-2 integration tests live in `crates/burrow-cli/tests/phase2.rs`: an
  idle-stop/wake round trip against a real listener, watch restarts (including
  the `.burrow` exclusion), the container preflight failure, and a live
  container round trip that skips when no engine responds.
- CI on `windows-latest` runs rustfmt, clippy (`-D warnings`), the workspace test
  suite, and a release build, then uploads `burrow.exe`. A separate job runs
  `cargo audit` and `cargo deny check`.
- The release workflow runs `cargo deny check`, builds with `--locked`, generates
  a CycloneDX 1.5 SBOM with `cargo-cyclonedx`, and hashes both the zip and the
  SBOM. `scripts/release-dry-run.ps1` rehearses that locally without tagging,
  signing, or publishing.
- `scripts/check.ps1` mirrors the format, clippy, and test checks locally.

## Resource limits are not a security boundary

Job Objects help contain accidental resource use and ensure child processes are
cleaned up. They do not isolate services from one another or from the user's
account. Every service runs with the same access to files, the network, and the
desktop as the user who started Burrow, and can read or modify anything that user
can. Do not use Burrow to run untrusted code.
