# FAQ

## What is Burrow?

Burrow is a local development environment manager for Windows. You describe
services in `burrow.toml`, and `burrow up` allocates ports, downloads and
verifies managed packages, starts native Windows processes in dependency order,
waits for health checks, and streams logs. It is not a container runtime, a
virtual machine, or a production deployment tool.

## Is Burrow a Docker replacement?

For projects whose services have trustworthy native Windows builds, it removes
the container layer: no Docker daemon, no VM, no WSL distribution, and no image
builds. It is not a drop-in replacement in general. Containers provide
filesystem and network isolation, reproducible base images, and Linux binaries;
Burrow provides none of those. See the fair comparison in
[`benchmarks.md`](benchmarks.md) and the limitations in
[`security.md`](security.md).

## Does Burrow work on Linux or macOS?

No. The supervisor uses Windows Job Objects, console control events, and Win32
process creation directly. There is no cross-platform backend.

## Does Burrow install Node, Python, Go, or Rust?

No. Burrow starts the command you configure; language runtimes must already be
installed and on `PATH` (or referenced by absolute path). The only packages
Burrow downloads are entries in its embedded registry. At present those are
PostgreSQL 16.15, NATS 2.15.1, and Caddy 2.11.7.

## Do I need to run Burrow as administrator?

No, and you should not. PostgreSQL's Windows build refuses to initialize a
cluster with an elevated token. `burrow doctor` warns when the terminal is
elevated. Run Burrow from a normal user terminal.

## Does Burrow need an internet connection?

The first start of a managed package downloads its archive from the upstream URL
in the registry. After verification the archive and extracted files live in the
shared cache, so later starts work offline. Your own services may still need the
network for `npm install`, `pip install`, and similar commands.

The registry itself is compiled into `burrow.exe`; Burrow never fetches registry
metadata at runtime.

## Where does Burrow store data?

| Path | Contents |
| --- | --- |
| `.burrow/data/<service>/` | Per-project service data, including the PostgreSQL cluster |
| `.burrow/snapshots/<name>/` | Full directory copies plus `manifest.json` |
| `.burrow/logs/<service>.log` | Rotating service logs |
| `.burrow/daemon.log` | Daemon log |
| `.burrow/state.json` | Live daemon pid, control port, token, and service snapshot |
| `.burrow/secrets` | Generated per-service passwords (plaintext JSON) |
| `.burrow/trust.json` | Approval hash for the executable configuration |
| `.burrow/ports.json` | Remembered automatic port assignments |
| `%LOCALAPPDATA%\burrow\cache` | Shared package cache (`BURROW_CACHE_DIR` overrides) |

`burrow down` stops processes but does not delete data. `burrow reset <service>`
deletes that service's data directory; `burrow snapshot` and `burrow restore`
copy it.

## Are generated secrets encrypted?

No. `.burrow/secrets` is plaintext JSON readable by your Windows account. It is
excluded from normal logs and status output, but `burrow env <service>` prints
the service's full environment, including `BURROW_SERVICE_PASSWORD` and any
connection URL containing it. Do not commit `.burrow/`, and do not paste
`burrow env <service>` output into issues. See
[`security.md`](security.md).

## How does the trust prompt decide when to ask again?

Approval is keyed by a SHA-256 hash of the project name, service names, and each
service's `use`, `run`, and `cwd` values. Changing a command, package, working
directory, or the set of services re-prompts. Changing `env`, health checks,
ports, routes, memory limits, or the contents of scripts does **not** re-prompt.
The exact coverage is documented in [`security.md`](security.md).

## Why does `burrow env` not print `DATABASE_URL`?

Without a service argument, `burrow env` prints project-level endpoint variables
(`BURROW_<SERVICE>_URL`, `_HOST`, `_PORT`, and `_ROUTE_URL`) so scripts can
discover ports without receiving credentials. Run `burrow env <service>` to get
that service's complete resolved environment, including `DATABASE_URL` and
generated passwords.

`burrow env` never downloads packages. `burrow exec` may download them because
it must materialize the service environment and managed binaries.

## Can I use an existing PostgreSQL server instead of the managed one?

Yes. Leave the `db` service out of `burrow.toml`, set `DATABASE_URL` to your
existing server, and start only the services you need:

