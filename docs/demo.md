# Demo script

A five-minute walkthrough of Burrow using `examples/node-postgres`. This page is
the recording script; no video or GIF is published yet. Every command is real,
and the output shown below comes from a recorded run on the benchmark machine.
PIDs, ports, timings, and log text change between runs.

For a clean first-run recording, start from a fresh checkout, or delete the
example's disposable `.burrow` directory. The 356 MiB PostgreSQL archive stays in
the shared package cache, so a repeat recording does not download it again.

## Before recording

```powershell
cd examples\node-postgres
npm install
cargo build --release -p burrow-cli
```

Use a normal, non-elevated terminal at roughly 120x30. Close other applications
that print notifications. Do not open `.burrow/secrets`, and do not run
`burrow env db` on camera: both contain generated passwords.

## 1. Start the stack

```powershell
..\..\target\release\burrow.exe up --detach
```

On a fresh project, the first run prints the trust summary and waits:

```text
Burrow will run commands from P:\Burrow\examples\node-postgres\burrow.toml in P:\Burrow\examples\node-postgres

  api: node server.js
  db: download and run `postgres@16` from the Burrow registry

Treat this like a Makefile or package.json script: it can run arbitrary code.
Approve and start? [y/N]
```

Type `y` and press Enter. The approval is stored in `.burrow/trust.json`; later
runs of this configuration do not prompt. On a cold cache, download progress for
`postgres` appears before the service list. A warm run looks like this:

```text
Burrow is running in the background for `node-postgres`
  api              healthy  http://api.localhost
  db               healthy  postgres://postgres@127.0.0.1:43655/postgres

  logs:   burrow logs -f
  status: burrow status
  stop:   burrow down
```

Say: “Both services are native Windows processes. There is no Docker daemon, VM,
or WSL distribution running.”

## 2. Show the API

```powershell
curl.exe -s http://api.localhost/api/todos
curl.exe -s http://api.localhost/health
```

Expected output:

```text
[{"id":1,"title":"Open http://api.localhost"}]
ok
```

If `api.localhost` does not resolve on the recording machine, use the direct
loopback URL instead. `burrow status` prints the port:

```powershell
curl.exe -s http://127.0.0.1:<api-port>/api/todos
```

The proxy still routes by the HTTP `Host` header; only name resolution changes.
The seed row is created once, after PostgreSQL passes its health check.

## 3. Show live state

```powershell
..\..\target\release\burrow.exe status
```

Representative output from one run:

```text
Project: node-postgres  (pid 28324, control 127.0.0.1:31560)

SERVICE  STATE    PORT   RAM        CPU   UPTIME  URL
api      healthy  42498  54.9 MiB   0.0%  5s      http://api.localhost
db       healthy  43655  110.3 MiB  0.0%  6s      postgres://postgres@127.0.0.1:43655/postgres
```

Point out that `RAM` is each service's working set, the daemon is separate, and
the ports were allocated automatically. The project remembers them in
`.burrow/ports.json`.

```powershell
..\..\target\release\burrow.exe doctor
```

Representative output:

```text
Burrow doctor · node-postgres (P:\Burrow\examples\node-postgres)
  ok    platform           windows (x86_64)
  ok    session            normal user token (not elevated)
  ok    config             burrow.toml is valid (2 services)
  ok    cache              C:\Users\Asus\AppData\Local\burrow\cache (1.3 GiB)
  ok    daemon             running (pid 28324, ready, 2 services)
  ok    trust              commands approved for this configuration (hash a9c061837826)
  ok    secrets            generated passwords present for 2 services
  ok    package db         postgres@16.15 installed in C:\Users\Asus\AppData\Local\burrow\cache\packages\43bb45f173a6f08cf1d29a97a6d8deb119e8e8093a24c00d2d1001a0ccaa8281
  ok    command api        `node` resolves to C:\Program Files\nodejs\node.exe
  ok    proxy              listening on port 80 for 1 route

10 ok, 0 warnings, 0 problems
```

Say: “`doctor` is read-only. Every failure includes a remedy, and `--json` makes
the same report machine-readable for CI.”

## 4. Show logs and the dashboard

```powershell
..\..\target\release\burrow.exe logs
```

Representative output:

```text
db           2026-10-24 18:14:33.273 IST [35404] LOG:  starting PostgreSQL 16.15, compiled by Visual C++ build 1944, 64-bit
db           2026-10-24 18:14:33.274 IST [35404] LOG:  listening on IPv4 address "127.0.0.1", port 43655
db           2026-10-24 18:14:33.364 IST [35404] LOG:  database system is ready to accept connections
api          API listening on http://127.0.0.1:42498
```

```powershell
..\..\target\release\burrow.exe top
```

The dashboard fills the terminal and refreshes once per second with service
state, ports, memory, CPU, uptime, and recent logs. Use the arrow keys or `j` /
`k` to select a service. Press `q` or `Esc` to leave it running and return to the
prompt. Do not press `r` during the recording unless you want to demonstrate a
restart; it restarts the selected service and its dependents.

## 5. Stop, snapshot, and restore

```powershell
..\..\target\release\burrow.exe down
```

Expected:

```text
Burrow stopped
```

```powershell
..\..\target\release\burrow.exe snapshot before-demo
..\..\target\release\burrow.exe snapshots
```

Representative output:

```text
Snapshot `before-demo` created (2 services, 39.4 MiB)
  api             no data
  db             39.4 MiB  979 files

Stored in P:\Burrow\examples\node-postgres\.burrow\snapshots\before-demo
NAME                     CREATED (UTC)              SIZE  SERVICES
before-demo              2026-10-24 12:44:51    39.4 MiB  db
```

Say: “Snapshots are full directory copies, not database dumps. They are built in
a temporary directory and renamed into place, and the daemon must be stopped so
nothing is writing to the data directory.”

```powershell
..\..\target\release\burrow.exe restore before-demo
```

Expected:

```text
Restored snapshot `before-demo` for 2 services.
Run `burrow up` to start from the restored data.
```

If the recording has time, run `burrow up --detach --yes` again and re-query
`/api/todos` to show the data is back.

## 6. Close

```powershell
..\..\target\release\burrow.exe down
```

Final message:

> Burrow runs the stack as native processes, verifies managed downloads, gives
> every service a stable loopback port, and keeps the whole environment small.
> The measured warm start for this example is 1.87-1.95 seconds, with the
> daemon at 10.6-10.7 MB RSS. The numbers and method are in
> [`benchmarks.md`](benchmarks.md).

## Recovery beats

If a step fails during recording, keep the terminal visible and use the real
diagnostic instead of hiding it:

| Symptom | Recovery |
| --- | --- |
| `api.localhost` does not resolve | Switch to `http://127.0.0.1:<api-port>` from `burrow status`, or add the hosts entry shown in [`troubleshooting.md`](troubleshooting.md#httphttpssomethinglocalhost-does-not-resolve). |
| A port is occupied | Run `burrow down`, then `burrow up` again; automatic ports are probed and remembered. |
| PostgreSQL initialization fails | Confirm the terminal is not elevated, then `burrow reset db` and `burrow up`. |
| A service is unhealthy | Run `burrow logs <service>` and read the last error; the daemon log is `.burrow/daemon.log`. |
| The trust prompt appears again | A command, package, working directory, or service name changed. Review the summary before approving. |

Never show `.burrow/secrets` or a full `burrow env <service>` output on camera.
