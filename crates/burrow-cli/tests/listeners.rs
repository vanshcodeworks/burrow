//! `doctor` listener-inspection tests.
//!
//! A supervised service that binds `0.0.0.0` without `expose = true` is a
//! real exposure risk: the service is reachable from the network even though
//! the reverse proxy is loopback-only. `doctor` inspects the actual sockets
//! owned by the service process tree and warns.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

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

    fn state(&self) -> Value {
        let contents = fs::read_to_string(self.state_path()).expect("read state");
        serde_json::from_str(&contents).expect("parse state")
    }

    fn wait_until_healthy(&self) {
        let started = Instant::now();
        loop {
            if self.state_path().exists() {
                let state = self.state();
                if state["ready"].as_bool() == Some(true)
                    && state["services"]["api"]["status"] == "healthy"
                {
                    return;
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "service did not become healthy"
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
                if let Ok(state) = serde_json::from_str::<Value>(&contents) {
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

fn config(name: &str, run: &str, extra: &str) -> String {
    format!(
        "[project]\nname = \"{name}\"\n\n[services.api]\nrun = '{run}'\n{extra}health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n"
    )
}

fn listener_check(report: &Value) -> &Value {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == "listeners")
        .expect("listeners check")
}

fn doctor_json(project: &TestProject) -> Value {
    let doctor = run_in(project.path(), &["doctor", "--json"]);
    assert!(
        doctor.status.success(),
        "doctor failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&doctor),
        stderr(&doctor)
    );
    serde_json::from_str(&stdout(&doctor)).expect("doctor JSON")
}

#[test]
fn doctor_warns_when_a_service_binds_the_wildcard_without_expose() {
    let project = TestProject::new(&config(
        "wildcard",
        &format!("{} bind 0.0.0.0", testbin().display()),
        "",
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until_healthy();

    let report = doctor_json(&project);
    let check = listener_check(&report);
    assert_eq!(check["status"], "warn", "check was: {check}");
    let detail = check["detail"].as_str().expect("detail");
    assert!(detail.contains("api"), "detail was: {detail}");
    assert!(detail.contains("0.0.0.0"), "detail was: {detail}");
    assert!(
        check["remedy"]
            .as_str()
            .is_some_and(|remedy| remedy.contains("expose = true")),
        "check was: {check}"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
}

#[test]
fn doctor_accepts_a_wildcard_bind_when_the_service_is_explicitly_exposed() {
    let project = TestProject::new(&config(
        "exposed",
        &format!("{} bind 0.0.0.0", testbin().display()),
        "expose = true\n",
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until_healthy();

    let report = doctor_json(&project);
    let check = listener_check(&report);
    assert_eq!(check["status"], "ok", "check was: {check}");
    assert!(
        check["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("none accept non-loopback traffic")),
        "check was: {check}"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
}

#[test]
fn doctor_accepts_a_loopback_only_service() {
    let project = TestProject::new(&config(
        "loopback",
        &format!("{} serve", testbin().display()),
        "",
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until_healthy();

    let report = doctor_json(&project);
    let check = listener_check(&report);
    assert_eq!(check["status"], "ok", "check was: {check}");

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
}