```powershell
burrow up api
```

Burrow only manages the services listed in the configuration and selected on the
command line. It does not discover or adopt processes it did not start.

## Does Burrow support Redis, MySQL, or other databases?

The registry contains PostgreSQL, NATS (message queue), and Caddy (static file
server) as managed packages. Entries are added by editing `registry/<name>.toml`
and rebuilding, and an entry is accepted only when it pins a trustworthy native
Windows build with a verified size and SHA-256. A Redis entry is deliberately
absent because no verified native Windows build was available when the registry
was written. For services like Redis, use the container fallback
(`use = "container:redis:7"`) with your own Docker or Podman, or run any native
server yourself with a `run` command and own its download, verification, and
lifecycle. Container services are clearly labelled in `burrow status`, keep
loopback-only ports, and have an ephemeral filesystem (no snapshots).

## Can I run multiple projects at the same time?

Yes, as separate `burrow` daemons. Automatic ports are remembered per project
and probed before reuse, so two projects do not intentionally share a port. A
fixed port in two projects will conflict; use `port = "auto"` for one of them.
Each project has its own state, logs, data, secrets, and snapshots.

## What happens if I edit `burrow.toml` while Burrow is running?

The running daemon keeps the configuration it started with. Run `burrow down`
and `burrow up` to apply changes. `burrow doctor` compares the running daemon's
config hash with the file and reports drift. Editing a command also changes the
trust hash, so the next `burrow up` asks for approval again.

## Why is my service restarting?

The default policy is `on-failure`. Burrow backs off from 500 ms to 30 seconds,
and stops retrying after five failures in 60 seconds. Check
`burrow logs <service>`, fix the cause, then `burrow restart <service>`; a manual
restart resets the breaker. See
[`troubleshooting.md`](troubleshooting.md#a-service-keeps-restarting-or-stopped-after-crash-loop).

## Does Burrow watch files and restart on change?

Yes, if a service opts in with `watch = ["api/src"]`. Changes under the listed
project-relative directories restart only that service after a short debounce;
dependents keep running. Watching is not enabled by default and is not a
whole-project watcher. Your framework's own watcher (Vite, uvicorn `--reload`,
`cargo watch`, ...) still works inside the service, because Burrow runs the
process normally. See the [configuration reference](config-reference.md#idle_stop-and-watch).

## Why does `api.localhost` not load?

Windows may not resolve `*.localhost` on your machine. Use the direct loopback
URL from `burrow status`, or add the name to your hosts file. The proxy still
routes by the `Host` header. Details are in
[`troubleshooting.md`](troubleshooting.md#httphttpssomethinglocalhost-does-not-resolve).

## Is the reverse proxy reachable from other machines?

Only if a routed service sets `expose = true`. Without it the proxy binds
`127.0.0.1`. With it the proxy binds all interfaces and Burrow logs a warning,
but requests from non-loopback peers are still restricted to exposed routes.
Service processes themselves are unaffected and continue to bind whatever their
commands specify; Burrow does not rewrite their bind address.

## Can I use Burrow in CI?

On a Windows runner, yes:

```powershell
burrow up --yes --detach
# run tests against the allocated ports
burrow down
```

`--yes` records trust for the current configuration without prompting. Use it
only after the file has been reviewed, such as in a reviewed pull request or a
throwaway CI checkout. `burrow top` requires an interactive terminal and will
refuse to run in CI; use `burrow status --json` and `burrow logs` instead.

## Is Burrow production-ready?

No. It is a local development tool. It has no sandbox, no privilege separation,
no high-availability behavior, and no promise of production parity. Services run
with your full user rights.

## How do I uninstall Burrow?

```powershell
cargo uninstall burrow-cli
```

Then remove any project `.burrow` directories you no longer need and, optionally,
the shared cache at `%LOCALAPPDATA%\burrow\cache`. Removing the cache forces the
next install to download and verify packages again.

## Where do I report a bug?

Open an issue using the repository's bug report template. Include
`burrow --version`, `burrow doctor --json`, `burrow status --json`, the exact
command, and the relevant logs with secrets removed. See
[`troubleshooting.md`](troubleshooting.md#reporting-a-bug).
