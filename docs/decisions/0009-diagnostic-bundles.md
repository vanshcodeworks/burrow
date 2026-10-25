# 0009 - Local, redacted diagnostic bundles

- Status: Accepted
- Date: 2026-10-26

## Context

A useful bug report needs the project configuration, runtime state, environment,
daemon output, and any crash that preceded the failure. Those same files can
contain generated service passwords, the daemon IPC token, application output,
and the user's directory layout. Asking users to assemble them by hand produces
incomplete reports and encourages pasting secrets into issues.

Burrow has no telemetry and no upload service. Any diagnostic artifact must be
created locally, be inspectable before it is shared, and fail closed when a
secret cannot be identified.

## Decision

1. `burrow doctor --report` writes a zip and prints its path. It never uploads
   the file. The default location is
   `<project>/.burrow/reports/burrow-report-<timestamp>-<pid>.zip`.
   `--report-dir <DIR>` overrides the directory and implies `--report`.
2. The bundle contains `README.txt`, `report.json`, `environment.json`, a
   redacted `config.redacted.json`, a redacted `state.redacted.json` when state
   exists, a tail of `.burrow/daemon.log`, and up to ten recent crash notes.
3. Service logs are deliberately excluded. Burrow does not scan application
   output, so a service log can contain anything the application printed.
   `environment.json` lists the log file names and sizes instead, and the README
   tells the user to share only the relevant log if a maintainer asks.
4. Redaction is applied to every text entry. It replaces known values from
   `.burrow/secrets`, the state token, sensitive environment values, URL
   passwords, Bearer tokens, and common `password`/`token`/`secret`
   assignments. The project directory becomes `<project>`; the home directory
   and `%LOCALAPPDATA%` become `%USERPROFILE%` and `%LOCALAPPDATA%`. Both raw
   and JSON-escaped path forms are replaced.
5. Redaction is best-effort. Values shorter than six characters are not treated
   as global secrets because replacing them would destroy ordinary text, and
   application-written text may contain a secret Burrow cannot recognize. The
   README says so and asks the user to review the bundle before sharing.
6. Crash notes are structured JSON under `%LOCALAPPDATA%\burrow\crashes`, with
   the temporary directory as a fallback. The panic hook writes a note before
   chaining to the previous hook, writes atomically through a temporary file,
   and records the version, RFC 3339 and Unix-millisecond timestamp, PID,
   thread, OS, architecture, project directory, message, location, and an
   optional backtrace.
7. A bundle includes notes whose recorded project matches the current project,
   plus notes with no recorded project, newest ten. Notes for other projects are
   never included.
8. The archive is written to a `.zip.tmp` file and renamed only after
   `finish()` succeeds, so an interrupted run does not leave a half-valid zip
   under the final name.
9. `doctor --json` gains `generated_at`, `burrow_version`, and an optional
   `report_path`. The bundle's copy of the report uses the same fields after
   path redaction.

## Consequences

- **Users can produce a reviewable artifact without a support account.** The
  command works offline and leaves the bundle in the project by default.
- **Service logs require a second step.** This is a deliberate trade: copying
  arbitrary application output would make the "redacted" claim false.
- **Short secrets are not globally replaced.** A one- or two-character password
  is already outside Burrow's generated-secret model; the bundle still redacts
  structured environment values and known assignments.
- **Crash notes accumulate locally.** There is no automatic deletion. Users can
  remove `%LOCALAPPDATA%\burrow\crashes` at any time; the bundle only reads it.
- **The panic hook itself is not exercised end to end by the test suite.** The
  note writer, reader, filtering, atomic rename, and redaction have unit and
  integration coverage; producing a real panic from the CLI has no supported
  command, so that path remains a manual/soak gap.
