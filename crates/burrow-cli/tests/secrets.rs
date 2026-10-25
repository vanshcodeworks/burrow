//! Adversarial test: generated secrets must never leak into Burrow's own
//! outputs. The one documented exception is `burrow env <service>`, which
//! exists to export a service environment.

use std::fs;
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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
    }
}

fn powershell_listener() -> &'static str {
    r#"run = 'powershell -NoProfile -NonInteractive -Command "$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,[int]$env:PORT);$l.Start();Write-Output ready;while($true){Start-Sleep -Seconds 1}"'"#
}

/// Reads the generated secret for every service from `.burrow/secrets`.
fn read_secrets(project: &TestProject) -> Vec<String> {
    let contents =
        fs::read_to_string(project.burrow_dir().join("secrets")).expect("read secrets file");
    let document: serde_json::Value = serde_json::from_str(&contents).expect("parse secrets");
    let secrets = document["secrets"]
        .as_object()
        .expect("secrets is an object keyed by service");
    let values: Vec<String> = secrets
        .values()
        .map(|value| value.as_str().expect("secret is a string").to_owned())
        .collect();
    assert!(
        values.len() >= 2 && values.iter().all(|secret| !secret.is_empty()),
        "expected one non-empty secret per service: {document}"
    );
    values
}

/// Fails with the file name when a secret appears in scanned content.
fn assert_absent(secrets: &[String], content: &str, where_: &str) {
    for secret in secrets {
        assert!(
            !content.contains(secret.as_str()),
            "secret leaked into {where_}"
        );
    }
}

#[test]
fn generated_secrets_never_appear_in_logs_state_or_status() {
    let project = TestProject::new(&format!(
        "[project]\nname = \"leaks\"\n\n[services.api]\n{}\n\
         health = {{ tcp = true, timeout = \"20s\", interval = \"100ms\" }}\n\n\
         [services.worker]\n\
         run = 'cmd /C ping -n 600 127.0.0.1 > NUL'\n",
        powershell_listener()
    ));

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    project.wait_until(Duration::from_secs(30), |state| {
        state["ready"].as_bool() == Some(true)
            && state["services"]["api"]["status"] == "healthy"
            && state["services"]["worker"]["status"] == "healthy"
    });

    let secrets = read_secrets(&project);

    // The service must have produced at least one log line before the scan,
    // otherwise (b) would pass vacuously.
    let logs_dir = project.burrow_dir().join("logs");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let combined = fs::read_dir(&logs_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| fs::read_to_string(entry.path()).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if combined.contains("ready") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "service output was never captured; the scan would be vacuous"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    // (a) + (b) every log file, including the daemon log.
    for entry in fs::read_dir(project.burrow_dir())
        .expect("read .burrow")
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("log") {
            let contents = fs::read_to_string(&path).unwrap_or_default();
            assert_absent(&secrets, &contents, &path.display().to_string());
        }
    }
    for entry in fs::read_dir(&logs_dir)
        .expect("read logs")
        .filter_map(Result::ok)
    {
        let contents = fs::read_to_string(entry.path()).unwrap_or_default();
        assert_absent(&secrets, &contents, &entry.path().display().to_string());
    }

    // (c) runtime state never carries secrets.
    let state = fs::read_to_string(project.state_path()).expect("read state");
    assert_absent(&secrets, &state, "state.json");

    // (d) + (e) + (f) command output.
    let status = run_in(project.path(), &["status"]);
    assert!(
        status.status.success(),
        "status failed: {}",
        stderr(&status)
    );
    assert_absent(&secrets, &stdout(&status), "burrow status stdout");
    assert_absent(&secrets, &stderr(&status), "burrow status stderr");

    let status_json = run_in(project.path(), &["--json", "status"]);
    assert!(
        status_json.status.success(),
        "status --json failed: {}",
        stderr(&status_json)
    );
    assert_absent(&secrets, &stdout(&status_json), "burrow status --json");

    let logs = run_in(project.path(), &["logs"]);
    assert!(logs.status.success(), "logs failed: {}", stderr(&logs));
    assert_absent(&secrets, &stdout(&logs), "burrow logs stdout");
    assert_absent(&secrets, &stderr(&logs), "burrow logs stderr");

    let env = run_in(project.path(), &["env"]);
    assert!(env.status.success(), "env failed: {}", stderr(&env));
    assert_absent(&secrets, &stdout(&env), "burrow env (project level)");
    assert_absent(&secrets, &stderr(&env), "burrow env (project level)");

    // Positive control: `burrow env <service>` is the documented exception.
    let service_env = run_in(project.path(), &["env", "api"]);
    assert!(
        service_env.status.success(),
        "env api failed: {}",
        stderr(&service_env)
    );
    assert!(
        secrets
            .iter()
            .any(|secret| stdout(&service_env).contains(secret.as_str())),
        "burrow env api must print the service secret (positive control)"
    );

    // Snapshots cover data directories, not .burrow/secrets; verify.
    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
    let snapshot = run_in(project.path(), &["snapshot", "before"]);
    assert!(
        snapshot.status.success(),
        "snapshot failed: {}",
        stderr(&snapshot)
    );
    let snapshots_dir = project.burrow_dir().join("snapshots").join("before");
    let snapshot_content = walk_files(&snapshots_dir)
        .map(|path| fs::read_to_string(&path).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert_absent(&secrets, &snapshot_content, "the before snapshot");
}

/// Every file under `root`, recursively.
fn walk_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk_files(&path));
            } else {
                files.push(path);
            }
        }
    }
    files.into_iter()
}
