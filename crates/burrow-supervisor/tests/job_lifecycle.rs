//! Windows Job Object lifecycle tests.
//!
//! Run with `cargo test --workspace --all-targets -- --test-threads=1`; these
//! tests exercise console control events and process-wide state.

#![cfg(windows)]

mod support;

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use burrow_supervisor::{ManagedProcess, ProcessSpec, ShutdownOutcome};
use support::{is_in_some_job, process_alive, spawn_helper, testbin, wait_until};

fn read_pids(path: &Path) -> Vec<u32> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

async fn spawn_tree(name: &str, temp: &tempfile::TempDir) -> (ManagedProcess, Vec<u32>) {
    let pidfile = temp.path().join("tree.pids");
    let pidfile_arg = pidfile.to_string_lossy().into_owned();
    let process = spawn_helper(name, &["tree", pidfile_arg.as_str()]).await;

    assert!(
        wait_until(|| pidfile.is_file(), Duration::from_secs(10)).await,
        "tree helper did not write {}",
        pidfile.display()
    );

    let pids = read_pids(&pidfile);
    assert_eq!(pids.len(), 2, "expected parent and child pid, got {pids:?}");
    assert!(
        wait_until(
            || pids.iter().all(|pid| process_alive(*pid)),
            Duration::from_secs(5)
        )
        .await,
        "helper process tree should be running: {pids:?}"
    );
    (process, pids)
}

#[tokio::test]
async fn reports_the_exit_code() {
    let mut process = spawn_helper("exit", &["exit", "7"]).await;

    let status = tokio::time::timeout(Duration::from_secs(10), process.wait())
        .await
        .expect("helper should exit promptly")
        .expect("wait should succeed");

    assert_eq!(status.code(), Some(7));

    let outcome = process
        .shutdown(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown after exit should succeed");
    assert!(
        matches!(outcome, ShutdownOutcome::AlreadyExited { .. }),
        "unexpected outcome: {outcome:?}"
    );
}

#[tokio::test]
async fn dropping_a_process_kills_the_whole_tree() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (process, pids) = spawn_tree("tree-drop", &temp).await;

    assert!(
        pids.iter().all(|pid| is_in_some_job(*pid)),
        "both processes must be inside the job at creation time: {pids:?}"
    );

    // Closing the last job handle must terminate parent and grandchild.
    drop(process);

    assert!(
        wait_until(
            || pids.iter().all(|pid| !process_alive(*pid)),
            Duration::from_secs(10)
        )
        .await,
        "processes survived their job handle closing (orphans): {pids:?}"
    );
}

#[tokio::test]
async fn enforces_a_job_memory_cap() {
    let spec = ProcessSpec::new("leak", testbin())
        .arg("leak")
        .arg("268435456")
        .memory_limit_bytes(64 * 1024 * 1024);

    let mut process = ManagedProcess::spawn(spec)
        .await
        .expect("leak helper should start");

    let status = tokio::time::timeout(Duration::from_secs(30), process.wait())
        .await
        .expect("the memory cap should terminate the leaky helper")
        .expect("wait should succeed");

    assert!(
        !status.success(),
        "leaky helper exited successfully; the memory cap was not enforced: {status:?}"
    );

    process
        .shutdown(Some(Duration::from_secs(1)))
        .await
        .expect("cleanup should succeed");
}

#[tokio::test]
async fn shutdown_forces_an_unresponsive_process_after_the_grace_period() {
    let temp = tempfile::tempdir().expect("temp dir");
    let ready = temp.path().join("ignore.ready");
    let ready_arg = ready.to_string_lossy().into_owned();

    let mut process = spawn_helper("ignore-ctrl", &["ignore-ctrl", ready_arg.as_str()]).await;
    assert!(
        wait_until(|| ready.is_file(), Duration::from_secs(10)).await,
        "helper did not report ready"
    );

    let started = Instant::now();
    let outcome = process
        .shutdown(Some(Duration::from_millis(300)))
        .await
        .expect("shutdown should force termination");

    assert!(
        matches!(outcome, ShutdownOutcome::Forced { .. }),
        "expected forced termination, got {outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "forced shutdown took too long: {:?}",
        started.elapsed()
    );
    assert!(!process_alive(process.pid()));
}

