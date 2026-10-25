# Benchmarks

These are real measurements, not estimates. They were produced by
`scripts/benchmark.ps1` on the machine below and can be reproduced with:

```powershell
cargo build --release -p burrow-cli
.\scripts\benchmark.ps1 -Project examples\node-postgres
.\scripts\benchmark.ps1 -Cold -Project examples\node-postgres
```

## Machine

| Property | Value |
| --- | --- |
| CPU | AMD Ryzen 7 4800H with Radeon Graphics |
| Cores / threads | 8 / 16 |
| RAM | 15.4 GB |
| OS | Windows 11 Home Single Language, 10.0.26200 |
| Burrow | 0.1.0, release build (`lto = "thin"`) |
| Rust | 1.94.1 |
| Measured | 2026-10-24 |

Docker CLI 24.0.7 is installed, but its daemon was not running during the
measurement. No Docker comparison is reported; the benchmark script marks the
comparison as skipped rather than inventing one.

## Project

`examples/node-postgres`: a Node.js API, PostgreSQL 16.15, a seeded database,
and the loopback proxy. The first run initializes the cluster and runs
`db/seed.sql`.

## Results

| Scenario | Burrow ready after | Burrow daemon RSS | Whole environment RSS |
| --- | ---: | ---: | ---: |
| Cold start, empty cache | 61.35 s | 12.6 MB | 178.6 MB |
| Warm start, populated cache | 1.87 s | 10.6 MB | 172.1 MB |

The cold-start number includes downloading the 356 MB PostgreSQL archive,
verifying its SHA-256, extracting it, running `initdb`, starting both services,
and waiting for health checks. A second warm-start run measured 1.95 s and
10.7 MB daemon RSS, which shows the result is stable on this machine.

Whole-environment RSS sums the daemon and every descendant process: the Node
API, the PostgreSQL backend and its helper processes, and the proxy. PostgreSQL
accounts for roughly 100 MB and Node for roughly 55 MB; Burrow's share is the
daemon figure.

Both published targets were met:

- Burrow daemon below 20 MB RSS idle: **10.6-12.6 MB**.
- Warm `burrow up` well under 10 seconds: **1.87-1.95 s**.

## Time to first success

A separate README pass on 2026-10-26 followed the documented quick start from a
fresh copy of `examples/node-postgres`, with the Cargo, npm, and Burrow caches
empty:

| Step | Wall clock |
| --- | ---: |
| `cargo install --path crates\burrow-cli --locked` (empty `CARGO_HOME`) | 162.62 s |
| `npm install` (empty npm cache) | 3.19 s |
| `burrow up --detach --yes` to both healthy and a seeded HTTP 200 (empty `BURROW_CACHE_DIR`) | 79.43 s |
| **Total, checkout to first successful request** | **245.24 s (4 min 5 s)** |

The `cargo install` step downloaded 0.27 GB of crates and compiled in a separate
temporary target directory, so it is not comparable to the 54.47 s incremental
release build measured in cycle 12. A developer with a populated Cargo registry
will see a shorter build.

The first start includes the 356.0 MiB PostgreSQL download, SHA-256
verification, extraction, `initdb`, the seed, and both health checks. It is
network-dependent; the 61.35 s cold start above was measured on a different day
with the same archive. A warm restart after `burrow down` took **1.87 s** to
both services healthy, and `http://127.0.0.1:<api-port>/api/todos` returned
`[{"id":1,"title":"Open http://api.localhost"}]`. `burrow doctor --json`
reported **11 ok / 0 warnings / 0 failures**.

On this machine `api.localhost` did not resolve (`DNS name does not exist`), so
the direct loopback URL was used. That is the documented fallback; it is a
Windows resolver behavior, not a Burrow failure.

Caveats: this was the development machine, not a clean VM. The Windows file
cache, the Rust toolchain, the MSVC build tools, Node 22.13.1, and npm 10.9.2
were already installed. The Cargo registry, npm cache, and Burrow package cache
were empty. The numbers are one sample and include real network time.

## TUI

Measured separately with the release binary in a console window, while the
`node-postgres` example was running:

| Property | Value |
| --- | ---: |
| `burrow top` working set | 11.0 MB |
| CPU over a 5-second idle window | 0.31% |

The TUI refreshes once per second. It stays under the ~15 MB target. With the
project stopped it measured 9.0 MB and the same 0.31% CPU.

## Method

- Startup is wall-clock time from launching `burrow up --detach --yes` until
  `.burrow/state.json` reports `ready`, polling every 100 ms.
- RSS is the Windows working set of the daemon after a two-second settle.
- Environment RSS walks the process tree from the daemon PID, so it includes
  PostgreSQL child processes and the proxy.
- The cold run deletes the benchmark cache first and lets Burrow download the
  verified artifact from the upstream source.
- The benchmark runs one or two samples, not a statistical distribution. Treat
  these numbers as reproducible measurements on this machine, not a promise for
  every laptop.
