# Benchmarks

These are real measurements, not estimates. The Burrow-only numbers were
produced by `scripts/benchmark.ps1`; the Docker Compose head-to-head uses
`scripts/benchmark-stacks.ps1`. Both can be reproduced on the machine below:

```powershell
cargo build --release -p burrow-cli
.\scripts\benchmark.ps1 -Project examples\node-postgres
.\scripts\benchmark.ps1 -Cold -Project examples\node-postgres
.\scripts\benchmark-stacks.ps1 -WarmRuns 3
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
| Measured | 2026-10-24 (Burrow-only); 2026-10-25 (head-to-head) |

Docker CLI 24.0.7 is installed. An earlier run on 2026-10-24 had no Docker
daemon, so it reported no comparison. The 2026-10-25 head-to-head below ran with
Docker Desktop 24.0.7 and Compose v2.23.3-desktop.2 on its WSL 2 backend.

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

## Docker Compose comparison (2026-10-25)

This is a same-machine, same-project, same-endpoint comparison. It uses the
`examples/node-postgres` app, schema, and seed. The Compose fixture is generated
by `scripts/benchmark-stacks.ps1`: PostgreSQL 16.15 on Alpine, a Node 22.13.1
image, a health-gated `depends_on`, and the same `/api/todos` response.

| Property | Value |
| --- | --- |
| Docker Desktop / engine | 24.0.7 |
| Docker Compose | v2.23.3-desktop.2 |
| Backend | WSL 2, kernel 6.18.54.1-microsoft-standard-WSL2 |
| Docker CPUs / memory | 16 / 7.469 GiB |
| PostgreSQL image | `postgres:16.15-alpine`, digest `sha256:f4610795587492cd4fb887c2dc3de301b813278bb6f616992061141b5f15ac36` |
| API image | locally built, 159,806,724 bytes |
| Raw data | [`benchmarks/2026-10-25-stacks.json`](benchmarks/2026-10-25-stacks.json) |

Startup is wall-clock time from launching the stack until
`http://127.0.0.1:<port>/api/todos` returns HTTP 200 with the seeded row. "Cold"
means no application data (empty Compose volume, or no `.burrow` data for
Burrow); "warm" means the data directory or volume is retained. Package and
image caches are warm in every row, and image pull/build is excluded from
startup.

| Scenario | Burrow to HTTP 200 | Docker Compose to HTTP 200 |
| --- | ---: | ---: |
| Cold application data | 10.7 s | 4.11 s |
| Warm sample 1 | 1.87 s | 3.20 s |
| Warm sample 2 | 1.88 s | 1.92 s |
| Warm sample 3 | 1.84 s | 2.08 s |

Burrow's warm range was **1.84-1.88 s** (median 1.87 s); Compose's was
**1.92-3.20 s** (median 2.08 s). The first Compose warm sample was the slowest;
the next two were 1.92 s and 2.08 s. This is one machine and one sample set, not
a statistical distribution.

Compose was faster when initializing empty application data: **4.11 s** versus
**10.7 s** for Burrow. That row excludes image pull and build. The final clean
run pulled the PostgreSQL image in **21.57 s** and built the API image in
**2.67 s**; those setup steps are not part of the startup numbers. Burrow's
equivalent cold start with an empty package cache was **61.35 s** on 2026-10-24,
but that run included a 356 MB download and is not the same setup boundary as
the 4.11 s Compose row.

Memory is not like-for-like. Burrow's daemon working set was **10.9-11.0 MB**
and its whole process tree was **171.5-172.2 MB**. Compose's two containers used
**37.2-62.1 MB combined** in `docker stats`, but that excludes Docker Desktop's
VM and daemon overhead. Do not read the daemon number as Docker Desktop's total
footprint, or the container sum as the whole Docker cost.

Caveats: one Windows 11 Home machine; Docker Desktop's WSL 2 backend allocates
memory dynamically; no Podman run; no Linux or macOS host; the Compose fixture
binds the API to `0.0.0.0` inside the container so Docker can forward the port,
while the native example binds loopback. The fixture and harness are in
`scripts/benchmark-stacks.ps1`; the raw JSON includes every sample, image ID,
and setup time.

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

- The Burrow-only `benchmark.ps1` startup is wall-clock time from launching
  `burrow up --detach --yes` until `.burrow/state.json` reports `ready`, polling
  every 100 ms.
- The head-to-head `benchmark-stacks.ps1` startup is wall-clock time from
  launching either stack until `/api/todos` returns HTTP 200 with the seeded
  row, polling every 100 ms.
- RSS is the Windows working set of the daemon after a two-second settle.
- Environment RSS walks the process tree from the daemon PID, so it includes
  PostgreSQL child processes and the proxy.
- Compose memory is `docker stats --no-stream` for the two containers. It
  excludes Docker Desktop's VM and daemon, so it is not comparable to Burrow's
  daemon RSS.
- The cold Burrow run deletes the benchmark cache first and lets Burrow download
  the verified artifact from the upstream source. The head-to-head cold rows
  keep the package/image caches warm and remove only application data, which is
  why they are labeled "cold application data."
- The benchmark runs a small number of samples, not a statistical distribution.
  Treat these numbers as reproducible measurements on this machine, not a
  promise for every laptop.
