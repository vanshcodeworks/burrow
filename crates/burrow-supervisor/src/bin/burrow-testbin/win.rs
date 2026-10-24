//! The only module in this binary that contains `unsafe` code.
//!
//! Both handlers are called by the console subsystem on a thread owned by
//! Windows. They only touch an atomic and return a `BOOL`, so they are safe to
//! run concurrently with the main thread.

use std::sync::atomic::{AtomicBool, Ordering};

// windows 0.62.2 does not re-export BOOL from `Win32::Foundation`;
// `PHANDLER_ROUTINE` names `windows_core::BOOL`, available as `windows::core::BOOL`.
use windows::core::BOOL;
use windows::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT, PHANDLER_ROUTINE,
};

/// Set when the process is asked to shut down via Ctrl+C or Ctrl+Break.
static GRACEFUL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Returns whether a graceful shutdown has been requested.
pub(crate) fn graceful_requested() -> bool {
    GRACEFUL_REQUESTED.load(Ordering::SeqCst)
}

/// Installs the handler used by `graceful` mode.
pub(crate) fn install_graceful_handler() -> Result<(), String> {
    install(Some(graceful_handler))
}

/// Installs the handler used by `ignore-ctrl` mode.
pub(crate) fn install_ignore_handler() -> Result<(), String> {
    install(Some(ignore_handler))
}

fn install(handler: PHANDLER_ROUTINE) -> Result<(), String> {
    // SAFETY: `handler` is a valid `extern "system"` callback that only performs
    // atomic operations and returns a Win32 BOOL. `add = true` registers it in
    // process-wide state owned by kernel32, and the function pointer lives for
    // the lifetime of the process (it is a plain function item, not a closure).
    unsafe { SetConsoleCtrlHandler(handler, true) }
        .map_err(|err| format!("failed to install console control handler: {err}"))
}

/// Records Ctrl+C / Ctrl+Break and asks the main loop to shut down.
///
/// # Safety
///
/// Called by the console subsystem on a thread owned by Windows. The body only
/// performs an atomic store, which is safe from any thread.
unsafe extern "system" fn graceful_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            GRACEFUL_REQUESTED.store(true, Ordering::SeqCst);
            true.into()
        }
        _ => false.into(),
    }
}

/// Swallows Ctrl+C / Ctrl+Break and lets the process keep running.
///
/// # Safety
///
/// Called by the console subsystem on a thread owned by Windows. Returning TRUE
/// stops the default handler from terminating the process.
unsafe extern "system" fn ignore_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => true.into(),
        _ => false.into(),
    }
}
