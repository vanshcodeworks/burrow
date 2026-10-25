# 0004 - File watching on ReadDirectoryChangesW

- Status: Accepted
- Date: 2026-10-25

## Context

`watch = ["api/src"]` asks Burrow to restart a service when source files
change. The obvious choice is the `notify` crate; the alternative is a small
wrapper over the Win32 API Burrow already depends on. Either way the risky part
is not the notification itself but the feedback loop: a restart writes logs,
and logs live in the project tree.

## Decision

1. Watch with `ReadDirectoryChangesW` directly, one OS thread per watched
   root, recursive, using an overlapped read plus two wait handles (change
   event, stop event) so the watch can be cancelled deterministically. This
   stays inside `burrow-supervisor/src/win.rs`, keeps the workspace free of a
   new dependency, and needs no additional `windows` crate features beyond
   `Win32_System_IO`.
2. Events are best-effort. A kernel buffer overflow surfaces as an `Overflow`
   event instead of silent loss; overflow is treated like any other change.
3. The daemon polls the watcher on its existing 250 ms tick and debounces per
   service: a restart fires when events stay quiet for 500 ms, or after 2 s at
   the latest, so an editor that saves a file in several steps produces one
   restart.
4. Paths containing a `.burrow` or `.git` component never trigger a restart.
   `.burrow` holds logs and data (a restart writing a log line would otherwise
   schedule the next restart); `.git` changes constantly while version control
   runs in the background. Everything else is the user's choice.
5. Only the watched service restarts. Dependents keep running: in development,
   restarting an API must not bounce the database it talks to. A deliberate
   `burrow restart` still cascades.

## Consequences

- Watch roots must exist at startup; a typo fails `burrow up` with the offending
  path instead of silently watching nothing.
- The watcher restarts services, it does not compile or build them; a `run`
  command that compiles on start composes naturally with watch.
- Containers and registry packages can be watched too; the restart path is the
  same one `burrow restart` uses.
