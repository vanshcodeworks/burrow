//! Fake service used by Burrow's integration tests.
//!
//! See `README.md` for the mode reference. This binary is test-only and is not
//! shipped with Burrow.

mod win;

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::process::{self, Command};
use std::thread;
use std::time::Duration;

use burrow_supervisor::{ManagedProcess, ProcessSpec};

/// Bad mode, missing argument, extra argument, or malformed integer.
const EXIT_USAGE: i32 = 2;
/// File or child-process setup failure inside the test helper.
const EXIT_IO: i32 = 3;
/// The console control handler could not be installed.
const EXIT_HANDLER: i32 = 4;

const MIB: usize = 1024 * 1024;
const PROGRESS_INTERVAL: usize = 8 * MIB;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const LONG_SLEEP: Duration = Duration::from_secs(60);

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = run(&args);
    process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let Some((mode, rest)) = args.split_first() else {
        print_usage();
        return EXIT_USAGE;
    };

    match mode.as_str() {
        "exit" => cmd_exit(rest),
        "sleep" => cmd_sleep(rest),
        "leak" => cmd_leak(rest),
        "tree" => cmd_tree(rest),
        "graceful" => cmd_graceful(rest),
        "ignore-ctrl" => cmd_ignore_ctrl(rest),
        "host" => cmd_host(rest),
        "serve" => cmd_serve(rest),
        "--help" | "-h" => {
            print_usage();
            EXIT_USAGE
        }
        _ => {
            eprintln!("error: unknown mode '{mode}'");
            print_usage();
            EXIT_USAGE
        }
    }
}

/// Exits immediately with the requested 32-bit process exit code.
fn cmd_exit(args: &[String]) -> i32 {
    let code = match single_arg("exit", args).and_then(|raw| parse_u32("exit", raw)) {
        Ok(code) => code,
        Err(code) => return code,
    };

    emit(&format!("exit {code}"));
    process::exit(code as i32);
}

/// Sleeps for a fixed number of milliseconds.
fn cmd_sleep(args: &[String]) -> i32 {
    let ms = match single_arg("sleep", args).and_then(|raw| parse_u64("sleep", raw)) {
        Ok(ms) => ms,
        Err(code) => return code,
    };

    emit(&format!("sleeping {ms}"));
    thread::sleep(Duration::from_millis(ms));
    0
}

/// Leaks memory in 1 MiB chunks, then idles so tests can observe the cap.
fn cmd_leak(args: &[String]) -> i32 {
    let bytes = match single_arg("leak", args).and_then(|raw| parse_u64("leak", raw)) {
        Ok(bytes) => bytes,
        Err(code) => return code,
    };

    emit(&format!("leak {bytes}"));

    // On 64-bit Windows this conversion is lossless. Saturating on a 32-bit
    // target means "leak as much as possible" instead of panicking.
    let target = usize::try_from(bytes).unwrap_or(usize::MAX);
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut allocated = 0usize;
    let mut next_progress = PROGRESS_INTERVAL;

    while allocated < target {
        // `vec!` fills every byte with 0xAB, which commits and touches the
        // whole chunk. If the Job Object cap is reached, the Rust allocator
        // aborts, which is the expected behaviour for this mode.
        chunks.push(vec![0xABu8; MIB]);
        allocated = allocated.saturating_add(MIB);

        if allocated >= next_progress {
            emit(&format!("allocated {} MiB", allocated / MIB));
            next_progress = next_progress.saturating_add(PROGRESS_INTERVAL);
        }
    }

    emit(&format!(
        "allocated {} MiB, sleeping {} s",
        chunks.len(),
        LONG_SLEEP.as_secs()
    ));
    // Keep the allocation observable so the optimizer cannot elide the touch.
    black_box(&chunks);
    thread::sleep(LONG_SLEEP);
    0
}

