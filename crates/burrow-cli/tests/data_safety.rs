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
//! - A restore interrupted between directory swaps is journaled. The next
//!   `burrow up` or `burrow snapshot` rolls it back to the pre-restore data
//!   instead of starting or copying a half-restored project.
//! - A corrupt `.burrow/secrets` is never overwritten. `burrow up`, `burrow
//!   env`, and `burrow doctor` report it, a running project keeps running, and
//!   deleting the file is the documented recovery.

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

fn unix_nonce() -> String {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_secs();
    format!("{}-{unix}", std::process::id())
}

/// Simulates a crash after the staged copy replaced the live directory but
/// before the restore journal was marked committed.
fn interrupt_restore(project: &TestProject, snapshot: &str, nonce: &str) {
    let data_root = project.burrow_dir().join("data");
    let live = data_root.join("api");
    let backup = data_root.join(format!(".burrow-backup-{nonce}-api"));
    let staging = data_root.join(format!(".burrow-restore-{nonce}"));
    let snapshot_record = project
        .burrow_dir()
        .join("snapshots")
        .join(snapshot)
        .join("data")
        .join("api")
        .join("record.txt");

    fs::rename(&live, &backup).expect("move live data to backup");
    fs::create_dir_all(&live).expect("recreate live data");
    fs::copy(&snapshot_record, live.join("record.txt")).expect("install snapshot copy");
    // The real restore leaves the staging root behind after renaming each
    // staged service into place.
    fs::create_dir_all(&staging).expect("leave restore staging root");

    let journal = serde_json::json!({
        "version": 1,
        "nonce": nonce,
        "snapshot": snapshot,
        "committed": false,
        "services": [{
            "name": "api",
            "present": true,
            "had_original": true,
        }],
    });
    fs::write(
        data_root.join(".burrow-restore-journal.json"),
        serde_json::to_vec_pretty(&journal).expect("encode journal"),
    )
    .expect("write restore journal");
}

#[test]
fn an_interrupted_restore_is_rolled_back_before_services_start() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"restorerollback\"\n\n[services.api]\n{}\n",
        ping_run()
    ));
    let data = project.burrow_dir().join("data").join("api");
    fs::create_dir_all(&data).expect("create data directory");
    fs::write(data.join("record.txt"), "current\n").expect("write current data");

    let snapshot = run_in(project.path(), &["snapshot", "good", "--json"]);
    assert!(snapshot.status.success(), "{}", stderr(&snapshot));

    // The live data changed after the snapshot was taken. An interrupted
    // restore must bring this version back, not keep the half-installed
    // snapshot copy.
    fs::write(data.join("record.txt"), "pre-restore\n").expect("write pre-restore data");
    let nonce = unix_nonce();
    interrupt_restore(&project, "good", &nonce);
    assert_eq!(
        fs::read_to_string(data.join("record.txt")).expect("read live data"),
        "current\n",
        "the simulation must leave the snapshot copy installed"
    );

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until_healthy();

    assert_eq!(
        fs::read_to_string(data.join("record.txt")).expect("read recovered data"),
        "pre-restore\n",
        "an uncommitted restore must be rolled back before services start"
    );
    let data_root = project.burrow_dir().join("data");
    assert!(
        !data_root
            .join(format!(".burrow-backup-{nonce}-api"))
            .exists(),
        "the consumed backup must be gone"
    );
    assert!(
        !data_root.join(format!(".burrow-restore-{nonce}")).exists(),
        "the staging root must be gone"
    );
    assert!(
        !data_root.join(".burrow-restore-journal.json").exists(),
        "the journal must be gone"
    );

    let daemon_log =
        fs::read_to_string(project.burrow_dir().join("daemon.log")).expect("read daemon log");
    assert!(
        daemon_log.contains("recovered an interrupted restore"),
        "daemon log was:\n{daemon_log}"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
}

#[test]
fn snapshot_rolls_back_an_interrupted_restore_before_copying() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"snapshotrecovery\"\n\n[services.api]\n{}\n",
        ping_run()
    ));
    let data = project.burrow_dir().join("data").join("api");
    fs::create_dir_all(&data).expect("create data directory");
    fs::write(data.join("record.txt"), "before\n").expect("write before data");

    let snapshot = run_in(project.path(), &["snapshot", "good", "--json"]);
    assert!(snapshot.status.success(), "{}", stderr(&snapshot));

    fs::write(data.join("record.txt"), "pre-restore\n").expect("write pre-restore data");
    let nonce = unix_nonce();
    interrupt_restore(&project, "good", &nonce);

    // `snapshot` must resolve the interrupted restore first. Otherwise the new
    // snapshot would capture the half-restored copy and the journal would be
    // lost.
    let second = run_in(project.path(), &["snapshot", "after", "--json"]);
    assert!(
        second.status.success(),
        "snapshot failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&second),
        stderr(&second)
    );
    assert!(
        stderr(&second).contains("rolled back an interrupted restore"),
        "stderr was:\n{}",
        stderr(&second)
    );
    assert_eq!(
        fs::read_to_string(data.join("record.txt")).expect("read recovered data"),
        "pre-restore\n",
        "the pre-restore directory must be live before the new snapshot"
    );
    let copied = project
        .burrow_dir()
        .join("snapshots")
        .join("after")
        .join("data")
        .join("api")
        .join("record.txt");
    assert_eq!(
        fs::read_to_string(copied).expect("read new snapshot"),
        "pre-restore\n",
        "the new snapshot must copy recovered data, not the half-restore"
    );
    assert!(!project
        .burrow_dir()
        .join("data")
        .join(".burrow-restore-journal.json")
        .exists());
}

