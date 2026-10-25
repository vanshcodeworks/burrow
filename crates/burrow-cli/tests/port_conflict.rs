//! Adversarial end-to-end test: two projects fighting over one fixed port.
//!
//! Two Burrow projects running at the same time must not share ports. A fixed
//! port (`port = 45123`) is validated as free before anything starts, so the
//! second project with the same fixed port must fail with an actionable
//! remedy, start nothing, leave no state behind, and leave the first project
//! untouched. Windows only, like the rest of Burrow.

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

/// A one-service project whose service pins exactly `port` and whose health is
/// a real TCP connect, so the port is genuinely bound once A is up.
fn fixed_port_config(name: &str, port: u16) -> String {
    format!(
        "[project]\nname = \"{name}\"\n\n[services.api]\n{}\n\
         port = {port}\n\
         health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n",
        powershell_listener()
    )
}

/// First fixed-port candidate tried for project A.
const FIRST_CANDIDATE: u16 = 45_123;
/// Candidates tried: 45123..=45130.
const CANDIDATE_COUNT: u16 = 8;

#[test]
fn a_fixed_port_conflict_fails_the_second_project_with_a_remedy() {
    // The fixed port must be genuinely free before project A claims it. Probe
    // each candidate with the same check the daemon uses, then prove the
    // daemon accepted it by starting A; a candidate snatched by an unrelated
    // process between the two is skipped, so the test never fails because of
    // the machine it runs on.
    let mut claimed = None;
    let mut last_conflict = String::new();
    for candidate in FIRST_CANDIDATE..FIRST_CANDIDATE + CANDIDATE_COUNT {
        if !burrow_net::is_free(candidate) {
            continue;
        }
        let project = TestProject::new(&fixed_port_config("port-a", candidate));
        let up = run_in(project.path(), &["up", "--detach", "--yes"]);
        if up.status.success() {
            project.wait_until(Duration::from_secs(20), |state| {
                state["ready"].as_bool() == Some(true)
                    && state["services"]["api"]["status"] == "healthy"
            });
            let bound = project.state()["services"]["api"]["port"]
                .as_u64()
                .expect("service port") as u16;
            assert_eq!(bound, candidate, "the daemon must honor the fixed port");
            claimed = Some((project, candidate));
            break;
        }
        let text = stderr(&up);
        assert!(
            text.contains("already in use"),
            "project A failed for a reason other than a busy port:\nstdout:\n{}\nstderr:\n{}",
            stdout(&up),
            text
        );
        last_conflict = text;
    }
    let (project_a, port) = claimed.unwrap_or_else(|| {
        panic!(
            "every candidate fixed port {FIRST_CANDIDATE}-{} was taken by unrelated \
             processes; last conflict:\n{last_conflict}",
            FIRST_CANDIDATE + CANDIDATE_COUNT - 1
        )
    });

    // Project A is live and its service really owns the fixed port.
    let daemon_pid = project_a.state()["pid"].as_u64().expect("daemon pid");
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "project A's service must be listening on 127.0.0.1:{port}"
    );

    // Project B: a separate project, separate directory, same fixed port.
    let project_b = TestProject::new(&fixed_port_config("port-b", port));
    let up = run_in(project_b.path(), &["up", "--detach", "--yes"]);
    assert!(
        !up.status.success(),
        "the second project must not start on a taken fixed port; stdout:\n{}",
        stdout(&up)
    );
    let text = stderr(&up);
    assert!(
        text.contains(port.to_string().as_str()),
        "stderr must name the busy port {port}:\n{text}"
    );
    assert!(
        text.contains("already in use"),
        "stderr must say the port is in use:\n{text}"
    );
    assert!(
        text.contains("choose another port") && text.contains(r#"port = "auto""#),
        "stderr must include an actionable remedy:\n{text}"
    );
    assert!(
        !project_b.state_path().exists(),
        "a failed up must not leave a state file behind"
    );

    // The conflict must not have disturbed project A.
    let state = project_a.state();
    assert_eq!(
        state["pid"].as_u64(),
        Some(daemon_pid),
        "project A's daemon must survive the failed second project"
    );
    assert_eq!(
        state["services"]["api"]["status"], "healthy",
        "project A's service must stay healthy: {}",
        state["services"]["api"]
    );
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "project A's port must still accept connections after the conflict"
    );

    // The failure was purely the collision: the same project B starts once its
    // config pins a different, currently free fixed port.
    let other = (FIRST_CANDIDATE..FIRST_CANDIDATE + CANDIDATE_COUNT)
        .find(|&candidate| candidate != port && burrow_net::is_free(candidate))
        .expect("no second candidate port was free");
    fs::write(
        project_b.path().join("burrow.toml"),
        fixed_port_config("port-b", other),
    )
    .expect("rewrite burrow.toml");
    let up = run_in(project_b.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "project B must start on a free fixed port:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project_b.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true) && state["services"]["api"]["status"] == "healthy"
    });
    assert_eq!(
        project_b.state()["services"]["api"]["port"].as_u64(),
        Some(u64::from(other)),
        "project B must run on its own fixed port"
    );

    // Clean down both projects; Drop is the safety net for failed assertions.
    let down_b = run_in(project_b.path(), &["down"]);
    assert!(
        down_b.status.success(),
        "down failed for project B:\n{}",
        stderr(&down_b)
    );
    assert!(!project_b.state_path().exists());
    let down_a = run_in(project_a.path(), &["down"]);
    assert!(
        down_a.status.success(),
        "down failed for project A:\n{}",
        stderr(&down_a)
    );
    assert!(!project_a.state_path().exists());
}
