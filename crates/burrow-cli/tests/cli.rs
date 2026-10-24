//! End-to-end CLI tests against a real daemon and real child processes.
//!
//! Every test uses its own temporary project, so ports and state never collide
//! with another test. Windows only, like the rest of Burrow.

use std::fs;
use std::io::{BufRead, BufReader, Write};
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

    fn burrow_dir(&self) -> PathBuf {
        self.path().join(".burrow")
    }

    fn state_path(&self) -> PathBuf {
        self.burrow_dir().join("state.json")
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
            // service because KILL_ON_JOB_CLOSE is set.
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

#[test]
fn version_reports_package_version() {
    let output = burrow().arg("--version").output().expect("run burrow");
    assert!(output.status.success());
    assert!(stdout(&output).contains("burrow 0.1.0"));
}

#[test]
fn help_lists_the_full_command_surface() {
    let output = burrow().arg("--help").output().expect("run burrow");
    assert!(output.status.success());
    let text = stdout(&output);
    for command in [
        "init",
        "up",
        "down",
        "restart",
        "status",
        "logs",
        "open",
        "env",
        "exec",
        "doctor",
        "top",
        "pkg",
        "snapshot",
        "restore",
        "snapshots",
        "reset",
        "schema",
    ] {
        assert!(
            text.contains(command),
            "help is missing `{command}`:\n{text}"
        );
    }
}

#[test]
fn up_without_a_project_reports_the_missing_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = run_in(dir.path(), &["up", "--yes"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("burrow.toml"), "stderr was:\n{text}");
    assert!(text.contains("burrow init"), "stderr was:\n{text}");
}

fn run_with_cache(dir: &Path, cache: &Path, args: &[&str]) -> Output {
    burrow()
        .arg("--project")
        .arg(dir)
        .args(args)
        .env("BURROW_CACHE_DIR", cache)
        .output()
        .expect("failed to execute burrow")
}

#[test]
fn pkg_list_gc_and_verify_work_on_an_empty_cache() {
    let dir = tempfile::tempdir().expect("temp dir");
    let cache = dir.path().join("cache");

    let list = run_with_cache(dir.path(), &cache, &["pkg", "list"]);
    assert!(list.status.success(), "stderr:\n{}", stderr(&list));
    assert!(stdout(&list).contains("cache is empty"));

    let gc = run_with_cache(dir.path(), &cache, &["pkg", "gc"]);
    assert!(gc.status.success(), "stderr:\n{}", stderr(&gc));
    assert!(stdout(&gc).contains("already clean"));

    let verify = run_with_cache(dir.path(), &cache, &["pkg", "verify"]);
    assert!(verify.status.success(), "stderr:\n{}", stderr(&verify));
    assert!(stdout(&verify).contains("healthy"));
}

#[test]
fn pkg_add_rejects_unknown_packages_before_downloading() {
    let project =
        TestProject::new("[project]\nname = \"pkgs\"\n\n[services.api]\nrun = 'cmd /C exit 0'\n");
    let cache = project.path().join("cache");
    let output = run_with_cache(project.path(), &cache, &["pkg", "add", "nope"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("unknown package"), "stderr was:\n{text}");
    assert!(!cache.join("archives").exists());
}

#[test]
fn pkg_add_rejects_a_missing_seed_file() {
    let project =
        TestProject::new("[project]\nname = \"pkgs\"\n\n[services.api]\nrun = 'cmd /C exit 0'\n");
    let cache = project.path().join("cache");
    let output = run_with_cache(
        project.path(),
        &cache,
        &["pkg", "add", "postgres@16", "--seed", "missing.sql"],
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("does not exist"), "stderr was:\n{text}");
    assert!(!cache.join("archives").exists());
}

#[test]
fn pkg_remove_edits_the_config_without_downloading() {
    let project = TestProject::new(
        "[project]\nname = \"pkgs\"\n\n[services.api]\nrun = 'cmd /C exit 0'\n\n\
         [services.db]\nuse = \"postgres@16\"\n",
    );
    let cache = project.path().join("cache");
    let output = run_with_cache(project.path(), &cache, &["pkg", "remove", "db"]);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    let config = fs::read_to_string(project.path().join("burrow.toml")).expect("read config");
    assert!(!config.contains("[services.db]"), "config was:\n{config}");
    assert!(config.contains("[services.api]"), "config was:\n{config}");
    assert!(!cache.join("archives").exists());

    let run_service = run_with_cache(project.path(), &cache, &["pkg", "remove", "api"]);
    assert!(!run_service.status.success());
    assert!(stderr(&run_service).contains("runs a command"));

    let missing = run_with_cache(project.path(), &cache, &["pkg", "remove", "nope"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("no service named"));
}

#[test]
fn env_json_reports_not_implemented() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = run_in(dir.path(), &["env", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("not implemented yet"));
}

#[test]
fn trust_is_required_without_yes() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"trust\"\n\n[services.api]\n{}\n",
        powershell_listener()
    ));
    let output = run_in(project.path(), &["up", "--detach"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("has not been approved"),
        "stderr was:\n{text}"
    );
    assert!(!project.state_path().exists());
}

#[test]
fn unknown_service_fails_before_starting_anything() {
    let project = TestProject::new(
        "[project]\nname = \"unknown\"\n\n[services.api]\nrun = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n",
    );
    let output = run_in(project.path(), &["up", "--yes", "nope"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("unknown service `nope`"),
        "stderr was:\n{text}"
    );
    assert!(!project.state_path().exists());
}

#[test]
fn daemon_lifecycle_covers_status_logs_auth_restart_and_down() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"smoke\"\n\n[services.api]\n{}\n\
         port = \"auto\"\n\
         health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n",
        powershell_listener()
    ));

    let output = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        output.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );

    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["api"]["status"] == "healthy"
    });
    let port = state["control_port"].as_u64().expect("control port") as u16;
    let token = state["token"].as_str().expect("token");

    // A wrong bearer token is rejected.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to control socket");
    let request = r#"{"version":1,"token":"wrong","command":{"type":"status"}}"#;
    stream.write_all(request.as_bytes()).expect("write request");
    stream.write_all(b"\n").expect("newline");
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .expect("read response");
    let response: serde_json::Value = serde_json::from_str(&response).expect("parse response");
    assert_eq!(response["ok"], false);
    assert!(
        response["message"]
            .as_str()
            .is_some_and(|message| message.contains("unauthorized")),
        "response was: {response}"
    );

    let status = run_in(project.path(), &["--json", "status"]);
    assert!(
        status.status.success(),
        "status failed:\n{}",
        stderr(&status)
    );
    let status_json: serde_json::Value =
        serde_json::from_str(&stdout(&status)).expect("status JSON");
    assert_eq!(status_json["services"]["api"]["status"], "healthy");
    assert_eq!(status_json["token"], serde_json::Value::Null);
    assert_eq!(
        status_json["services"]["api"]["port"],
        state["services"]["api"]["port"]
    );

    // The service printed a line and it is available through the daemon.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let logs = run_in(project.path(), &["--json", "logs"]);
        assert!(logs.status.success(), "logs failed:\n{}", stderr(&logs));
        let entries: serde_json::Value = serde_json::from_str(&stdout(&logs)).expect("logs JSON");
        if entries
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["text"] == "ready"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "service output was not captured");
        std::thread::sleep(Duration::from_millis(200));
    }

    let restart = run_in(project.path(), &["restart", "api"]);
    assert!(
        restart.status.success(),
        "restart failed:\n{}",
        stderr(&restart)
    );
    project.wait_until(Duration::from_secs(20), |state| {
        state["services"]["api"]["status"] == "healthy"
            && state["services"]["api"]["restarts"].as_u64().unwrap_or(0) >= 1
    });

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
    assert!(!project.state_path().exists());

    // Ensure the control token really came from the daemon, not a default.
    assert!(token.len() >= 32, "unexpected token: {token}");
}

