//! Adversarial end-to-end tests: abuse the daemon itself and prove the
//! Windows Job Object safety net plus stale-state recovery hold through the
//! real CLI, not just the supervisor layer.
//!
//! Like `cli.rs`, every test uses its own temporary project, so ports and
//! state never collide with another test. Windows only, like the rest of
//! Burrow.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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
            // Last-resort cleanup: closing the daemon's job handles kills every
            // service because KILL_ON_JOB_CLOSE is set. The state file carries
            // the pid of the daemon that is running right now.
            if let Ok(contents) = fs::read_to_string(self.state_path()) {
                if let Ok(state) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(pid) = state["pid"].as_u64() {
                        let _ = Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                }
            }
        }
    }
}

fn powershell_listener() -> &'static str {
    r#"run = 'powershell -NoProfile -NonInteractive -Command "$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,[int]$env:PORT);$l.Start();Write-Output ready;while($true){Start-Sleep -Seconds 1}"'"#
}

/// Diagnostic for a failed no-orphans proof: list the survivor candidates so
/// the failure says *what* leaked, not just *that* something leaked.
fn process_listing() -> String {
    let mut text = String::new();
    for image in ["powershell.exe", "burrow.exe"] {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV"])
            .output();
        match output {
            Ok(output) => text.push_str(&format!(
                "tasklist {image}:\n{}\n",
                String::from_utf8_lossy(&output.stdout)
            )),
            Err(error) => text.push_str(&format!("tasklist {image} failed: {error}\n")),
        }
    }
    text
}

#[test]
fn a_force_killed_daemon_leaves_no_orphans_and_the_project_recovers() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"adversary\"\n\n[services.api]\n{}\n\
         port = \"auto\"\n\
         health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n",
        powershell_listener()
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
    let daemon_pid = state["pid"].as_u64().expect("daemon pid") as u32;
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "the service must be listening on 127.0.0.1:{port} before the daemon dies"
    );

    // Kill only the daemon. Without /T, taskkill cannot touch the service;
    // the kernel must kill it when the daemon's job handles close.
    let kill = Command::new("taskkill")
        .args(["/F", "/PID", &daemon_pid.to_string()])
        .output()
        .expect("run taskkill");
    assert!(
        kill.status.success(),
        "taskkill /F /PID {daemon_pid} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&kill.stdout),
        String::from_utf8_lossy(&kill.stderr)
    );

    // The no-orphans proof: the port must refuse connections once the killed
    // daemon's job handles are closed. A refused connect is the only accepted
    // signal; timeouts (a full accept queue, a loaded machine) keep polling.
    let started = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => break,
            Err(_) => {}
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "127.0.0.1:{port} is still accepting connections 15s after its \
             daemon (pid {daemon_pid}) was force-killed; the service orphaned:\n{}",
            process_listing()
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // The daemon died mid-flight, so nothing removed its state file.
    assert!(
        project.state_path().exists(),
        "a force-killed daemon must leave its state file behind for the next up"
    );

    // Recovery: the leftover state is stale because nothing authenticates on
    // the control port, so the next `up` removes it and starts fresh.
    let again = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        again.status.success(),
        "up after the crash failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&again),
        stderr(&again)
    );

    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["api"]["status"] == "healthy"
    });
    assert_eq!(
        state["services"]["api"]["port"].as_u64(),
        Some(u64::from(port)),
        "ports.json must hand the recovered service the same port"
    );
    assert_ne!(
        state["pid"].as_u64(),
        Some(u64::from(daemon_pid)),
        "the recovered daemon must be a new process"
    );
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "the recovered service must be listening again on 127.0.0.1:{port}"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(
        down.status.success(),
        "down failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&down),
        stderr(&down)
    );
    assert!(!project.state_path().exists());
}
