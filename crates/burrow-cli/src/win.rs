//! Windows-only CLI helpers.
//!
//! All `unsafe` blocks live here and carry a `SAFETY:` comment.

/// Stops future child processes from inheriting Burrow's standard handles.
///
/// `std::process::Command` on Windows sets `bInheritHandles`, so a detached
/// daemon would otherwise keep any pipe that captured `burrow up`'s output
/// open. Shell pipelines, CI logs captured by a parent, and integration tests
/// would then wait for the daemon to exit before seeing `burrow up` finish.
///
/// The daemon's own standard handles are redirected to `.burrow/daemon.log`
/// before creation, so inheriting the caller's pipe is never useful.
pub fn make_standard_handles_non_inheritable() {
    use windows::Win32::Foundation::{
        SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT,
    };
    use windows::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    for standard in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `GetStdHandle` reads the process standard-handle table. It
        // takes a constant selector, has no pointer arguments, and cannot
        // cause memory unsafety.
        let Ok(handle) = (unsafe { GetStdHandle(standard) }) else {
            continue;
        };
        if handle.is_invalid() || handle == HANDLE::default() {
            continue;
        }
        // SAFETY: `handle` came from `GetStdHandle` and is only read by
        // `SetHandleInformation`. Clearing `HANDLE_FLAG_INHERIT` makes future
        // `CreateProcess` calls skip this handle while leaving it usable by
        // the current process.
        let _ = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) };
    }
}
