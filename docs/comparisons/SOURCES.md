# Sources for the comparison

Every competitor claim in [README.md](README.md) is tied to one of the official
pages below. All pages were retrieved on **2026-10-26**. The comparison does not
use blog posts, third-party benchmarks, or recollection of a product.

**Rule for absent claims:** if a source does not state a feature, the comparison
does not say the product lacks it. "Not documented" means only that.

## S1 Docker Compose overview

- URL: <https://docs.docker.com/compose/>
- Retrieved: 2026-10-26
- Supports:
  - Docker Compose is "a tool for defining and running multi-container
    applications".
  - It manages services, networks, and volumes in a single YAML configuration
    file.
  - It has commands to start, stop, and rebuild services; view status; stream
    logs; and run a one-off command on a service.
  - Docker documents Compose for "production, staging, development, testing, as
    well as CI workflows".

## S2 Docker Desktop WSL 2 backend

- URL: <https://docs.docker.com/desktop/features/wsl/>
- Retrieved: 2026-10-26
- Supports:
  - WSL 2 is "a full Linux kernel built by Microsoft".
  - Docker Desktop can run on the WSL 2 backend.
  - "Because WSL 2 uses dynamic memory allocation, Docker Desktop requests only
    the CPU and memory it actually needs" rather than a fixed allocation.

## S3 Install Docker Desktop on Windows

- URL: <https://docs.docker.com/desktop/setup/install/windows-install/>
- Retrieved: 2026-10-26
- Supports:
  - Docker Desktop for Windows supports WSL 2, Hyper-V, and Docker VMM (Beta)
    backends; WSL 2 is the default.
  - The documented WSL 2 and Hyper-V requirements include a 64-bit processor
    with SLAT, 8 GB of system RAM, and hardware virtualization enabled in
    firmware.
  - Windows containers require Windows 10 or 11 Professional or Enterprise
    edition; Home and Education editions only allow Linux containers.
  - Windows containers are only supported in the all-users installation mode.
  - A per-user installation does not require administrator privileges; enabling
    WSL 2 for the first time is a one-time per-machine operation that does.

## S4 Docker Desktop license agreement

- URL: <https://docs.docker.com/subscription/desktop-license/>
- Retrieved: 2026-10-26
- Supports:
  - Docker Desktop is free for personal use, education, non-commercial open
    source projects, and small businesses with fewer than 250 employees and less
    than $10 million in annual revenue.
  - It requires a paid subscription for professional use in larger
    organizations, government entities, and commercial use beyond the free-tier
    limits.

## S5 Control startup and shutdown order in Compose

- URL: <https://docs.docker.com/compose/how-tos/startup-order/>
- Retrieved: 2026-10-26
- Supports:
  - Compose starts and stops containers in dependency order.
  - By default, Compose waits only until a container is running, not until it is
    ready.
  - `depends_on` with `condition: service_healthy` makes a dependent wait for a
    dependency's `healthcheck` to pass.

## S6 Podman machine

- URL: <https://docs.podman.io/en/latest/markdown/podman-machine.1.html>
- Retrieved: 2026-10-26
- Supports:
  - "Podman on MacOS and Windows requires a virtual machine."
  - The Windows machine providers are WSL (the default) and Hyper-V.

## S7 Podman Compose

- URL: <https://github.com/containers/podman-compose>
- Retrieved: 2026-10-26
- Supports:
  - Podman Compose is "an implementation of Compose Spec with Podman backend".
  - The project focuses on rootless operation and a "daemon-less process model,
    we directly execute podman, no running daemon".
  - It depends on Podman, a Podman network backend with DNS name resolution,
    Python 3.9 or newer, PyYAML, and python-dotenv.
  - It is distributed as a single Python file script.

## S8 Process Compose

- URL: <https://github.com/F1bonacc1/process-compose>
- Retrieved: 2026-10-26
- Supports:
  - Process Compose is "a simple and flexible scheduler and orchestrator to
    manage non-containerized applications".
  - It is written in Go, is a single binary, and has no other dependencies.
  - Its documented features include process dependencies and startup order,
    recovery policies, manual restart, per-process and global environment
    variables, logs, liveness and readiness health checks, a TUI or CLI, a REST
    API with optional token authentication, log caching, configuration merging,
    namespaces, multiple replicas, foreground and interactive processes,
    on-the-fly configuration edit, recipes, scheduled processes, file watching,
    dependency graph visualization, and an MCP server.

## S9 Process Compose release