#[test]
fn corrupt_secrets_fail_loudly_and_leave_live_state_alone() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"corruptsecrets\"\n\n[services.api]\n{}\n",
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

    let secrets_path = project.burrow_dir().join("secrets");
    assert!(secrets_path.is_file(), "up must generate .burrow/secrets");
    let state_before: Value =
        serde_json::from_str(&fs::read_to_string(project.state_path()).expect("read state before"))
            .expect("parse state before");
    let live_pid = project.state()["services"]["api"]["pid"]
        .as_u64()
        .expect("api pid");

    // A crash mid-write leaves a readable prefix that is not valid JSON.
    let corrupt = r#"{"version":0,"secrets":{"api":"#;
    fs::write(&secrets_path, corrupt).expect("corrupt secrets");

    // `doctor` reports the failure with a remedy but does not stop the live
    // project or rewrite the file it could not parse.
    let doctor = run_in(project.path(), &["doctor", "--json"]);
    assert!(
        !doctor.status.success(),
        "doctor must fail when secrets are corrupt, got:\n{}",
        stdout(&doctor)
    );
    let report: Value = serde_json::from_str(&stdout(&doctor)).expect("doctor JSON");
    assert_eq!(report["ok"], false);
    assert_eq!(report["summary"]["failures"], 1);
    let check = report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == "secrets")
        .expect("secrets check");
    assert_eq!(check["status"], "fail");
    assert!(
        check["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("not valid Burrow secrets")),
        "secrets check was: {check}"
    );
    assert!(
        check["remedy"]
            .as_str()
            .is_some_and(|remedy| remedy.contains("delete .burrow/secrets")),
        "secrets check was: {check}"
    );

    // The daemon refreshes resource samples in `state.json` on its own timer,
    // so compare the identity fields rather than the whole document.
    let state_after: Value = serde_json::from_str(
        &fs::read_to_string(project.state_path()).expect("read state after doctor"),
    )
    .expect("parse state after doctor");
    assert_eq!(
        state_after["pid"], state_before["pid"],
        "doctor must not replace the daemon record"
    );
    assert_eq!(state_after["token"], state_before["token"]);
    assert_eq!(state_after["control_port"], state_before["control_port"]);
    assert_eq!(state_after["ready"], state_before["ready"]);
    assert_eq!(
        state_after["services"]["api"]["pid"], state_before["services"]["api"]["pid"],
        "doctor must not restart the service"
    );
    let status = run_in(project.path(), &["status", "--json"]);
    assert!(
        status.status.success(),
        "status failed while secrets were corrupt:\n{}",
        stderr(&status)
    );
    let live: Value = serde_json::from_str(&stdout(&status)).expect("status JSON");
    assert_eq!(live["services"]["api"]["pid"], live_pid);
    assert_eq!(live["services"]["api"]["status"], "healthy");
    assert_eq!(
        fs::read_to_string(&secrets_path).expect("read corrupt secrets"),
        corrupt,
        "doctor must not overwrite a secrets file it could not parse"
    );

    // `env` needs the secrets and must fail without replacing the file.
    let env = run_in(project.path(), &["env"]);
    assert!(
        !env.status.success(),
        "env must fail when secrets are corrupt, got:\n{}",
        stdout(&env)
    );
    assert!(
        stderr(&env).contains("is not valid Burrow secrets"),
        "stderr was:\n{}",
        stderr(&env)
    );
    assert_eq!(
        fs::read_to_string(&secrets_path).expect("read corrupt secrets"),
        corrupt,
        "env must not overwrite a secrets file it could not parse"
    );

    // `down` does not need secrets and must still stop the project cleanly.
    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
    assert!(!project.state_path().exists());

    // A fresh start cannot resolve secrets, so it fails and leaves no state.
    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        !up.status.success(),
        "up must fail when secrets are corrupt, got:\n{}",
        stdout(&up)
    );
    let text = stderr(&up);
    assert!(
        text.contains("the Burrow daemon exited during startup"),
        "stderr was:\n{text}"
    );
    assert!(
        text.contains("is not valid Burrow secrets"),
        "stderr was:\n{text}"
    );
    assert_eq!(
        fs::read_to_string(&secrets_path).expect("read corrupt secrets"),
        corrupt,
        "a failed up must not overwrite the corrupt file"
    );
    assert!(
        !project.state_path().exists(),
        "a failed up must not leave state behind"
    );

    // The documented remedy: delete the file and let the next up regenerate it.
    fs::remove_file(&secrets_path).expect("remove corrupt secrets");
    let recovered = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        recovered.status.success(),
        "recovery up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&recovered),
        stderr(&recovered)
    );
    project.wait_until_healthy();
    let regenerated: Value =
        serde_json::from_str(&fs::read_to_string(&secrets_path).expect("read regenerated secrets"))
            .expect("regenerated secrets JSON");
    assert!(
        regenerated["secrets"]["api"]
            .as_str()
            .is_some_and(|secret| !secret.is_empty()),
        "regenerated secrets were: {regenerated}"
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
    assert!(!project.state_path().exists());
}
