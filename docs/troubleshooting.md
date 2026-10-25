# Troubleshooting

Start with the two read-only diagnostics:

```powershell
burrow doctor
burrow logs
```

`burrow doctor` checks the platform, elevation, configuration, cache, trust,
secrets, packages, fixed ports, command resolution, daemon liveness, and proxy
exposure. Every failure includes a remedy. `--json` produces the same report for
scripts and exits `1` when any check fails.

`burrow logs` reads the authenticated control socket, so it shows live output
only while the project daemon is running. The raw files are also on disk:

| File | Contents |
| --- | --- |
| `.burrow/logs/<service>.log` | Rotating stdout/stderr for one service |
| `.burrow/daemon.log` | Daemon startup, health, restart, and proxy messages |

## `http://something.localhost` does not resolve

The reverse proxy routes by the HTTP `Host` header, but Windows still has to
resolve the name before the request reaches the proxy. Some Windows and DNS
configurations resolve `*.localhost`; others do not.

Two workarounds work everywhere:

1. Use the service's direct loopback URL. `burrow status` prints each service's
   `url` and port; open `http://127.0.0.1:<port>`.
2. Add the route names to `C:\Windows\System32\drivers\etc\hosts` (requires an
   elevated editor):

   ```text
   127.0.0.1 api.localhost
   127.0.0.1 shop.localhost
   ```

The proxy prefers port 80 so `http://api.localhost` needs no port suffix. When
port 80 is unavailable it picks a stable random port; `burrow status` shows the
proxy port, and `burrow open` prints the exact URL.

`burrow env` exposes `BURROW_<SERVICE>_ROUTE_URL` only when the route can be
built, so it is a convenient way to discover the real URL:

```powershell
burrow env --format powershell
```

If the browser does not start, set `BURROW_NO_BROWSER=1` and `burrow open` will
print the URL instead.

## "Burrow is already running for this project"

The daemon for this project is alive. Inspect it with:

```powershell
burrow status
burrow logs -f
```

Stop it with `burrow down`, or use the existing instance. The "already starting"
variant means `burrow up` is still waiting for health checks; let it finish or
stop it with `burrow down`.

## "Burrow is not running ... but stale state remains"

The daemon exited without cleaning `.burrow/state.json`. `burrow down` verifies
whether the recorded control socket answers, and if not, removes the stale
document:

```powershell
burrow down
```

If the state file exists but the control socket rejects the token, Burrow reports
that another process may be using the recorded port. Check for a second copy of
the project, a leftover `burrow.exe`, or an unrelated program bound to the same
port, then run `burrow down` again.

## Port conflicts

A fixed port that is occupied produces:

```text
port 5432 is already in use; choose another port or use `port = "auto"`
```

Change the service's `port` or set `port = "auto"`. Automatic allocation scans
the range 40000-49999 and remembers assignments in `.burrow/ports.json`. If the
range is exhausted the error is:

```text
could not find a free port in 40000-49999; free a port or set one explicitly
```

That usually means thousands of listeners, or a firewall/security product that
holds loopback ports. Closing stale processes or rebooting clears it.

If the reverse proxy cannot bind its port, startup fails with:

```text
could not bind the reverse proxy on 127.0.0.1:<port>
```

Something else is listening on the proxy port. Stop it, or restart Burrow so it
chooses a different fallback port. When any routed service sets `expose = true`
the proxy binds all interfaces instead of loopback, and the daemon log records a
warning.

## Proxy error pages

| Status | Body starts with | Meaning |
| --- | --- | --- |
| `400` | `Burrow proxy: this request has no Host header.` | The client did not send HTTP/1.1 `Host`. Use a normal browser or `curl`. |
| `404` | `Burrow proxy: no service is routed to ...` | Add `route = "<name>.localhost"` to a service, or use the direct port. |
| `404` | `Burrow proxy: ... is not exposed to other machines.` | The request came from a non-loopback address and the route lacks `expose = true`. |
| `502` | `Burrow proxy: ... is not accepting connections on 127.0.0.1:<port>` | The service process is down or not listening yet. Check `burrow status` and the service log. |
| `502` | `Burrow proxy: ... failed to answer` | The service accepted the connection but closed or malformed the response. |
| `504` | `Burrow proxy: timed out connecting to ...` | The TCP connect to the service timed out. Check for a hung process or firewall rule. |

