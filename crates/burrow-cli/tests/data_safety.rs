//! Data-safety tests: what happens when a crashed machine leaves `.burrow`
//! damaged.
//!
//! These tests assert the *real* product behavior, even where it is stricter
//! than "never wedge the project":
//!
//! - A corrupt `state.json` is never silently discarded. Both `burrow down`
//!   and `burrow up` fail loudly with `is not valid Burrow state` until the
//!   file is removed by hand; once it is gone the project comes up cleanly.
//! - A corrupt `ports.json` is rejected by the port allocator instead of being
//!   silently rebuilt, so `burrow up` fails with an actionable error that names
//!   the file. Deleting the file recovers, and the next cycle is clean.
//! - A half-written snapshot lives in a `.burrow-tmp-*` staging directory that
//!   is never listed, never restored, and never blocks a later snapshot.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

    fn state(&self) -> Value {
        let contents = fs::read_to_string(self.state_path()).expect("read state");
        serde_json::from_str(&contents).expect("parse state")
    }

    fn wait_until(&self, timeout: Duration, mut predicate: impl FnMut(&Value) -> bool) -> Value {
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

    fn wait_until_healthy(&self) {
        self.wait_until(Duration::from_secs(20), |state| {
            state["ready"].as_bool() == Some(true)
                && state["services"]["api"]["status"] == "healthy"
        });
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = run_in(self.path(), &["down"]);
        if self.state_path().exists() {
            // Last-resort cleanup: closing the daemon's job handles kills every
            // service because KILL_ON_JOB_CLOSE is set.
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

fn ping_run() -> &'static str {
    "run = 'cmd /C ping -n 30 127.0.0.1 > NUL'"
}

#[test]
fn corrupt_state_json_is_recovered_not_fatal() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"corruptstate\"\n\n[services.api]\n{}\n",
        ping_run()
    ));
    fs::create_dir_all(project.burrow_dir()).expect("create .burrow");
    // A crash mid-write leaves truncated JSON: readable text that is not valid
    // state, exactly what `RuntimeState::read` must refuse to interpret.
    fs::write(
        project.state_path(),
        r#"{"version": 1, "project": "corruptstate""#,
    )
    .expect("write corrupt state");

    // Real behavior: `down` refuses to guess and fails loudly instead of
    // deleting a file it could not parse.
    let down = run_in(project.path(), &["down"]);
    assert!(
        !down.status.success(),
        "`down` with a corrupt state.json must fail, got:\n{}",
        stdout(&down)
    );
    assert!(
        stderr(&down).contains("is not valid Burrow state"),
        "stderr was:\n{}",
        stderr(&down)
    );
    assert!(
        project.state_path().exists(),
        "`down` must not delete a state file it could not read"
    );

    // Real behavior: `up` fails with the same actionable message rather than
    // silently discarding the previous daemon's record.
    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        !up.status.success(),
        "`up` with a corrupt state.json must fail, got:\n{}",
        stdout(&up)
    );
    assert!(
        stderr(&up).contains("is not valid Burrow state"),
        "stderr was:\n{}",
        stderr(&up)
    );

    // Recovery is the documented remedy: remove the corrupt file, and the
    // project must come up healthy and stop cleanly afterwards.
    fs::remove_file(project.state_path()).expect("remove corrupt state");
    let started = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        started.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&started),
        stderr(&started)
    );
    project.wait_until_healthy();

    let stopped = run_in(project.path(), &["down"]);
    assert!(
        stopped.status.success(),
        "down failed:\n{}",
        stderr(&stopped)
    );
    assert!(!project.state_path().exists());
}

