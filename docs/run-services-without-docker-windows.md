# Run local services on Windows without Docker

If your goal is a local PostgreSQL database, a NATS queue, or a static server on
Windows, you have two realistic options: start the native Windows binaries
yourself, or run Linux containers through a VM. Burrow does the first for three
services and can fall back to the second for everything else.

## Option 1: native Windows binaries (no Docker daemon, no VM)

Burrow downloads a pinned, SHA-256-verified Windows build into a shared cache,
allocates a stable port for your project, initializes the data directory once,
waits for health, and supervises the process in a Windows Job Object.

Built-in packages:

| Package | Version | Upstream |
| --- | --- | --- |
| PostgreSQL | 16.15 | EnterpriseDB binaries |
| NATS Server | 2.15.1 | nats.io |
| Caddy | 2.11.7 | caddyserver.com |

Install and start:

```powershell
iwr https://raw.githubusercontent.com/vanshcodeworks/burrow/main/scripts/install.ps1 | iex

mkdir C:\dev\my-app
cd C:\dev\my-app
burrow init --template postgres
burrow up
```

`burrow init --from .\docker-compose.yml` converts an existing Compose file
instead. The [migration guide](migration-from-docker-compose.md) explains the
mapping and the differences.

## What you still provide

- **Language runtimes.** Node, Python, Go, .NET, Java and friends must be on
  `PATH`. Burrow starts your command; it does not install languages.
- **A normal, non-elevated terminal.** PostgreSQL's `initdb` refuses to
  initialize a cluster under an administrator token.
- **Windows 11 x86-64 for the verified path.** Windows 10, ARM64, standard-user
  accounts, OneDrive/UNC paths and long-path edge cases are documented as
  unverified in the [compatibility matrix](compatibility.md).

## Option 2: container fallback for services without a Windows build

Redis, MongoDB, MySQL, Elasticsearch and most other services do not ship a
trustworthy, pinned Windows binary that Burrow can verify. Instead of inventing
a hash, Burrow delegates those to Docker Desktop or Podman:

```toml
[services.cache]
use = "container:redis:7"
container_port = 6379
```

Container services get loopback-only port mapping and an ephemeral filesystem.
Burrow snapshots do not capture container internals. This still requires Docker
Desktop or Podman to be installed; it is not a Docker replacement for those
services.

## Why not just use Docker Compose?

Compose is the better tool if you need Linux images, cross-platform parity,
isolation, or a large service catalog. Burrow is for the narrower case where
most of your stack has a real Windows binary and you want:

- no Linux VM / WSL backend for native services
- stable per-project ports instead of random host ports
- health-gated startup instead of `depends_on` on process start
- verified downloads with a pinned URL, byte size and SHA-256
- full-directory snapshots and a live terminal dashboard

The [sourced comparison](comparisons/README.md) and
[measured benchmarks](benchmarks.md) state where each tool wins, including the
cases where Compose is faster.

## Trade-offs to accept

- Burrow is **not a sandbox**. Services run with your user rights.
- It is a local development tool: no clustering, remote deployment or production
  support.
- The binaries are unsigned; SmartScreen may warn on first run.
- It is Windows-only and pre-1.0.
