# Burrow in the local-development landscape

Burrow is a Windows-native process manager for local development stacks. It is
not trying to be every tool in this space. This page compares it with the tools a
Windows developer is most likely to weigh, says where each alternative is the
better fit, and links every competitor claim to official documentation.

**No competitor was run or benchmarked for this page.** Competitor facts
come from the official pages listed in [SOURCES.md](SOURCES.md), all retrieved on
2026-10-26. Burrow's own measurements come from
[`docs/benchmarks.md`](../benchmarks.md). Last reviewed: 2026-10-26.

## What Burrow is

For native services, Burrow reads one `burrow.toml` and starts each service as a
Windows child process in a Job Object. It health-gates dependents, allocates
stable per-project ports, proxies `*.localhost` routes over loopback, generates
environments and secrets, downloads pinned packages with SHA-256 verification,
captures logs, shows a terminal dashboard, and snapshots service data. It is
Windows 10/11 x86-64 only, runs services with your user rights, and is not a
sandbox or a production deployment tool. A service without a trustworthy native
build can use the optional Docker or Podman fallback, which is documented as
ephemeral and is not a Compose replacement. See [`README.md`](../../README.md),
[`docs/security.md`](../security.md), and
[`docs/compatibility.md`](../compatibility.md).

There is no published release or signed installer yet; the documented install
path is `cargo install --path crates\burrow-cli --locked` from a checkout.

## At a glance

| Tool | Unit of work | Windows story | Best fit |
| --- | --- | --- | --- |
| **Burrow** | Native Windows process | Native; Windows 10/11 x86-64 (11 Home verified, 10 unverified) | A Windows-native stack, no Docker daemon or VM, one config, verified package downloads |
| **Docker Desktop + Compose** | Linux container by default; Windows containers require all-users mode and Pro/Enterprise | Docker Desktop; WSL 2 default backend, or Hyper-V / Docker VMM | Linux images, isolation, a large ecosystem, cross-platform parity |
| **Podman + podman-compose** | Linux container | Podman machine VM (WSL or Hyper-V) on Windows | Rootless, daemonless Compose workflows |
| **Process Compose** | Native process | Published Windows amd64 and arm64 binaries | Portable YAML process orchestration without containers |
| **Overmind** | Native process | Not available | Procfile + tmux workflows on Linux, macOS, and BSD |
| **VS Code Dev Containers** | Container workspace | Requires Docker (Desktop or remote) | An editor-integrated, reproducible toolchain inside a container |
| **WinSW / NSSM** | Windows service | Native | Boot-time or Service Control Manager-managed Windows services |

## Docker Desktop + Docker Compose

**What it is.** Docker Compose defines and runs multi-container applications
from a single YAML file, with commands to start, stop, rebuild, inspect status,
stream logs, and run one-off commands [S1]. On Windows, Docker Desktop's default
WSL 2 backend runs Linux containers on WSL 2, which provides a Linux kernel;
Docker also documents Hyper-V and Docker VMM backends [S2][S3]. The documented
WSL 2 and Hyper-V requirements include 8 GB of RAM and hardware virtualization
[S3].

**Where it is a better fit than Burrow.**

- You need Linux binaries, Linux base images, or the container ecosystem. Burrow
  does not run Linux images natively; its optional container fallback delegates
  to Docker or Podman.
- You need filesystem and network isolation, or the VM boundary Docker Desktop
  uses on Windows [S2][S3][S15]. Burrow services run with your full user rights.
- You want one workflow across Windows, macOS, Linux, and CI. Compose is
  documented for development, testing, CI, and production, and Docker Desktop is
  available for Mac, Windows, and Linux [S1][S15]; Burrow is a local Windows
  development tool.
- You need image builds, a large image registry, or production-like parity
  [S1][S15].
- You want Compose's health-gated dependencies: `depends_on` with
  `condition: service_healthy` waits for a dependency's health check, although
  the default is only to wait until a container is running [S5].

**Where Burrow differs.** For native services, Burrow has no Docker daemon,
Linux VM, WSL distribution, or image build in the loop. It starts native Windows
processes, downloads three pinned native packages (PostgreSQL, NATS, Caddy), and
adds stable ports, a loopback proxy, generated environments, snapshots, `doctor`,
and a TUI. It can delegate an individual service to Docker or Podman with
`use = "container:IMAGE"`, but that container's filesystem is ephemeral and
Burrow is not a Compose replacement.

**Licensing.** Docker Desktop is free for personal use, education,
non-commercial open source, and small businesses with fewer than 250 employees
and less than $10 million in annual revenue; larger organizations, government
entities, and commercial use beyond that tier need a paid subscription [S4].
Burrow is MIT OR Apache-2.0.

**Fairness note on resources.** Docker Desktop's WSL 2 backend uses dynamic
memory allocation and requests CPU and memory as needed rather than reserving a
fixed block [S2]. This page does not report a Docker memory or startup number,
and Burrow's daemon RSS should not be read as a like-for-like comparison.

