# 0010 - Long-path process working directories

- Status: Accepted
- Date: 2026-10-26

## Context

`CreateProcessW` has two path limits that Rust's standard library does not
abstract away for us:

- `lpCurrentDirectory` is limited to `MAX_PATH` (260 UTF-16 units). A project
  directory longer than that makes the daemon, every service, health commands,
  `burrow exec`, and package steps fail with `ERROR_DIRECTORY` (os error 267).
- The `\\?\` verbatim prefix does not help for `lpCurrentDirectory`. The call
  rejects it, and `cmd.exe` treats a verbatim directory as a UNC path and
  silently falls back to `C:\Windows`. Stripping the prefix is required for
  shell-based commands to run where the user expects.
- `lpApplicationName` is also limited to `MAX_PATH`. A relative program inside
  a long project resolves to a long absolute path.

Rust's own filesystem calls already handle long paths, so configuration,
`.burrow/` state, logs, and service data work; only process creation fails. The
project path is stored and displayed without a verbatim prefix, and that must
not change.

## Decision

1. Add `burrow_supervisor::process_working_directory`. It removes a `\\?\`
   prefix from drive-letter and UNC paths. If the plain path is shorter than
   260 UTF-16 units it is returned unchanged.
2. For longer paths, resolve the existing 8.3 short name with
   `GetShortPathNameW`, passing the verbatim form so the call itself is not
   subject to `MAX_PATH`. Remove the prefix from the result and verify that it
   is still shorter than 260 units.
3. If the short name cannot be resolved, or is still too long (for example
   because an administrator disabled 8.3 name generation on the volume), return
   an error that names the directory and tells the user to move the project to
   a shorter path. Do not fall back to a verbatim path or to a different
   directory.
4. Apply the helper at every `current_dir` site: supervised service creation,
   `cmd` health checks, the detached daemon, `burrow exec`, the browser
   launcher used by `burrow open`, and package init/seed steps. Apply the same
   shortening to a long `lpApplicationName`.
5. Keep `BURROW_PROJECT_DIR` and all user-facing paths in their original long
   form. Only the process working directory and application path use the short
   form.

## Consequences

- **Long project paths work when the volume has 8.3 names**, which is the
  default on NTFS. This was verified with a 273-character project directory on
  Windows 11.
- **A volume with 8.3 names disabled cannot host a project at 260+ characters.**
  The failure is explicit and actionable rather than `os error 267`.
- **Services see a short current directory.** `std::env::current_dir()` and
  relative paths resolve inside the same directory; only the spelling differs.
  `BURROW_PROJECT_DIR` still carries the long path.
- **The helper adds one Win32 call only for long paths.** Short paths take the
  unchanged fast path.
- **Not verified here**: `LongPathsEnabled=0`, a volume with 8.3 names disabled,
  UNC project paths, and non-NTFS filesystems. The compatibility page records
  those as gaps rather than claims.

## Alternatives considered

- **Require short project paths.** Simple, but rejects a normal Windows
  directory layout and gives no path forward.
- **Pass `\\?\` as `lpCurrentDirectory`.** Rejected by `CreateProcessW`, and
  breaks `cmd.exe` even where accepted.
- **Change the process working directory to a short temporary directory and
  pass the project through environment variables.** Breaks relative commands,
  tools that inspect the current directory, and the meaning of `cwd`.
- **Create a junction with a short name.** Leaves machine state outside the
  project, needs cleanup, and fails on volumes or policies that forbid
  junctions.