#[test]
fn truncated_ports_json_recovers() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"corruptports\"\n\n[services.api]\n{}\n",
        ping_run()
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until_healthy();
    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
    assert!(!project.state_path().exists());

    let ports_path = project.burrow_dir().join("ports.json");
    assert!(
        ports_path.exists(),
        "the first cycle must remember port assignments"
    );
    // A crash mid-write leaves a truncated document: valid text, invalid JSON.
    fs::write(&ports_path, r#"{"version":1,"ports":{"api":"#).expect("corrupt ports.json");

    // Real behavior: `PortAllocator::load` rejects malformed content instead of
    // silently rebuilding it, so the daemon refuses to start and the failure is
    // reported with the file name on the command surface (via the daemon log
    // tail that `up --detach` prints).
    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        !up.status.success(),
        "`up` with a corrupt ports.json must fail, got:\n{}",
        stdout(&up)
    );
    let text = stderr(&up);
    assert!(
        text.contains("the Burrow daemon exited during startup"),
        "stderr was:\n{text}"
    );
    assert!(
        text.contains("invalid port assignment file"),
        "stderr was:\n{text}"
    );
    let daemon_log =
        fs::read_to_string(project.burrow_dir().join("daemon.log")).expect("read daemon log");
    assert!(
        daemon_log.contains("invalid port assignment file"),
        "daemon log was:\n{daemon_log}"
    );
    assert!(
        !project.state_path().exists(),
        "a failed `up` must not leave state behind"
    );

    // Recovery: deleting the corrupt file lets the allocator start empty, and
    // the project must end this test stopped and clean.
    fs::remove_file(&ports_path).expect("remove corrupt ports.json");
    let started = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        started.status.success(),
        "recovery up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&started),
        stderr(&started)
    );
    project.wait_until_healthy();

    let stopped = run_in(project.path(), &["down"]);
    assert!(
        stopped.status.success(),
        "down failed:\n{}",
        stderr(&stopped)
    );
    assert!(!project.state_path().exists());
}

#[test]
fn half_written_snapshot_is_never_listed() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"halffull\"\n\n[services.api]\n{}\n",
        ping_run()
    ));
    let data = project.burrow_dir().join("data").join("api");
    fs::create_dir_all(&data).expect("create data directory");
    fs::write(data.join("record.txt"), "before\n").expect("write record");

    let good = run_in(project.path(), &["snapshot", "good", "--json"]);
    assert!(good.status.success(), "{}", stderr(&good));
    let manifest: Value = serde_json::from_str(&stdout(&good)).expect("snapshot JSON");
    assert_eq!(manifest["name"], "good");
    assert_eq!(manifest["services"][0]["present"], true);

    // Simulate a crash in the middle of `burrow snapshot second`. The staging
    // code copies into `.burrow-tmp-<name>-<pid>-<unix>` inside snapshots/ and
    // only renames it into place after the manifest is written, so the debris
    // a crash leaves behind has exactly this name and no manifest.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_secs();
    let staging = project
        .burrow_dir()
        .join("snapshots")
        .join(format!(".burrow-tmp-second-{}-{nonce}", std::process::id()));
    fs::create_dir_all(staging.join("data").join("api")).expect("create fake staging");
    fs::write(
        staging.join("data").join("api").join("record.txt"),
        "half-written",
    )
    .expect("write partial copy");

    // Corrupt the live data so the restore below has something to fix.
    fs::write(data.join("record.txt"), "corrupted\n").expect("mutate record");

    let list = run_in(project.path(), &["snapshots", "--json"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let listed: Vec<Value> = serde_json::from_str(&stdout(&list)).expect("snapshots JSON");
    let names: Vec<&str> = listed
        .iter()
        .filter_map(|snapshot| snapshot["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["good"],
        "the half-written staging directory must not be listed"
    );

    let restore = run_in(project.path(), &["restore", "good", "--json"]);
    assert!(restore.status.success(), "{}", stderr(&restore));
    assert_eq!(
        fs::read_to_string(data.join("record.txt")).expect("read record"),
        "before\n"
    );

    // The debris must not block a later snapshot either.
    let second = run_in(project.path(), &["snapshot", "second", "--json"]);
    assert!(second.status.success(), "{}", stderr(&second));

    let list = run_in(project.path(), &["snapshots", "--json"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let listed: Vec<Value> = serde_json::from_str(&stdout(&list)).expect("snapshots JSON");
    let mut names: Vec<&str> = listed
        .iter()
        .filter_map(|snapshot| snapshot["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["good", "second"],
        "both snapshots must be listed"
    );
}