#[tokio::test]
async fn ctrl_break_allows_a_graceful_exit_when_a_console_is_available() {
    let temp = tempfile::tempdir().expect("temp dir");
    let flag = temp.path().join("graceful.flag");
    let ready = temp.path().join("graceful.flag.ready");
    let flag_arg = flag.to_string_lossy().into_owned();

    let mut process = spawn_helper("graceful", &["graceful", flag_arg.as_str()]).await;
    assert!(
        wait_until(|| ready.is_file(), Duration::from_secs(10)).await,
        "graceful helper did not report ready"
    );

    let outcome = process
        .shutdown(Some(Duration::from_secs(10)))
        .await
        .expect("shutdown should succeed");

    match outcome {
        ShutdownOutcome::Graceful { status, .. } => {
            assert!(status.success(), "graceful exit should be successful");
            assert!(
                flag.is_file(),
                "the control handler did not run (flag file missing)"
            );
        }
        ShutdownOutcome::Forced {
            graceful_signal_sent,
            ..
        } => {
            assert!(
                !graceful_signal_sent,
                "CTRL_BREAK was delivered but the helper did not exit gracefully"
            );
            eprintln!(
                "note: no console is available to this test process; \
                 CTRL_BREAK delivery is covered by manual/desktop CI runs"
            );
        }
        other => panic!("unexpected shutdown outcome: {other:?}"),
    }
}

#[tokio::test]
async fn shutdown_terminates_descendants_and_empties_the_job() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (mut process, pids) = spawn_tree("tree-shutdown", &temp).await;

    let outcome = process
        .shutdown(Some(Duration::from_secs(2)))
        .await
        .expect("shutdown should succeed");

    assert!(
        matches!(
            outcome,
            ShutdownOutcome::Graceful { .. }
                | ShutdownOutcome::Forced { .. }
                | ShutdownOutcome::AlreadyExited { .. }
        ),
        "unexpected outcome: {outcome:?}"
    );

    assert!(
        wait_until(
            || pids.iter().all(|pid| !process_alive(*pid)),
            Duration::from_secs(10)
        )
        .await,
        "shutdown left descendant processes alive: {pids:?}"
    );
}

#[tokio::test]
async fn job_close_on_host_death_leaves_no_orphans() {
    let temp = tempfile::tempdir().expect("temp dir");
    let tree_pidfile = temp.path().join("host-tree.pids");
    let tree_pidfile_arg = tree_pidfile.to_string_lossy().into_owned();

    // The host is a *separate supervisor process*: when it dies abruptly, only
    // the kernel closing its job handle can save its children from orphaning.
    let mut host = std::process::Command::new(testbin())
        .arg("host")
        .arg(testbin())
        .arg("tree")
        .arg(tree_pidfile_arg.as_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("could not start the supervisor host");

    assert!(
        wait_until(|| tree_pidfile.is_file(), Duration::from_secs(15)).await,
        "host did not spawn the tree helper"
    );
    let pids = read_pids(&tree_pidfile);
    assert_eq!(pids.len(), 2, "expected parent and child pid");
    assert!(
        wait_until(
            || pids.iter().all(|pid| process_alive(*pid)),
            Duration::from_secs(5)
        )
        .await,
        "tree helper should be running before the host is killed: {pids:?}"
    );

    host.kill()
        .expect("could not terminate the supervisor host");
    let _ = host.wait();

    assert!(
        wait_until(
            || pids.iter().all(|pid| !process_alive(*pid)),
            Duration::from_secs(10)
        )
        .await,
        "the job did not kill the process tree when the host died: {pids:?}"
    );
}