A `404` for a name that should exist usually means the route is missing from
`burrow.toml`, or the daemon is running an older configuration. Stop and start
again with `burrow down` then `burrow up`.

## Package downloads and verification

Downloads resume from `<sha256>.<ext>.part`. A short body is reported as:

```text
download of <url> was incomplete: expected <n> bytes, got <m>
```

Run the command again to resume. If the server does not support range requests,
Burrow restarts the download.

A corrupted archive produces:

```text
SHA-256 mismatch for <url>: expected <hash>, got <hash>; the download was deleted
```

The next run downloads it again. To check the whole cache:

```powershell
burrow pkg verify
```

`pkg verify` re-hashes cached archives and validates package markers. It reports
corrupt archives and packages with missing or invalid markers, and exits `1` if
there are problems. Remove the affected entries with `burrow pkg gc` (which
drops packages no project references) or delete the specific cache directory.
The default cache is `%LOCALAPPDATA%\burrow\cache`; `BURROW_CACHE_DIR` overrides
it.

A 30-minute lock wait means another Burrow process is downloading the same
archive. Let it finish rather than deleting the lock file. Stale locks older
than one hour are reclaimed automatically.

The registry is compiled into `burrow.exe`. Adding a package requires editing
`registry/<name>.toml` and rebuilding; `burrow pkg add` only selects an existing
entry. See [`registry.md`](registry.md).

## A command is not found or starts the wrong program

Burrow resolves bare names against the service `PATH`, trying exact files and
then `.exe`, `.cmd`, and `.bat`. Batch shims run through `cmd.exe`. Check what
the service will actually see:

```powershell
burrow env <service> --format powershell
burrow exec <service> -- where node
```

Common causes:

- **The runtime is not installed or not on `PATH`.** Burrow does not install
  Node, Python, Go, or Rust. Install them and open a new terminal.
- **A virtual environment is not active.** Burrow starts the configured command
  directly. Either activate the venv before `burrow up`, use an absolute
  interpreter path, or put the venv's `Scripts` directory on the service `PATH`.
  The daemon inherits the environment of the terminal that started it, so
  `burrow down` and `burrow up` after activating the venv.
- **A package manager shim was picked instead of the Windows shim.** Burrow
  follows `PATHEXT`-style resolution (`.exe`, `.cmd`, `.bat`) and does not run
  Unix shell shims without an extension. Use `npm.cmd` explicitly if a wrapper
  is shadowing it.
- **The command relies on shell syntax.** `run` is not a shell command line.
  Use `cmd /C "..."` or `powershell -NoProfile -Command "..."` when you need
  redirection, pipes, or `&&`.

The spawn error names the program:

```text
could not start `node`: The system cannot find the file specified.
```

## PostgreSQL initialization fails

PostgreSQL's Windows build refuses to run `initdb` with an elevated
(administrator) token. Run Burrow from a normal terminal. `burrow doctor`
reports elevation as a warning before you start.

If a previous attempt left a partial data directory, the init marker is absent
and the next `burrow up` retries initialization. If the directory is corrupt or
owned by another account, reset just that service:

```powershell
burrow down
burrow reset db
burrow up
```

`burrow reset` deletes `.burrow/data/<service>` and is irreversible. Take a
snapshot first if the data matters. PostgreSQL also writes its own log inside
the data directory; check `.burrow/data/db/log/` when the service starts but
health checks fail.

## "service `name` did not become healthy"

The process started but the configured health check never passed within the
timeout (default `60s`). Burrow prints the last health error and points at the
service log and daemon log.

