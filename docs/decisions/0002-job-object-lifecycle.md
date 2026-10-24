# 0002 - Job Object lifecycle

- Status: Accepted
- Date: 2026-10-24

## Context

Burrow must not leave orphaned database, cache, or web-server processes behind
when a project stops, when a service crashes, or when Burrow itself is killed.
Orphans hold ports and file locks and can corrupt data directories. Graceful
shutdown also matters: databases must be allowed to flush and close their files
instead of being terminated mid-write.

Windows Job Objects provide both containment and cleanup, but the order and
timing of job creation, process assignment, handle closure, and termination
determine whether those guarantees hold.

## Decision

1. Create one Job Object per service with `CreateJobObjectW` and set
   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` in the job's extended limit information.
2. Assign the child to the job at creation time using
   `PROC_THREAD_ATTRIBUTE_JOB_LIST` in a `STARTUPINFOEX` attribute list passed to
   `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT`. Do not use a post-spawn
   `AssignProcessToJobObject` call; that leaves a race in which the child can
   create descendants that remain outside the job.
3. Apply a job-wide commit cap through `JobMemoryLimit` and a per-process cap
   through `ProcessMemoryLimit`. Apply an optional hard CPU rate cap when the
   service requests one.
4. Create the child with `CREATE_NEW_PROCESS_GROUP`. For graceful shutdown, send
   `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)` to that group, wait for the
   grace period, and then call `TerminateJobObject` if processes remain.
5. Close the job handle only after the job is empty. Closing it earlier triggers
   kill-on-close and defeats the graceful path.
6. Drain stdout and stderr through pipes on reader tasks while the child runs so
   that log forwarding cannot deadlock the child.

## Consequences

- Normal shutdown gives services a chance to flush, while the forced path still
  guarantees that the process tree is gone.
- `KILL_ON_JOB_CLOSE` covers the crash case because Windows closes the job handle
  when Burrow exits.
- STARTUPINFOEX plumbing is more complex than a post-spawn assignment, but the
  complexity is localized in `burrow-supervisor/src/win.rs`.
- **Known limitation:** Ctrl+Break relay requires a console. If Burrow is started
  from a detached or GUI parent with no console, the signal cannot be delivered
  and shutdown falls back to forced termination. Service-specific stop commands
  (for example `pg_ctl stop -m fast`) can be added later for gentler stops.
- Resource limits are operational guardrails, **not a security boundary**. They
  do not sandbox services, which run with the user's filesystem and network
  access.