/// Spawns a child `sleep` in the same process group and records both PIDs.
fn cmd_tree(args: &[String]) -> i32 {
    let pidfile = match single_arg("tree", args) {
        Ok(path) => path,
        Err(code) => return code,
    };

    emit("tree");

    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            eprintln!("error: cannot locate the current executable: {err}");
            return EXIT_IO;
        }
    };

    // No CREATE_NEW_PROCESS_GROUP: the child must inherit our console process
    // group so group-targeted Ctrl+Break events reach the whole tree.
    let child = match Command::new(&exe).arg("sleep").arg("60000").spawn() {
        Ok(child) => child,
        Err(err) => {
            eprintln!("error: failed to spawn child process: {err}");
            return EXIT_IO;
        }
    };

    let pids = format!("{}\n{}\n", process::id(), child.id());
    if let Err(err) = fs::write(pidfile, pids) {
        eprintln!("error: failed to write pid file '{pidfile}': {err}");
        return EXIT_IO;
    }

    // `child` stays in scope (and its handle open) for the whole sleep.
    thread::sleep(LONG_SLEEP);
    0
}

/// Waits for Ctrl+C / Ctrl+Break, records it, and exits cleanly.
fn cmd_graceful(args: &[String]) -> i32 {
    let flagfile = match single_arg("graceful", args) {
        Ok(path) => path,
        Err(code) => return code,
    };

    emit("graceful");

    // Install the handler before publishing `.ready`, so a test that signals
    // as soon as it sees the file can never hit the default handler.
    if let Err(err) = win::install_graceful_handler() {
        eprintln!("error: {err}");
        return EXIT_HANDLER;
    }

    let readyfile = format!("{flagfile}.ready");
    if let Err(err) = fs::write(&readyfile, "ready") {
        eprintln!("error: failed to write ready file '{readyfile}': {err}");
        return EXIT_IO;
    }

    while !win::graceful_requested() {
        thread::sleep(POLL_INTERVAL);
    }

    // File I/O happens here, on the main thread, never in the handler.
    if let Err(err) = fs::write(flagfile, "graceful") {
        eprintln!("error: failed to write flag file '{flagfile}': {err}");
        return EXIT_IO;
    }

    process::exit(0);
}

/// Ignores Ctrl+C / Ctrl+Break and idles until the process is killed.
fn cmd_ignore_ctrl(args: &[String]) -> i32 {
    let readyfile = match single_arg("ignore-ctrl", args) {
        Ok(path) => path,
        Err(code) => return code,
    };

    emit("ignore-ctrl");

    // Install before publishing readiness, for the same race-free reason as
    // `graceful`.
    if let Err(err) = win::install_ignore_handler() {
        eprintln!("error: {err}");
        return EXIT_HANDLER;
    }

    if let Err(err) = fs::write(readyfile, "ready") {
        eprintln!("error: failed to write ready file '{readyfile}': {err}");
        return EXIT_IO;
    }

    loop {
        thread::sleep(POLL_INTERVAL);
    }
}

/// Supervises a child in a Job Object, then idles until killed.
///
/// This mode exists so tests can prove that `KILL_ON_JOB_CLOSE` terminates the
/// whole tree when a *separate supervisor process* dies abruptly, rather than
/// only when a job handle is dropped in-process.
fn cmd_host(args: &[String]) -> i32 {
    let Some((program, rest)) = args.split_first() else {
        return usage_error("host requires a program to supervise");
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: could not create the Tokio runtime: {err}");
            return EXIT_IO;
        }
    };

    runtime.block_on(async {
        let spec =
            ProcessSpec::new("host-child", program.as_str()).args(rest.iter().map(String::as_str));
        match ManagedProcess::spawn(spec).await {
            Ok(process) => {
                emit(&format!("host child pid: {}", process.pid()));
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            }
            Err(err) => {
                eprintln!("error: failed to supervise '{program}': {err}");
                EXIT_IO
            }
        }
    })
}