**Choose Docker Desktop + Compose when** you need Linux services, isolation, or
cross-platform parity. **Choose Burrow when** your services have trustworthy
native Windows builds and you want to remove the VM, daemon, and image layer.

## Podman + podman-compose

**What it is.** Podman Compose is an implementation of the Compose Spec with a
Podman backend. It focuses on rootless operation and a daemonless model: it
executes `podman` directly instead of talking to a running daemon. It depends on
Podman, a Podman network backend with DNS, Python 3.9+, PyYAML, and
python-dotenv, and is distributed as a single Python file [S7]. Podman itself
requires a virtual machine on Windows; the machine providers are WSL (default)
and Hyper-V [S6].

**Where it is a better fit than Burrow.**

- You want rootless, daemonless containers and the Compose Spec rather than
  Docker Desktop.
- You want an open-source container stack without Docker Desktop's subscription
  terms [S7][S16].
- You need Linux images, container isolation, or a Compose-style workflow on a
  platform where Podman runs [S6][S7].

**Where Burrow differs.** "Daemonless" does not mean "VM-less" on Windows:
Podman still runs Linux containers inside a Podman machine [S6]. Burrow avoids
that VM entirely by running native Windows processes, but it gives up container
isolation and Linux images. Burrow's container fallback can call Podman for an
individual service, but it does not manage Podman machines or Compose projects.

**Choose Podman + podman-compose when** you want a rootless, daemonless
container workflow and accept the Windows VM. **Choose Burrow when** the VM
itself is the thing you are trying to avoid and your services are native.

## Process Compose

**What it is.** Process Compose is a scheduler and orchestrator for
non-containerized applications. It is a single Go binary with no other
dependencies, configured in YAML [S8]. Its documented features include process
dependencies and startup order, recovery policies, manual restart, environment
variables, logs, liveness and readiness health checks, a TUI or CLI, a REST API
with optional token authentication, log caching, configuration merging,
namespaces, multiple replicas, foreground and interactive processes,
on-the-fly configuration edits, recipes, scheduled processes, file watching,
dependency graph visualization, and an MCP server [S8]. Windows amd64 and arm64
binaries are published [S9].

**Where it is a better fit than Burrow.**

- You want the same process orchestration on Windows, macOS, and Linux [S9].
- You need multiple replicas, namespaces, scheduled processes, a REST API,
  interactive processes, or the MCP integration [S8].
- You already install your own services and want a portable process runner
  rather than a Windows-specific package manager.
- You want a dependency graph view and on-the-fly configuration editing [S8].

**Where Burrow differs.** Burrow is deliberately Windows-specific. It adds a
registry of pinned native packages with URL, size, and SHA-256 verification, a
shared content-addressed cache, stable per-project ports, a loopback proxy with
`*.localhost` routes, generated environments and secrets, full-directory
snapshots and restore, `doctor`, and a trust prompt. Process Compose's documented
feature set centers on process orchestration; it is the closer fit when
portability and orchestration breadth matter more than those Windows-specific
conveniences.

**Choose Process Compose when** you want a cross-platform, single-binary process
runner and already own tool installation. **Choose Burrow when** you want the
Windows package, port, proxy, environment, and snapshot plumbing in one tool.

## Overmind

**What it is.** Overmind is a process manager for Procfile-based applications
and tmux. It starts processes in a tmux session, can connect to a process,
restart, stop, or kill individual processes, scale processes, allow a process to
die without stopping the others, auto-restart selected processes, assign `PORT`
values (base 5000, step 100 by default), and run as a daemon [S10]. It supports
Linux, *BSD, and macOS only, and requires tmux [S10].

**Where it is a better fit than Burrow.**

- You are on Linux, macOS, or BSD and already use tmux and Procfiles.
- You want to attach to a running process, scale a process, or restart one
  process without restarting the stack [S10].
- You want a single terminal with live process output and minimal configuration
  [S10].

**Where Burrow differs.** Burrow does not run on those platforms at all. On
Windows, Overmind is not an option. On Unix, Overmind's Procfile and tmux
workflow is a different model; if that is your team's standard, it is the better
fit.

**Choose Overmind when** you are on a supported Unix and want the Procfile/tmux
workflow. **Choose Burrow when** you are on Windows and want native process
supervision with health-gated startup and Windows package handling.

## VS Code Dev Containers

**What it is.** The VS Code Dev Containers extension uses a container as a
full-featured development environment. A `devcontainer.json` file describes the
container, workspace files are mounted or copied into it, and extensions run
inside it [S11]. The extension requires Docker locally or remotely [S11]. The
Development Containers specification is a metadata format for describing these
environments, and its reference CLI supports Docker Compose and a single
container [S12].

**Where it is a better fit than Burrow.**

- You want the editor, language runtimes, tools, and extensions to live in one
  reproducible container [S11].
- You want Linux tooling on Windows, remote or cloud development, or the same
  environment in local development and CI [S11][S12].
- You want Dev Container Features and templates, or team-wide toolchain
  consistency [S11].

