# burrow-testbin

Test-only fake service used by Burrow's integration tests (Milestone M1:
Job Objects, memory caps, kill-on-close, and graceful console shutdown).

**This binary is not shipped with Burrow.** It lives inside the
`burrow-supervisor` package so that Cargo builds it automatically for that
package's integration tests (`CARGO_BIN_EXE_burrow-testbin`). It is test-only
and exists for tests and manual debugging of the supervisor.

## Usage

```
burrow-testbin <mode> [arguments...]
```

The first argument selects the mode. `exit`, `sleep`, `leak`, `tree`,
`graceful`, and `ignore-ctrl` take exactly one argument; `host` takes a program
and zero or more arguments; `serve` takes an optional port; `bind` takes an
address and reads the port from `$PORT`. Paths may contain spaces; pass each as
a single argument (no shell quoting is involved when tests use
`Command::arg`). All stdout lines are flushed immediately, before any long
operation.

### `exit <CODE>`

Prints `exit <CODE>` and exits immediately with that code. `CODE` is parsed as
a `u32` and passed to the OS as a 32-bit process exit code.

### `sleep <MS>`

Prints `sleeping <MS>`, sleeps for `MS` milliseconds, then exits 0.

### `leak <BYTES>`

Prints `leak <BYTES>`, then allocates and touches 1 MiB chunks filled with
`0xAB`, keeping them in a `Vec`, until at least `BYTES` are allocated. Progress
(`allocated <N> MiB`) is printed every 8 MiB, plus a final summary.

If a Job Object memory cap is reached, the Rust allocator aborts the process;
that abort is the expected result. If the target is reached without aborting,
the process sleeps for 60 seconds and exits 0.

### `tree <PIDFILE>`

Prints `tree`, spawns `burrow-testbin sleep 60000`, and writes two lines to
`PIDFILE`:

```
<own pid>
<child pid>
```

The child is deliberately **not** created in a new process group, so it
inherits the console process group of the parent. The parent keeps the child
handle open while sleeping for 60 seconds. If `PIDFILE` cannot be written, the
process exits 3.

### `graceful <FLAGFILE>`

Prints `graceful`, installs a console control handler for `CTRL_C_EVENT` and
`CTRL_BREAK_EVENT`, then writes `<FLAGFILE>.ready` containing `ready`.

The handler is installed **before** the `.ready` file appears, so a test may
send Ctrl+Break as soon as it observes readiness. The handler only stores to an
atomic flag; all file I/O happens on the main thread. The main loop polls the
flag every 20 ms. When set, it writes `FLAGFILE` containing `graceful` and calls
`std::process::exit(0)`.

### `ignore-ctrl <READYFILE>`

Prints `ignore-ctrl`, installs a handler that returns `TRUE` for
`CTRL_C_EVENT`/`CTRL_BREAK_EVENT` (ignoring them), writes `READYFILE`
containing `ready`, then sleeps 20 ms at a time forever, until the process is
killed (normally by closing its Job Object).

### `host <PROGRAM> [ARGS...]`

Prints `host child pid: <pid>`, then supervises `PROGRAM` in a Job Object using
`burrow_supervisor::ManagedProcess` and idles for an hour at a time. The process
is meant to be killed abruptly by a test: the kernel then closes the job handle
and `KILL_ON_JOB_CLOSE` terminates the supervised tree. Exit code 3 if the child
cannot be spawned or the Tokio runtime cannot be created.

### `serve [PORT]`

Binds `127.0.0.1:PORT` (or `127.0.0.1:$PORT` when no argument is given), prints
`serving on 127.0.0.1:<port>`, greets each client with
`hello from burrow-testbin`, and echoes received bytes. Backs the idle-stop,
lazy wake-up, and proxy tests. Exit code 3 when the bind fails.

### `bind <ADDRESS>`

Binds `ADDRESS:$PORT` and otherwise behaves like `serve`. Used by the listener
inspection tests: `bind 0.0.0.0` creates a wildcard listener, `bind 127.0.0.1`
stays loopback-only. `$PORT` is required. Exit code 3 when the bind fails.

## Usage errors

No arguments, `--help`/`-h`, an unknown mode, a missing/extra argument, or a
malformed integer prints usage to stderr and exits 2.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| 0 | Success (`sleep`, `leak`, `tree`, `graceful`) |
| 2 | Usage error |
| 3 | Failed to write a PID/ready/flag file, locate the executable, spawn the child, or start the `host` runtime |
| 4 | `SetConsoleCtrlHandler` failed |
| other | The value requested by `exit <CODE>` |

## Windows notes and test assumptions

- Console handlers are registered with
  `windows 0.62.2`'s `SetConsoleCtrlHandler`; all `unsafe` is confined to
  `src/win.rs`, and every `unsafe` block has a `// SAFETY:` comment.
- Tests should spawn `graceful`/`ignore-ctrl` with `CREATE_NEW_PROCESS_GROUP`
  and then call
  `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, <pid>)`. `CTRL_C_EVENT` can also
  be handled, but cannot be generated for a specific process group reliably.
- `tree`'s child inherits its process group so group-targeted control events
  reach both processes.
- The expected cleanup primitive is a Job Object with `KILL_ON_JOB_CLOSE`; this
  helper intentionally has no self-cleanup logic for its child.

## Manual smoke test (PowerShell)

```powershell
cargo run -p burrow-supervisor --bin burrow-testbin -- exit 7
$LASTEXITCODE   # 7

cargo run -p burrow-supervisor --bin burrow-testbin -- sleep 500
```
