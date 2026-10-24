//! Shared helpers for supervisor integration tests.
//!
//! The Win32 checks live in this module (not in the product crate) because the
//! tests must observe process liveness and job membership from the outside.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use burrow_supervisor::{ManagedProcess, ProcessSpec};
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Absolute path to the test-helper binary built by Cargo for these tests.
pub fn testbin() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_burrow-testbin"));
    assert!(
        path.is_file(),
        "test helper not found at {}; Cargo should build it for integration tests",
        path.display()
    );
    path
}

/// Spawns `burrow-testbin` with the given arguments as a managed process.
pub async fn spawn_helper(name: &str, arguments: &[&str]) -> ManagedProcess {
    let spec = ProcessSpec::new(name, testbin()).args(arguments.iter().copied());
    ManagedProcess::spawn(spec)
        .await
        .unwrap_or_else(|error| panic!("failed to spawn test helper {name}: {error}"))
}

/// Polls `condition` every 25 ms until it is true or `timeout` elapses.
pub async fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let started = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Returns whether `pid` refers to a live process.
///
/// The helper processes used by these tests never exit with code 259, so the
/// `STILL_ACTIVE` sentinel is unambiguous here.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: OpenProcess takes plain values and returns an owned handle. The
    // handle is closed exactly once below. `GetExitCodeProcess` writes to a
    // valid stack location.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let queried = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        queried && code == STILL_ACTIVE.0 as u32
    }
}

/// Returns whether `pid` is assigned to any job object.
pub fn is_in_some_job(pid: u32) -> bool {
    // SAFETY: OpenProcess returns an owned handle closed exactly once below.
    // `IsProcessInJob` writes a BOOL to a valid stack location; a null job
    // handle means "any job", which is the documented query we want.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut in_job = windows::core::BOOL::default();
        let queried = IsProcessInJob(handle, None, &mut in_job).is_ok();
        let _ = CloseHandle(handle);
        queried && in_job.as_bool()
    }
}
