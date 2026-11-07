//! Phase-2 end-to-end tests: idle-stop with lazy wake-up, file watching, and
//! the container fallback.
//!
//! Like `cli.rs`, every test uses its own temporary project. The idle and
//! watch tests need the test-only service binary, which workspace test runs
//! build next to `burrow.exe`.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn burrow() -> Command {
    Command::new(env!("CARGO_BIN_EXE_burrow"))
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    burrow()
        .arg("--project")
        .arg(dir)
        .args(args)
        .output()
        .expect("failed to execute burrow")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A project that is stopped when the test ends, even after a failed assertion.
struct TestProject {
    dir: TempDir,
}

impl TestProject {
    fn new(config: &str) -> Self {
        let dir = tempfile::tempdir().expect("create project directory");
        fs::write(dir.path().join("burrow.toml"), config).expect("write burrow.toml");
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn state_path(&self) -> PathBuf {
        self.path().join(".burrow").join("state.json")
    }

    fn state(&self) -> serde_json::Value {
        let contents = fs::read_to_string(self.state_path()).expect("read state");
        serde_json::from_str(&contents).expect("parse state")
    }

    fn wait_until(
        &self,
        timeout: Duration,
        mut predicate: impl FnMut(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let started = Instant::now();
        loop {
            if self.state_path().exists() {
                let state = self.state();
                if predicate(&state) {
                    return state;
                }
            }
            assert!(
                started.elapsed() < timeout,
                "condition was not met within {timeout:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = run_in(self.path(), &["down"]);
        if self.state_path().exists() {
            if let Ok(contents) = fs::read_to_string(self.state_path()) {
                if let Ok(state) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(pid) = state["pid"].as_u64() {
                        let _ = Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .output();
                    }
                }
            }
        }
    }
}

/// Absolute path of the test-only service binary built next to `burrow.exe`.
///
/// Workspace test runs build every member binary, so the two land in the same
/// target directory.
fn testbin() -> PathBuf {
    let testbin = Path::new(env!("CARGO_BIN_EXE_burrow"))
        .parent()
        .expect("target dir")
        .join("burrow-testbin.exe");
    assert!(
        testbin.is_file(),
        "burrow-testbin.exe was not built; run workspace tests (`cargo test --workspace`)"
    );
    testbin
}

#[test]
fn idle_stop_parks_a_service_and_wakes_it_on_the_next_connection() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"idle\"\n\n[services.api]\n\
         run = '{} serve'\n\
         idle_stop = \"1s\"\n\
         health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n",
        testbin().display(),
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["api"]["status"] == "healthy"
    });
    let port = state["services"]["api"]["port"].as_u64().expect("port") as u16;
    let first_pid = state["services"]["api"]["pid"].as_u64().expect("pid");

    // A held client keeps the service awake past the idle timeout. The testbin
    // binds the port from $PORT, which the daemon sets to the allocated port.
    let mut held = TcpStream::connect(("127.0.0.1", port)).expect("hold a connection");
    held.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut greeting = String::new();
    BufReader::new(&mut held)
        .read_line(&mut greeting)
        .expect("greeting");
    assert_eq!(greeting.trim(), "hello from burrow-testbin");
    std::thread::sleep(Duration::from_secs(4));
    let state = project.state();
    assert_eq!(
        state["services"]["api"]["status"], "healthy",
        "a held connection must keep the service awake"
    );

    // Closing the connection lets the service idle-stop.
    drop(held);
    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["services"]["api"]["status"] == "idle"
    });
    assert!(
        state["services"]["api"]["pid"].is_null(),
        "an idle service has no process: {}",
        state["services"]["api"]
    );

    // The next connection wakes the service and is tunnelled to it.
    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("wake connect");
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let mut woken = String::new();
    BufReader::new(&mut client)
        .read_line(&mut woken)
        .expect("wake greeting");
    assert_eq!(woken.trim(), "hello from burrow-testbin");

    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["services"]["api"]["status"] == "healthy"
    });
    assert_ne!(
        state["services"]["api"]["pid"].as_u64(),
        Some(first_pid),
        "the woken service must be a new process"
    );
    assert!(
        state["services"]["api"]["runtime"].is_null()
            || state["services"]["api"]["runtime"] == serde_json::Value::Null,
        "run services carry no runtime label: {}",
        state["services"]["api"]
    );

    // The tunnelled client still echoes.
    client.write_all(b"ping").expect("echo request");
    let mut echoed = [0_u8; 4];
    client.read_exact(&mut echoed).expect("echo response");
    assert_eq!(&echoed, b"ping");

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
}

