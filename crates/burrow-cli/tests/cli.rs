//! End-to-end CLI tests against a real daemon and real child processes.
//!
//! Every test uses its own temporary project, so ports and state never collide
//! with another test. Windows only, like the rest of Burrow.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
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
fn pkg_add_rejects_reserved_service_names_before_writing() {
    let project =
        TestProject::new("[project]\nname = \"pkgs\"\n\n[services.api]\nrun = 'cmd /C exit 0'\n");
    let cache = project.path().join("cache");
    let config_path = project.path().join("burrow.toml");
    let before = fs::read_to_string(&config_path).expect("read config before");
    let output = run_with_cache(
        project.path(),
        &cache,
        &["pkg", "add", "postgres@16", "--service", "con"],
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("reserved Windows device name"),
        "stderr was:\n{text}"
    );
    assert_eq!(
        fs::read_to_string(&config_path).expect("read config after"),
        before,
        "a rejected service name must not modify burrow.toml"
    );
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
fn env_prints_project_endpoints_and_the_selected_service_environment() {
    let project = TestProject::new(
        "[project]\nname = \"envproj\"\n\n\
         [services.api]\n\
         run = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n\
         env = { GREETING = \"hello world\" }\n\n\
         [services.web]\n\
         run = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n\
         route = \"shop.localhost\"\n",
    );

    let all = run_in(project.path(), &["env", "--format", "json"]);
    assert!(
        all.status.success(),
        "env failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&all),
        stderr(&all)
    );
    let values: serde_json::Value = serde_json::from_str(&stdout(&all)).expect("env JSON");
    assert_eq!(values["BURROW_PROJECT"], "envproj");
    assert_eq!(values["BURROW_API_HOST"], "127.0.0.1");
    assert_eq!(values["BURROW_WEB_HOST"], "127.0.0.1");
    assert!(
        values["BURROW_API_URL"]
            .as_str()
            .is_some_and(|url| url.starts_with("http://127.0.0.1:")),
        "values were: {values}"
    );
    assert!(
        values["BURROW_WEB_ROUTE_URL"]
            .as_str()
            .is_some_and(|url| url.starts_with("http://shop.localhost")),
        "values were: {values}"
    );
    assert!(
        values.get("GREETING").is_none(),
        "endpoint output leaked a service variable: {values}"
    );

    let service = run_in(project.path(), &["env", "api", "--format", "json"]);
    assert!(
        service.status.success(),
        "service env failed:\n{}",
        stderr(&service)
    );
    let values: serde_json::Value = serde_json::from_str(&stdout(&service)).expect("service JSON");
    assert_eq!(values["GREETING"], "hello world");
    assert_eq!(values["BURROW_SERVICE"], "api");
    assert_eq!(values["HOST"], "127.0.0.1");
    assert!(values["PORT"].as_str().is_some());
    assert!(
        values["BURROW_SERVICE_PASSWORD"]
            .as_str()
            .is_some_and(|password| !password.is_empty()),
        "values were: {values}"
    );

    let dotenv = run_in(project.path(), &["env", "api"]);
    assert!(dotenv.status.success(), "stderr:\n{}", stderr(&dotenv));
    let text = stdout(&dotenv);
    assert!(
        text.contains("GREETING=\"hello world\""),
        "env was:\n{text}"
    );
    assert!(text.contains("BURROW_SERVICE=api"), "env was:\n{text}");

    let powershell = run_in(project.path(), &["env", "api", "--format", "powershell"]);
    assert!(
        powershell.status.success(),
        "stderr:\n{}",
        stderr(&powershell)
    );
    assert!(
        stdout(&powershell).contains("$env:GREETING = 'hello world'"),
        "env was:\n{}",
        stdout(&powershell)
    );
}

#[test]
fn env_for_a_package_service_does_not_download_the_package() {
    let project =
        TestProject::new("[project]\nname = \"envpkg\"\n\n[services.db]\nuse = \"postgres@16\"\n");
    let cache = project.path().join("cache");
    let output = run_with_cache(project.path(), &cache, &["env", "--format", "json"]);
    assert!(
        output.status.success(),
        "env failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        !cache.join("archives").exists(),
        "burrow env downloaded a package archive"
    );
    let values: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("env JSON");
    let port = values["BURROW_DB_PORT"].as_str().expect("db port");
    assert_eq!(
        values["BURROW_DB_URL"],
        format!("postgres://postgres@127.0.0.1:{port}/postgres")
    );
}

#[test]
fn exec_uses_the_service_environment_working_directory_and_exit_code() {
    let project = TestProject::new(
        "[project]\nname = \"execproj\"\n\n\
         [services.api]\n\
         run = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n\
         cwd = \"sub\"\n\
         env = { GREETING = \"hello from the service\" }\n",
    );
    let sub = project.path().join("sub");
    fs::create_dir_all(&sub).expect("create subdirectory");
    fs::write(sub.join("hello.cmd"), "@echo off\r\necho batch-ok\r\n").expect("write batch file");

    let output = run_in(
        project.path(),
        &["exec", "api", "--", "cmd", "/D", "/C", "echo", "%GREETING%"],
    );
    assert!(
        output.status.success(),
        "exec failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(stdout(&output).trim(), "hello from the service");

    let cwd = run_in(
        project.path(),
        &["exec", "api", "--", "cmd", "/D", "/C", "cd"],
    );
    assert!(cwd.status.success(), "stderr:\n{}", stderr(&cwd));
    assert!(
        stdout(&cwd).trim().to_ascii_lowercase().ends_with("sub"),
        "working directory was:\n{}",
        stdout(&cwd)
    );

    // `.cmd` shims are found on the service working directory and executed
    // through cmd.exe without Burrow building a shell string.
    let batch = run_in(project.path(), &["exec", "api", "--", "hello.cmd"]);
    assert!(
        batch.status.success(),
        "batch exec failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&batch),
        stderr(&batch)
    );
    assert!(
        stdout(&batch).contains("batch-ok"),
        "stdout:\n{}",
        stdout(&batch)
    );

    let exit = run_in(
        project.path(),
        &["exec", "api", "--", "cmd", "/D", "/C", "exit", "7"],
    );
    assert_eq!(exit.status.code(), Some(7), "stderr:\n{}", stderr(&exit));

    let missing = run_in(
        project.path(),
        &["exec", "api", "--", "not-a-real-burrow-command"],
    );
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("could not resolve"),
        "stderr:\n{}",
        stderr(&missing)
    );

    let unknown = run_in(
        project.path(),
        &["exec", "nope", "--", "cmd", "/C", "exit", "0"],
    );
    assert!(!unknown.status.success());
    assert!(
        stderr(&unknown).contains("unknown service `nope`"),
        "stderr:\n{}",
        stderr(&unknown)
    );
}

fn http_request(port: u16, host: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut response = String::new();
    std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
    response
}

#[test]
fn proxy_routes_by_host_and_open_prints_the_route() {
    let project = TestProject::new(
        "[project]\nname = \"proxyproj\"\n\n\
         [services.web]\n\
         run = 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File server.ps1'\n\
         route = \"shop.localhost\"\n\
         health = { tcp = true, timeout = \"20s\", interval = \"100ms\" }\n",
    );
    fs::write(
        project.path().join("server.ps1"),
        r#"$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, [int]$env:PORT)
$listener.Start()
while ($true) {
  $client = $listener.AcceptTcpClient()
  try {
    $stream = $client.GetStream()
    $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::ASCII)
    $null = $reader.ReadLine()
    while (($line = $reader.ReadLine()) -ne $null -and $line -ne '') { }
    $body = "hello through proxy"
    $response = "HTTP/1.1 200 OK`r`nContent-Type: text/plain`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body"
    $bytes = [Text.Encoding]::ASCII.GetBytes($response)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
  } finally { $client.Close() }
}
"#,
    )
    .expect("write HTTP server");

    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(
        up.status.success(),
        "up failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&up),
        stderr(&up)
    );
    assert!(
        stdout(&up).contains("http://shop.localhost"),
        "up output:\n{}",
        stdout(&up)
    );

    let state = project.wait_until(Duration::from_secs(20), |state| {
        state["ready"].as_bool() == Some(true)
            && state["services"]["web"]["status"] == "healthy"
            && state["services"]["web"]["route_url"].is_string()
    });
    let proxy_port = state["proxy_port"].as_u64().expect("proxy port") as u16;
    let route_url = state["services"]["web"]["route_url"]
        .as_str()
        .expect("route URL")
        .to_owned();
    assert!(
        route_url.starts_with("http://shop.localhost"),
        "route URL: {route_url}"
    );

    let routed = http_request(proxy_port, "shop.localhost", "/check?x=1");
    assert!(routed.starts_with("HTTP/1.1 200"), "response:\n{routed}");
    assert!(
        routed.ends_with("hello through proxy"),
        "response:\n{routed}"
    );

    let unknown = http_request(proxy_port, "nope.localhost", "/");
    assert!(unknown.starts_with("HTTP/1.1 404"), "response:\n{unknown}");
    assert!(unknown.contains("Known routes"), "response:\n{unknown}");

    let open = burrow()
        .arg("--project")
        .arg(project.path())
        .arg("open")
        .env("BURROW_NO_BROWSER", "1")
        .output()
        .expect("run burrow open");
    assert!(open.status.success(), "open failed:\n{}", stderr(&open));
    assert_eq!(stdout(&open).trim(), route_url);

    let named = burrow()
        .arg("--project")
        .arg(project.path())
        .arg("open")
        .arg("web")
        .env("BURROW_NO_BROWSER", "1")
        .output()
        .expect("run burrow open web");
    assert!(
        named.status.success(),
        "open web failed:\n{}",
        stderr(&named)
    );
    assert_eq!(stdout(&named).trim(), route_url);

    let missing = burrow()
        .arg("--project")
        .arg(project.path())
        .arg("open")
        .arg("nope")
        .env("BURROW_NO_BROWSER", "1")
        .output()
        .expect("run burrow open nope");
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("is not running"),
        "stderr:\n{}",
        stderr(&missing)
    );

    let down = run_in(project.path(), &["down"]);
    assert!(down.status.success(), "down failed:\n{}", stderr(&down));
    assert!(!project.state_path().exists());
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
        started.elapsed() < Duration::from_secs(60),
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