**Where Burrow differs.** Dev Containers is an editor and workspace environment,
not a service orchestrator. It requires Docker, so it inherits Docker Desktop's
VM/WSL requirements, and it does not support Windows container images [S11].
Burrow runs services on the host and is editor-agnostic, but it does not isolate
the toolchain or the filesystem. The two solve different layers; a team could use
both, at the cost of maintaining two runtime stories.

**Choose Dev Containers when** the whole development environment should be a
container. **Choose Burrow when** you want host-native services and a
lightweight, editor-independent workflow.

## Adjacent tools: WinSW and NSSM

WinSW wraps and manages a custom process as a Windows service, with `install`,
`uninstall`, `start`, `stop`, `stopwait`, `restart`, and `status` commands and
XML or YAML configuration; most commands require Administrator privileges [S13].
NSSM installs an application as a Windows service, can set it to start
automatically, and supports restart throttling, I/O redirection, file rotation,
environment variables, and a multi-stage graceful shutdown [S14].

**Where they are a better fit than Burrow.** These are the right tools when a
process must be registered with the Windows Service Control Manager, can be
configured to start automatically, and can run under a service account
[S13][S14]. Burrow runs services as children of its per-user daemon and does not
install Windows services.

**Choose WinSW or NSSM when** you need a Windows service. **Choose Burrow when**
you need a development stack with dependency ordering, health checks, ports, and
logs.

## Where Burrow is the wrong choice

Be honest about the boundary:

- **Linux-only services or images:** use a container tool directly (Docker
  Compose, Podman Compose, or Dev Containers); Burrow's fallback delegates to
  one but does not replace it.
- **Isolation or a security boundary:** use containers or a VM. Burrow services
  run as your Windows user.
- **Production deployment:** Burrow is a local development tool. It has no
  production support, clustering, or remote deployment.
- **macOS or Linux:** Burrow has no backend for either. Use Overmind, Process
  Compose, Docker, or Podman.
- **A service that must start at boot:** use WinSW or NSSM.
- **A fully reproducible toolchain:** use Dev Containers or a toolchain manager.
- **Process scaling, tmux attach, or a REST control plane:** use Overmind on
  Unix or Process Compose.
- **A large service catalog:** Burrow's registry has three native packages;
  everything else needs a container fallback or your own `run` command.

## Performance: what is measured and what is not

Burrow's numbers below are from [`docs/benchmarks.md`](../benchmarks.md), measured
on one Windows 11 Home machine (Ryzen 7 4800H, 15.4 GB RAM) with the
`examples/node-postgres` project. They are one sample; the cold-start and
quick-start figures include network time.

| Measurement | Burrow result | Notes |
| --- | ---: | --- |
| Warm `burrow up` | 1.87 s | Second warm run: 1.95 s |
| Daemon RSS | 10.6-12.6 MB | Burrow's own process, not the whole stack |
| Cold `burrow up` | 61.35 s | Includes the 356 MiB PostgreSQL download, SHA-256 verification, extraction, `initdb`, seed, and health checks |
| Checkout to first HTTP 200 | 245.24 s | Empty Cargo, npm, and Burrow caches; includes `cargo install` (162.62 s) and `npm install` (3.19 s) |

**No competitor was benchmarked for this page.** The benchmark run did not have
Docker's daemon running, so it reports no Docker comparison rather than
inventing one. Docker Desktop's WSL 2 backend also uses dynamic memory
allocation [S2], so a single "Docker uses X MB" number would be misleading even
if measured. A future benchmark suite may add a like-for-like Compose run when
the daemon is available; until then, treat the table as Burrow-only evidence.

## Maintaining this page

- Competitor facts are tied to [SOURCES.md](SOURCES.md), with a retrieval date
  for every page. If an official page changes, update the source entry and the
  claim together.
- Do not add a competitor performance number without a reproducible method,
  machine details, and raw data.
- Do not turn "not documented" into "does not exist". If a feature is not in the
  official docs, leave it out or mark it as unverified.
- Corrections are welcome; open an issue with the source that contradicts the
  page.

[S1]: SOURCES.md#s1-docker-compose-overview
[S2]: SOURCES.md#s2-docker-desktop-wsl-2-backend
[S3]: SOURCES.md#s3-install-docker-desktop-on-windows
[S4]: SOURCES.md#s4-docker-desktop-license-agreement
[S5]: SOURCES.md#s5-control-startup-and-shutdown-order-in-compose
[S6]: SOURCES.md#s6-podman-machine
[S7]: SOURCES.md#s7-podman-compose
[S8]: SOURCES.md#s8-process-compose
[S9]: SOURCES.md#s9-process-compose-release
[S10]: SOURCES.md#s10-overmind
[S11]: SOURCES.md#s11-vs-code-dev-containers
[S12]: SOURCES.md#s12-development-containers-specification
[S13]: SOURCES.md#s13-winsw
[S14]: SOURCES.md#s14-nssm
[S15]: SOURCES.md#s15-docker-overview
[S16]: SOURCES.md#s16-podman