- URL: <https://github.com/F1bonacc1/process-compose/releases/tag/v1.122.0>
- Retrieved: 2026-10-26
- Supports:
  - Release v1.122.0 (published 2026-08-17) includes
    `process-compose_windows_amd64.zip` and
    `process-compose_windows_arm64.zip` assets, so Windows binaries are
    published.
  - The same release publishes macOS (amd64/arm64) and Linux
    (386/amd64/arm/arm64) assets, so builds exist for Windows, macOS, and
    Linux.

## S10 Overmind

- URL: <https://github.com/DarthSim/overmind>
- Retrieved: 2026-10-26
- Supports:
  - Overmind is "a process manager for Procfile-based applications and tmux".
  - It starts processes in a tmux session and can connect to, restart, stop, or
    kill individual processes.
  - It supports process scaling, processes that may die without stopping the
    others, automatic restart, `PORT` assignment (base 5000, step 100 by
    default), and `OVERMIND_PROCESS_<name>_PORT` variables.
  - It supports daemon mode and a Unix-socket or TCP command channel.
  - "At the moment, Overmind supports Linux, *BSD, and macOS only." It requires
    tmux.

## S11 VS Code Dev Containers

- URL: <https://code.visualstudio.com/docs/devcontainers/containers>
- Retrieved: 2026-10-26
- Supports:
  - The Dev Containers extension "lets you use a container as a full-featured
    development environment".
  - A `devcontainer.json` file tells VS Code how to access or create a
    development container with a defined tool and runtime stack.
  - Workspace files are mounted from the local file system, or copied or cloned
    into the container; extensions run inside the container.
  - Docker is required locally or remotely. On Windows, Docker Desktop is the
    documented path; Windows 10 Home (2004+) requires Docker Desktop 2.3+ and
    the WSL 2 backend.
  - "Windows container images are not supported."
  - Bind mounts have "some performance overhead on Windows and macOS".

## S12 Development Containers specification

- URL: <https://containers.dev/overview>
- Retrieved: 2026-10-26
- Supports:
  - The specification uses a structured `devcontainer.json` metadata format to
    enrich existing development environments.
  - The open-source CLI reference implementation supports Docker Compose and a
    simplified single-container option.

## S13 WinSW

- URL: <https://github.com/winsw/winsw>
- Retrieved: 2026-10-26
- Supports:
  - WinSW "can be used to wrap and manage a custom process as a Windows
    service".
  - It provides `install`, `uninstall`, `start`, `stop`, `stopwait`, `restart`,
    and `status` commands and uses XML or YAML configuration.
  - "Most commands require Administrator privileges to execute."

## S14 NSSM

- URL: <https://nssm.cc/usage>
- Retrieved: 2026-10-26
- Supports:
  - NSSM installs an application as a Windows service (`nssm install
    <servicename>`).
  - Service start mode can be set to automatic (`SERVICE_AUTO_START`).
  - It supports restart throttling and a configurable delay between restarts.
  - It supports stdout/stderr redirection, file rotation, environment
    variables, and a multi-stage graceful shutdown with a final forced
    termination.

## S15 Docker overview

- URL: <https://docs.docker.com/get-started/docker-overview/>
- Retrieved: 2026-10-26
- Supports:
  - Docker packages and runs an application in a "loosely isolated environment
    called a container"; "the isolation and security" let many containers run
    on one host.
  - Docker uses namespaces to provide the isolated workspace, and those
    namespaces "provide a layer of isolation".
  - Docker uses a client-server architecture; the Docker daemon does the heavy
    lifting, and Docker Desktop includes the daemon, client, and Compose.
  - A container is a runnable instance of an image; images are built from a
    Dockerfile; Docker Hub is a public registry used by default.
  - Changes to a container's state that are not in persistent storage disappear
    when the container is removed.

## S16 Podman

- URL: <https://podman.io/>
- Retrieved: 2026-10-26
- Supports:
  - Podman describes itself as "The best free & open source container tools".
  - The site lists the Apache License 2.0.

## Burrow sources

These are repository documents, not competitor sources:

- `README.md` - positioning, limitations, measured resource numbers, and the
  existing Docker comparison.
- `docs/security.md` - trust model and the statement that Burrow is not a
  sandbox or privilege boundary.
- `docs/compatibility.md` - verified and unverified Windows configurations.
- `docs/benchmarks.md` - machine details, benchmark method, and raw numbers.
- `docs/config-reference.md` - container fallback behavior and its ephemeral
  filesystem.
- `docs/architecture.md` - Job Objects, native process supervision, and the
  container fallback design.