#[test]
fn a_failed_health_check_stops_the_daemon_and_leaves_no_state() {
    let project = TestProject::new(
        "[project]\nname = \"sick\"\n\n[services.api]\n\
         run = 'powershell -NoProfile -NonInteractive -Command \"Write-Output starting; Start-Sleep -Seconds 30\"'\n\
         port = \"auto\"\n\
         health = { tcp = true, timeout = \"1s\", interval = \"100ms\", retries = 3 }\n",
    );

    let started = Instant::now();
    let output = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("did not become healthy"),
        "stderr was:\n{text}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "health timeout took too long"
    );
    assert!(!project.state_path().exists());

    let log = fs::read_to_string(project.burrow_dir().join("logs").join("api.log"))
        .expect("service log exists");
    assert!(log.contains("[stdout]"), "log was:\n{log}");
}

#[test]
fn a_crashing_service_is_restarted_with_backoff() {
    let project = TestProject::new(
        "[project]\nname = \"crashy\"\n\n[services.api]\n\
         run = 'cmd /C exit 7'\n\
         restart = \"on-failure\"\n",
    );

    let output = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        output.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );

    project.wait_until(Duration::from_secs(15), |state| {
        state["services"]["api"]["restarts"].as_u64().unwrap_or(0) >= 1
    });

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
}

#[test]
fn down_removes_state_left_by_a_dead_daemon() {
    let project = TestProject::new(
        "[project]\nname = \"stale\"\n\n[services.api]\nrun = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n",
    );
    fs::create_dir_all(project.burrow_dir()).expect("create .burrow");
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        listener.local_addr().expect("address").port()
    };
    let state = serde_json::json!({
        "version": 1,
        "project": "stale",
        "project_dir": project.path(),
        "pid": 4_000_000_000u32,
        "token": "dead",
        "control_port": port,
        "started_at": 0,
        "config_hash": "dead",
        "ready": false,
        "services": {},
    });
    fs::write(project.state_path(), state.to_string()).expect("write state");

    let output = run_in(project.path(), &["down"]);
    assert!(output.status.success(), "down failed:\n{}", stderr(&output));
    assert!(stdout(&output).contains("removed stale state"));
    assert!(!project.state_path().exists());
}
