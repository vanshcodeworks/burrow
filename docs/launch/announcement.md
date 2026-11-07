# Announcement drafts

**Draft only.** Do not post until a human has reviewed the copy and the release
checklist is complete. The repository URL is filled in; do not add a download
URL, hash, or version that has not been published.

## One line

Native Windows local development stacks - no Docker daemon or VM by default.

## Short announcement (about 130 words)

Burrow is a Windows-native local development environment manager written in
Rust. You describe a stack in one `burrow.toml`; `burrow up` allocates stable
loopback ports, downloads and verifies pinned packages, starts services in
dependency order, waits for health checks, captures logs, and shows a live
dashboard. PostgreSQL, NATS, and Caddy are managed as native Windows processes;
other services can run as commands or through an optional Docker/Podman
fallback. There is no telemetry, and the code is MIT OR Apache-2.0.

Windows 11 x86-64 is verified; Windows 10 and ARM64 are not yet verified. There
is no signed installer or published release yet. Try it from a checkout with
`cargo install --path crates\burrow-cli --locked`, then run the
`examples\node-postgres` stack. Details: [`README.md`](../../README.md).

## Longer announcement (about 550 words)

### Burrow: native Windows local stacks without the container layer

Local development on Windows often means running Linux services through Docker
Desktop, a VM, or WSL. That is the right trade-off for many teams, but it is a
lot of machinery for a small stack of native Windows services.

Burrow is a different trade-off. It is a Windows-native local development
environment manager: one `burrow.toml`, one `burrow up`, and each native service
runs as a Windows child process in a Job Object. For those services there is no
Docker daemon, VM, or WSL distribution in the loop. Burrow allocates stable
loopback ports, expands environment templates, generates per-project secrets,
downloads pinned packages with SHA-256 verification, starts services in
dependency order, waits for health checks, captures logs, and shows a terminal
dashboard.

**What works today**

- Strict config validation, local overrides, dependency graphs, cycle reports,
  template expansion, generated secrets, and a checked JSON Schema.
- A supervisor with Job Objects, memory and CPU limits, health-gated startup,
  restart backoff, a crash-loop breaker, and graceful shutdown.
- A verified registry for PostgreSQL 16.15, NATS, and Caddy, with pinned URL,
  size, SHA-256, and version.
- Automatic stable ports and a loopback proxy for `*.localhost` routes.
- Snapshot, restore, and reset with atomic directory operations and recovery
  journals.
- `doctor` with a concrete remedy for every failure, machine-readable JSON, and
  local-only redacted diagnostic bundles.
- A TUI dashboard, CLI JSON output, idle stop with lazy wake-up, restart on file
  change, and an optional Docker/Podman fallback for a service without a
  trustworthy native build.

**What it is not**

Burrow is not a Docker replacement, a container runtime, a security sandbox, or
a production deployment tool. It does not run Linux binaries or images
natively, and its optional container fallback is ephemeral. Services run with
your user rights. Read [`docs/security.md`](../security.md) before deciding
whether that trade-off fits a project. There is no telemetry, no cloud account,
and no GUI.

**Measured, with caveats**

On one Windows 11 Home machine (Ryzen 7 4800H, 15.4 GB RAM), using the same
`examples/node-postgres` app, schema, seed, and `/api/todos` endpoint, warm
startup to HTTP 200 was **1.84-1.88 s** for Burrow and **1.92-3.20 s** for
Docker Compose. Compose initialized empty application data faster: **4.11 s**
versus **10.7 s** for Burrow. Image pull/build is excluded from those startup
numbers, and Burrow's package download is excluded too (the package cache was
warm). Memory is not like-for-like: Burrow's daemon used **10.9-11.0 MB**,
while Compose's two containers used **37.2-62.1 MB** excluding Docker Desktop's
VM and daemon. This is one machine and a small sample, not a statistical
distribution. The method, raw JSON, and caveats are in
[`docs/benchmarks.md`](../benchmarks.md).

A sourced comparison with Docker Compose, Podman Compose, Process Compose,
Overmind, VS Code Dev Containers, WinSW, and NSSM is in
[`docs/comparisons/README.md`](../comparisons/README.md). Only Docker Compose was
run; the other comparisons are documentation-based, and the page says where each
alternative is the better fit.

**Try it**

There is no signed installer or published release yet. From a checkout:

```powershell
cargo install --path crates\burrow-cli --locked
cd examples\node-postgres
npm install
burrow up
```

Then run `burrow doctor`, open the printed URL, and stop with `burrow down`.
The five-minute walkthrough is in [`docs/demo.md`](../demo.md); the full
walkthrough is in [`docs/getting-started.md`](../getting-started.md).

**Status and feedback**

Windows 11 x86-64 is verified on one non-elevated machine. Windows 10, ARM64, a
true standard-user account, OneDrive/UNC paths, FAT/exFAT, corporate antivirus,
`LongPathsEnabled=0`, and 8.3-disabled volumes are not yet verified. Podman
parity is not yet verified. The project needs clean-machine and CI runs before
it can claim broad Windows support.

Useful feedback: a real project that does not fit the model, a Windows 10 or
ARM64 result, a Podman comparison, and any error message that did not tell you
what to do next. Please open an issue with the `doctor --report` bundle if it is
safe to share; the bundle is local and redacted, and Burrow never uploads it.

## Show HN draft

**Title:** Show HN: Burrow - native Windows local dev stacks without Docker by default

**Body:**

I built Burrow because I wanted a reproducible local stack on Windows without
running every service through Docker Desktop, a VM, or WSL. It reads one
`burrow.toml`, starts native Windows services in dependency order, allocates
stable loopback ports, verifies pinned downloads, health-gates dependents, and
gives you logs, snapshots, `doctor`, and a TUI.

It is deliberately narrow: Windows 11 x86-64 today (Windows 10 is unverified),
native services first, with an optional Docker/Podman fallback for a service
that has no trustworthy native build. It is not a container runtime, a sandbox,
or a production tool.

The measured warm start for the Node + PostgreSQL example was 1.84-1.88 s on one
Windows 11 machine; Docker Compose on the same example and endpoint was
1.92-3.20 s. Compose was faster at initializing empty application data (4.11 s
vs 10.7 s). Those are small samples on one machine, not a benchmark suite; the
method and raw JSON are in the repo. Only Docker Compose was run; the other
comparisons are documentation-based.

There is no signed installer yet, so the current path is `cargo install --path
crates\burrow-cli --locked` from a checkout. I would especially value Windows 10
and ARM64 results, Podman parity checks, and honest feedback on where the
native-process model is the wrong tool. Source and docs:
<https://github.com/vanshcodeworks/burrow>.

## Channel notes

- The repository is public; confirm the URL and the source install path before
  posting.
- If a signed release exists, replace the install section with the real release
  URL and hash from `SHA256SUMS.txt`. Do not invent either.
- Keep the benchmark caveats in the post. Do not shorten them into a claim that
  Burrow is always faster or lighter than Docker.
- Do not add competitor performance numbers beyond the measured Compose
  head-to-head.
