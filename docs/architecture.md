# Architecture

This document describes the M1 foundation: crate boundaries, the Windows
supervisor design, and the safety rules later milestones must preserve. For
current status, see [`roadmap.md`](roadmap.md).

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
process. The stub crates are intentionally empty in M1 and carry their design
intent in crate-level docs.

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
both streams continuously and forward complete lines to `tracing` while the
child runs, so a full pipe buffer cannot block the child. M1 provides the
forwarding primitives; M2 adds per-service rotating files and the ring buffer
used by the TUI.

## Safety rules

- Product `unsafe` is confined to `crates/burrow-supervisor/src/win.rs`.
- Every `unsafe` block carries a `// SAFETY:` comment that explains why its
  preconditions hold.
- Win32 handles are owned by wrappers in the `win` module and cannot escape as
  raw handles through the safe public API.
- Public functions validate inputs and translate Win32 failures into errors;
  expected failure paths do not panic.
- No other product module uses `unsafe`. The supervisor's test-only support
  module uses a few Win32 query calls to inspect process liveness and job
  membership from the outside; those calls carry SAFETY comments too. This is a
  review-enforced rule, backed by clippy with `-D warnings` in CI.

## Testing strategy

- Unit tests cover pure logic: configuration validation, dependency ordering,
  port allocation, and extraction path checks as those modules land.
- CLI integration tests spawn the built `burrow` binary with
  `std::process::Command` and assert exit codes and stderr.
- Windows supervisor integration tests spawn real children through
  `burrow-testbin` inside Job Objects. They exercise memory-cap enforcement,
  kill-on-close, graceful shutdown ordering, and the forced-termination fallback.
- Tests that exercise console control events run with `--test-threads=1` because
  console events are process-wide. CI enforces this.
- CI on `windows-latest` runs rustfmt, clippy (`-D warnings`), the workspace test
  suite, and a release build, then uploads `burrow.exe`. A separate job runs
  `cargo audit`.
- `scripts/check.ps1` mirrors the format, clippy, and test checks locally.

## Resource limits are not a security boundary

Job Objects help contain accidental resource use and ensure child processes are
cleaned up. They do not isolate services from one another or from the user's
account. Every service runs with the same access to files, the network, and the
desktop as the user who started Burrow, and can read or modify anything that user
can. Do not use Burrow to run untrusted code.