Check, in order:

1. `burrow logs <service>` — did the process print a startup error?
2. The health check itself: an HTTP check accepts any non-5xx response, a TCP
   check only succeeds when `tcp = true` is set, and a command check requires
   exit code 0.
3. The port: a service that ignores `PORT` and listens elsewhere will never
   satisfy a check against the allocated port.
4. Startup time: a slow first build may need a larger `health.timeout`.

Example health settings are in [`config-reference.md`](config-reference.md).

## A service keeps restarting or stopped after "crash loop"

The default restart policy is `on-failure`. Backoff starts at 500 ms and doubles
to 30 seconds. Five failures within 60 seconds trip the crash-loop breaker, and
the service is marked failed instead of restarting forever. Thirty seconds of
stable uptime clears the failure history.

Fix the underlying error, then:

```powershell
burrow restart <service>
```

A manual restart resets the breaker. `restart = "never"` disables automatic
restarts; `restart = "always"` restarts even after a clean exit.

## Data commands refuse to run

`snapshot`, `restore`, and `reset` require the daemon to be stopped:

```text
services are running for this project; data operations need them stopped
help: run `burrow down`, then retry
```

This is deliberate: the snapshot is a plain directory copy, and copying a live
database directory can capture a torn state. Stale state from a crashed daemon
does not block the command, because Burrow authenticates against the control
socket before refusing.

Other snapshot errors:

- **Invalid name.** Names are 1-64 characters, start with a lowercase letter or
  digit, and contain only lowercase letters, digits, `-`, `_`, and `.`. Path
  separators, drive prefixes, and `.`/`..` are rejected.
- **Snapshot already exists.** Choose another name; Burrow does not overwrite.
- **Reparse point found.** Snapshots reject symlinks, junctions, and other
  reparse points instead of following them, so data outside the project cannot
  be captured silently. Remove the link from the data directory or snapshot a
  service that does not contain it.
- **Missing or invalid manifest.** `burrow snapshots` reports broken snapshot
  directories rather than hiding them. Delete the directory if it is not needed.
- **Restore failed.** Restore stages every service first, then swaps directories
  one at a time and rolls back services already replaced. The project is left
  with either the old or the restored directory for each service, never a
  partial copy. Check free disk space and file locks first.

## `burrow top` refuses to start

```text
`burrow top` needs an interactive terminal
help: use `burrow status` or `burrow logs -f` in scripts and CI
```

Run it from Windows Terminal, PowerShell, or another console. Redirected output
and CI jobs are not supported. If the dashboard is left in a strange state after
a crash, open a new terminal; the normal exit path restores the console mode and
leaves the alternate screen.

## Build and toolchain problems

- **`link.exe` not found / MSVC errors.** Install Visual Studio Build Tools and
  select "Desktop development with C++", including the Windows SDK.
- **Rust is too old.** The workspace MSRV is 1.85; CI uses stable. Run
  `rustup update stable`.
- **Access denied or file-in-use errors under `target\`.** Antivirus and file
  indexers can hold freshly built binaries. Exclude the repository from
  real-time scanning, or run `cargo clean` and rebuild.
- **`cargo install --path crates\burrow-cli --locked` fails.** Run it from the
  repository root, and make sure the same MSVC toolchain used for the build is
  on `PATH`. The installed binary lands in `%USERPROFILE%\.cargo\bin`.
- **A package download is slow or blocked.** Burrow uses HTTPS to the upstream
  URL recorded in the registry. A proxy must be configured at the system level;
  Burrow has no proxy settings of its own.

## Reporting a bug

Collect the following before opening an issue:

```powershell
burrow --version
burrow doctor --json
burrow status --json
```

Include the `burrow.toml` with secrets removed, the exact command, the expected
and actual behavior, and the relevant part of `.burrow/logs/` or
`.burrow/daemon.log`. Never paste `.burrow/secrets`, `BURROW_SERVICE_PASSWORD`,
or a connection URL containing a password.