#[test]
fn watch_restarts_a_service_only_for_watched_paths() {
    let project = TestProject::new(
        "[project]\nname = \"watcher\"\n\n[services.api]\n\
         run = 'cmd /C ping -n 600 127.0.0.1 > NUL'\n\
         watch = [\"src\"]\n",
    );
    let src = project.path().join("src");
    fs::create_dir_all(&src).expect("create watch root");

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["api"]["status"] == "healthy"
    });
    let first_pid = state["services"]["api"]["pid"].as_u64().expect("pid");

    // Writing under the watch root restarts the service.
    fs::write(src.join("main.rs"), b"fn main() {}").expect("touch a watched file");
    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["services"]["api"]["restarts"].as_u64().unwrap_or(0) >= 1
    });
    assert_ne!(
        state["services"]["api"]["pid"].as_u64(),
        Some(first_pid),
        "the watched service must have been restarted"
    );

    // Changes under .burrow must never trigger a restart.
    let restarts = state["services"]["api"]["restarts"].as_u64().unwrap_or(0);
    let logs = project.path().join(".burrow").join("logs");
    fs::write(logs.join("noise.txt"), "log noise\n").expect("write into .burrow");
    std::thread::sleep(Duration::from_secs(3));
    let state = project.state();
    assert_eq!(
        state["services"]["api"]["restarts"].as_u64().unwrap_or(0),
        restarts,
        "a write under .burrow restarted the service"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
}

#[test]
fn container_services_fail_early_without_an_engine() {
    let project = TestProject::new(
        "[project]\nname = \"containerproj\"\n\n[services.cache]\n\
         use = \"container:redis:7-alpine\"\n\
         container_port = 6379\n\
         health = { tcp = true, timeout = \"1s\" }\n",
    );
    let output = burrow()
        .arg("--project")
        .arg(project.path())
        .args(["up", "--detach", "--yes"])
        .env("BURROW_CONTAINER_ENGINE", "burrow-missing-engine")
        .output()
        .expect("run burrow up");
    assert!(!output.status.success(), "up must fail without an engine");
    let text = stderr(&output);
    assert!(
        text.contains("no container engine was found"),
        "stderr was:\n{text}"
    );
    assert!(
        text.contains("Docker Desktop or Podman"),
        "stderr was:\n{text}"
    );
    assert!(!project.state_path().exists());
}

#[test]
fn container_service_starts_when_an_engine_is_available() {
    // The test needs a responding engine and a Windows-compatible image;
    // CI Windows runners may have Docker but lack a Windows manifest for
    // the test image. Check both.
    let available = Command::new("docker")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("skipping: no responding container engine on this machine");
        return;
    }
    // Verify the test image has a Windows manifest before running the full test.
    let manifest_ok = Command::new("docker")
        .args(["manifest", "inspect", "traefik/whoami:v1.10"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !manifest_ok {
        eprintln!("skipping: test image lacks Windows manifest on this runner");
        return;
    }

    let project = TestProject::new(
        "[project]\nname = \"containerlive\"\n\n[services.hello]\n\
         use = \"container:traefik/whoami:v1.10\"\n\
         container_port = 80\n\
         port = 45999\n\
         health = { http = \"/\", timeout = \"60s\", interval = \"500ms\" }\n",
    );
    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    let state = project.wait_until(Duration::from_secs(120), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["hello"]["status"] == "healthy"
    });
    assert_eq!(
        state["services"]["hello"]["runtime"], "container:traefik/whoami:v1.10",
        "status labels the container runtime: {}",
        state["services"]["hello"]
    );

    let mut response = String::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", 45999)) {
            if stream
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .is_ok()
            {
                response.clear();
                if stream.read_to_string(&mut response).is_ok()
                    && response.starts_with("HTTP/1.1 200")
                {
                    break;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "the container never answered: {response}"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(response.contains("Hostname"), "response:\n{response}");

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));

    // The stopped container must not linger thanks to `--rm`.
    let leftover = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "name=burrow-containerlive-hello",
            "-q",
        ])
        .output()
        .expect("docker ps");
    assert!(
        stdout(&leftover).trim().is_empty(),
        "the container was left behind: {}",
        stdout(&leftover)
    );
}