/// Listens on a port, greets each client, and echoes bytes back.
///
/// This mode backs the idle-stop and lazy wake-up integration tests: held
/// clients keep the service "busy", closing them lets it idle-stop, and the
/// greeting proves that a woken client's tunnel carries server-first bytes.
///
/// Ctrl+Break uses the default console handler, which terminates the process;
/// the supervisor relies on that for graceful shutdown.
fn cmd_serve(args: &[String]) -> i32 {
    let port = match args.first() {
        Some(raw) => match raw.parse::<u16>() {
            Ok(port) => port,
            Err(_) => return usage_error(&format!("serve: invalid port '{raw}' (expected u16)")),
        },
        // Without an argument the port comes from $PORT, which the daemon
        // sets to the service's allocated port.
        None => match std::env::var("PORT").ok().and_then(|raw| raw.parse().ok()) {
            Some(port) => port,
            None => return usage_error("serve: pass a port, or run with $PORT set"),
        },
    };

    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("error: could not bind 127.0.0.1:{port}: {err}");
            return EXIT_IO;
        }
    };
    emit(&format!("serving on 127.0.0.1:{port}"));

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        thread::spawn(move || {
            use std::io::{Read, Write};
            if stream.write_all(b"hello from burrow-testbin\n").is_err() {
                return;
            }
            let mut buffer = [0_u8; 512];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if stream.write_all(&buffer[..read]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });
    }
    0
}

/// Returns the single argument required by a mode.
fn single_arg<'a>(mode: &str, args: &'a [String]) -> Result<&'a str, i32> {
    match args {
        [arg] => Ok(arg),
        [] => Err(usage_error(&format!("{mode} requires one argument"))),
        _ => Err(usage_error(&format!("{mode} accepts exactly one argument"))),
    }
}

fn parse_u32(mode: &str, raw: &str) -> Result<u32, i32> {
    raw.parse::<u32>()
        .map_err(|_| usage_error(&format!("{mode}: invalid integer '{raw}' (expected a u32)")))
}

fn parse_u64(mode: &str, raw: &str) -> Result<u64, i32> {
    raw.parse::<u64>()
        .map_err(|_| usage_error(&format!("{mode}: invalid integer '{raw}' (expected a u64)")))
}

/// Prints an error plus usage and returns the usage exit code.
fn usage_error(message: &str) -> i32 {
    eprintln!("error: {message}");
    print_usage();
    EXIT_USAGE
}

/// Writes one line to stdout and flushes it before any long operation.
fn emit(line: &str) {
    let mut stdout = io::stdout().lock();
    // A test harness closing the pipe is not this helper's problem.
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn print_usage() {
    eprintln!(
        "\
burrow-testbin: fake service used by Burrow integration tests

Usage:
  burrow-testbin exit <CODE>
  burrow-testbin sleep <MS>
  burrow-testbin leak <BYTES>
  burrow-testbin tree <PIDFILE>
  burrow-testbin graceful <FLAGFILE>
  burrow-testbin ignore-ctrl <READYFILE>
  burrow-testbin host <PROGRAM> [ARGS...]
  burrow-testbin serve [PORT]

Modes:
  exit         Exit immediately with the given process exit code.
  sleep        Sleep for MS milliseconds, then exit 0.
  leak         Allocate BYTES of memory, then sleep for 60 seconds.
  tree         Spawn a child sleep, write both PIDs to PIDFILE, sleep 60 s.
  graceful     Wait for Ctrl+C/Ctrl+Break, write FLAGFILE, exit 0.
  ignore-ctrl  Ignore Ctrl+C/Ctrl+Break and sleep until killed.
  host         Supervise PROGRAM in a Job Object, then idle until killed.
  serve        Listen on PORT (or $PORT), greet each client, echo bytes back.

Exit codes: 0 success, 2 usage error, 3 file/process setup error,
            4 console control handler setup error."
    );
}