#[test]
fn init_detects_vite_and_refuses_to_overwrite() {
    let dir = tempfile::tempdir().expect("create project directory");
    fs::write(
        dir.path().join("package.json"),
        r#"{
  "name": "shop",
  "private": true,
  "scripts": { "dev": "vite" },
  "devDependencies": { "vite": "^6.0.0" }
}"#,
    )
    .expect("write package.json");

    let output = run_in(dir.path(), &["init", "--json"]);
    assert!(output.status.success(), "init failed:\n{}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("init --json prints JSON");
    assert_eq!(report["template"], "node");
    assert_eq!(report["services"][0]["name"], "web");
    assert!(report["summary"].as_str().unwrap_or("").contains("Vite"));

    let config = fs::read_to_string(dir.path().join("burrow.toml")).expect("read burrow.toml");
    assert!(config.contains(r#"name = ""#));
    assert!(config.contains("npm run dev -- --port {{web.port}} --host 127.0.0.1"));
    assert!(config.contains(r#"route = "web.localhost""#));
    assert!(config.contains(r#"PORT = "{{web.port}}""#));

    let again = run_in(dir.path(), &["init"]);
    assert!(!again.status.success(), "a second init must fail");
    assert!(stderr(&again).contains("already exists"));
}

#[test]
fn init_template_postgres_writes_a_seed_file() {
    let dir = tempfile::tempdir().expect("create project directory");
    let output = run_in(dir.path(), &["init", "--template", "postgres", "--json"]);
    assert!(output.status.success(), "init failed:\n{}", stderr(&output));

    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("init --json prints JSON");
    assert_eq!(report["template"], "postgres");
    assert!(report["created"]
        .as_array()
        .expect("created list")
        .iter()
        .any(|entry| entry == "db/seed.sql"));

    let config = fs::read_to_string(dir.path().join("burrow.toml")).expect("read burrow.toml");
    assert!(config.contains(r#"use = "postgres@16""#));
    assert!(config.contains(r#"seed = "db/seed.sql""#));
    assert!(dir.path().join("db").join("seed.sql").is_file());
}

#[test]
fn init_without_a_detectable_stack_explains_the_templates() {
    let dir = tempfile::tempdir().expect("create project directory");
    let output = run_in(dir.path(), &["init"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("could not detect a stack"));
    assert!(stderr(&output).contains("node|python|postgres|cargo|go"));
    assert!(!dir.path().join("burrow.toml").exists());
}

#[test]
fn doctor_reports_a_valid_project_as_json() {
    let config = format!(
        "[project]\nname = \"diagnosed\"\n\n[services.api]\n{}\n",
        powershell_listener()
    );
    let project = TestProject::new(&config);

    let human = run_in(project.path(), &["doctor"]);
    assert!(
        human.status.success(),
        "doctor should pass with warnings only:\n{}",
        stderr(&human)
    );
    let text = stdout(&human);
    assert!(text.contains("Burrow doctor"));
    assert!(text.contains("config"));
    assert!(text.contains("not approved yet"));

    let json = run_in(project.path(), &["doctor", "--json"]);
    assert!(
        json.status.success(),
        "doctor --json failed:\n{}",
        stderr(&json)
    );
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&json)).expect("doctor --json prints JSON");
    assert_eq!(report["ok"], true);
    assert_eq!(report["project"], "diagnosed");
    assert!(report["summary"]["failures"].as_u64() == Some(0));
    assert!(report["checks"]
        .as_array()
        .expect("checks list")
        .iter()
        .any(|check| check["name"] == "config" && check["status"] == "ok"));
}

#[test]
fn doctor_fails_without_a_config_file() {
    let dir = tempfile::tempdir().expect("create project directory");
    let output = run_in(dir.path(), &["doctor", "--json"]);
    assert!(
        !output.status.success(),
        "doctor must fail without burrow.toml"
    );
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("doctor --json prints JSON");
    assert_eq!(report["ok"], false);
    assert!(report["checks"]
        .as_array()
        .expect("checks list")
        .iter()
        .any(|check| check["name"] == "config" && check["status"] == "fail"));
    assert!(stderr(&output).contains("doctor found"));
}

fn zip_entries(path: &Path) -> Vec<(String, String)> {
    let file = fs::File::open(path).expect("open diagnostic bundle");
    let mut archive = zip::ZipArchive::new(file).expect("read diagnostic bundle");
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("bundle entry");
        let name = entry.name().expect("entry name").into_owned();
        let mut contents = String::new();
        entry.read_to_string(&mut contents).expect("read entry");
        entries.push((name, contents));
    }
    entries
}

#[test]
fn doctor_report_writes_a_redacted_local_bundle() {
    let config = "[project]\nname = \"reported\"\n\n\
                  [services.api]\n\
                  run = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n\n\
                  [services.api.env]\n\
                  API_TOKEN = \"env-secret-value\"\n";
    let project = TestProject::new(config);
    fs::create_dir_all(project.burrow_dir()).expect("create .burrow");
    fs::write(
        project.burrow_dir().join("secrets"),
        r#"{"version":0,"secrets":{"api":"known-secret-0123456789"}}"#,
    )
    .expect("write secrets");
    fs::write(
        project.burrow_dir().join("daemon.log"),
        format!(
            "started\npassword=log-secret\npath {}\n",
            project.path().display()
        ),
    )
    .expect("write daemon log");

    let isolated = tempfile::tempdir().expect("isolated local app data");
    let crash_dir = isolated.path().join("burrow").join("crashes");
    fs::create_dir_all(&crash_dir).expect("create crash directory");
    let canonical = project.path().canonicalize().expect("canonical project");
    let note = serde_json::json!({
        "version": 1,
        "timestamp": "2026-10-26T12:00:00.000Z",
        "unix_ms": 1,
        "pid": 42,
        "thread": "main",
        "burrow_version": "0.1.0",
        "os": "windows",
        "arch": "x86_64",
        "project_dir": canonical,
        "message": "password=crash-secret",
        "location": "src/main.rs:1:1",
    });
    fs::write(
        crash_dir.join("crash-1-42.json"),
        serde_json::to_vec_pretty(&note).expect("encode crash note"),
    )
    .expect("write crash note");

    let output = burrow()
        .env("LOCALAPPDATA", isolated.path())
        .arg("--project")
        .arg(project.path())
        .args(["doctor", "--report", "--json"])
        .output()
        .expect("run doctor");
    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("doctor --json prints JSON");
    let path = PathBuf::from(report["report_path"].as_str().expect("report_path"));
    assert!(path.is_file(), "missing {}", path.display());
    assert_eq!(report["burrow_version"], "0.1.0");
    assert!(report["generated_at"]
        .as_str()
        .is_some_and(|value| value.contains('T')));

    let entries = zip_entries(&path);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    for expected in [
        "README.txt",
        "report.json",
        "environment.json",
        "config.redacted.json",
        "daemon-log.txt",
        "crashes/crash-1-42.json",
    ] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
    let project_path = project.path().display().to_string();
    for (name, contents) in &entries {
        for secret in [
            "known-secret-0123456789",
            "env-secret-value",
            "log-secret",
            "crash-secret",
        ] {
            assert!(
                !contents.contains(secret),
                "{name} leaked {secret}:\n{contents}"
            );
        }
        assert!(
            !contents.contains(project_path.as_str()),
            "{name} leaked the project path:\n{contents}"
        );
        if name.ends_with(".json") {
            serde_json::from_str::<serde_json::Value>(contents)
                .unwrap_or_else(|error| panic!("{name} is not valid JSON: {error}\n{contents}"));
        }
    }

    let human = burrow()
        .env("LOCALAPPDATA", isolated.path())
        .arg("--project")
        .arg(project.path())
        .args(["doctor", "--report"])
        .output()
        .expect("run doctor");
    assert!(human.status.success(), "{}", stderr(&human));
    assert!(stdout(&human).contains("created locally; nothing was uploaded"));
}

#[test]
fn doctor_report_dir_implies_report_and_overrides_the_default() {
    let config = format!(
        "[project]\nname = \"reported\"\n\n[services.api]\n{}\n",
        powershell_listener()
    );
    let project = TestProject::new(&config);
    let reports = tempfile::tempdir().expect("report directory");
    let output = run_in(
        project.path(),
        &[
            "doctor",
            "--report-dir",
            reports.path().to_str().expect("utf-8 report path"),
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("doctor --json prints JSON");
    let path = PathBuf::from(report["report_path"].as_str().expect("report_path"));
    assert_eq!(path.parent(), Some(reports.path()));
    assert!(path.is_file());
    assert!(
        !project.burrow_dir().join("reports").exists(),
        "--report-dir must override the default directory"
    );
}

#[test]
fn doctor_report_works_without_a_config_file() {
    let dir = tempfile::tempdir().expect("create project directory");
    let output = run_in(dir.path(), &["doctor", "--report", "--json"]);
    assert!(!output.status.success());
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("doctor --json prints JSON");
    assert_eq!(report["ok"], false);
    let path = PathBuf::from(report["report_path"].as_str().expect("report_path"));
    let entries = zip_entries(&path);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"report.json"));
    assert!(!names.contains(&"config.redacted.json"));
    let contents = entries
        .iter()
        .find(|(name, _)| name == "report.json")
        .map(|(_, contents)| contents.as_str())
        .expect("report entry");
    let bundled: serde_json::Value = serde_json::from_str(contents).expect("bundled report");
    assert_eq!(bundled["ok"], false);
    assert!(stderr(&output).contains("doctor found"));
}

#[test]
fn top_requires_an_interactive_terminal() {
    let config = format!(
        "[project]\nname = \"dashboard\"\n\n[services.api]\n{}\n",
        powershell_listener()
    );
    let project = TestProject::new(&config);
    let output = run_in(project.path(), &["top"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("interactive terminal"));
    assert!(stderr(&output).contains("burrow status"));
}

#[test]
fn snapshots_restore_data_atomically_and_reset_removes_it() {
    let project = TestProject::new(
        "[project]\nname = \"data\"\n\n[services.api]\nrun = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n\n[services.worker]\nrun = 'cmd /C ping -n 30 127.0.0.1 > NUL'\n",
    );
    let data = project.path().join(".burrow").join("data");
    fs::create_dir_all(data.join("api")).expect("create api data");
    fs::write(data.join("api").join("record.txt"), "before\n").expect("write record");

    let empty = run_in(project.path(), &["snapshots", "--json"]);
    assert!(empty.status.success(), "{}", stderr(&empty));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout(&empty)).expect("json"),
        serde_json::json!([])
    );

    let create = run_in(project.path(), &["snapshot", "before", "--json"]);
    assert!(create.status.success(), "{}", stderr(&create));
    let manifest: serde_json::Value =
        serde_json::from_str(&stdout(&create)).expect("snapshot json");
    assert_eq!(manifest["name"], "before");
    assert_eq!(manifest["services"][0]["present"], true);
    assert_eq!(manifest["services"][1]["present"], false);

    fs::write(data.join("api").join("record.txt"), "after\n").expect("mutate");
    fs::create_dir_all(data.join("worker")).expect("create worker data");

    let restore = run_in(project.path(), &["restore", "before", "--json"]);
    assert!(restore.status.success(), "{}", stderr(&restore));
    assert_eq!(
        fs::read_to_string(data.join("api").join("record.txt")).expect("read"),
        "before\n"
    );
    assert!(
        !data.join("worker").exists(),
        "a directory absent at snapshot time must be removed"
    );

    let list = run_in(project.path(), &["snapshots", "--json"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let listed: serde_json::Value = serde_json::from_str(&stdout(&list)).expect("json");
    assert_eq!(listed[0]["name"], "before");

    let duplicate = run_in(project.path(), &["snapshot", "before"]);
    assert!(!duplicate.status.success());
    assert!(stderr(&duplicate).contains("already exists"));

    let invalid = run_in(project.path(), &["snapshot", "../escape"]);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("invalid snapshot name"));

    let reset = run_in(project.path(), &["reset", "api", "--json"]);
    assert!(reset.status.success(), "{}", stderr(&reset));
    assert!(!data.join("api").exists());

    let unknown = run_in(project.path(), &["reset", "missing"]);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("unknown service"));
}

#[test]
fn data_commands_refuse_to_run_while_the_daemon_is_up() {
    let config = format!(
        "[project]\nname = \"busy\"\n\n[services.api]\n{}\n",
        powershell_listener()
    );
    let project = TestProject::new(&config);
    let up = run_in(project.path(), &["up", "--detach", "--yes"]);
    assert!(up.status.success(), "up failed:\n{}", stderr(&up));
    project.wait_until(Duration::from_secs(15), |state| {
        state["ready"].as_bool() == Some(true)
    });

    let output = run_in(project.path(), &["snapshot", "busy"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("services are running"));
    assert!(stderr(&output).contains("burrow down"));
}
