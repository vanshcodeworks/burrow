//! Windows-only CLI helpers.
//!
//! All `unsafe` blocks live here and carry a `SAFETY:` comment.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

/// Extensions tried when a command name has no extension of its own.
///
/// `PATHEXT` is consulted first so machines that disable scripting or add
/// `.PY` behave like `cmd.exe`. The usual Windows defaults are the fallback.
fn executable_extensions() -> Vec<OsString> {
    match std::env::var_os("PATHEXT") {
        Some(value) if !value.is_empty() => value
            .to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|extension| !extension.is_empty())
            .map(|extension| {
                let extension = extension.strip_prefix('.').unwrap_or(extension);
                OsString::from(format!(".{extension}").to_ascii_lowercase())
            })
            .collect(),
        _ => [".com", ".exe", ".bat", ".cmd"]
            .into_iter()
            .map(OsString::from)
            .collect(),
    }
}

fn has_extension(name: &OsStr) -> bool {
    Path::new(name).extension().is_some()
}

fn exists_in(directory: &Path, name: &OsStr, extensions: &[OsString]) -> Option<PathBuf> {
    let candidate = directory.join(name);
    if candidate.is_file() {
        return Some(candidate);
    }
    if has_extension(name) {
        return None;
    }
    for extension in extensions {
        let mut with_extension = candidate.as_os_str().to_owned();
        with_extension.push(extension);
        let path = PathBuf::from(with_extension);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Finds the executable that Windows would run for `program`.
///
/// `burrow exec` must resolve against the *service's* environment: managed
/// packages prepend their `bin` directories to `PATH`, and the service may run
/// in a subdirectory. Names with a path separator are resolved against `cwd`;
/// bare names search `cwd` and then every `search_path` entry. `.cmd`/`.bat`
/// shims are returned as-is; [`std::process::Command`] invokes those through
/// `cmd.exe`, so no shell string is built here.
pub fn resolve_executable(
    program: &OsStr,
    cwd: &Path,
    search_path: Option<&OsStr>,
) -> io::Result<PathBuf> {
    let extensions = executable_extensions();
    let program_path = Path::new(program);

    if program.to_string_lossy().contains(['\\', '/']) {
        let candidate = if program_path.is_absolute() {
            program_path.to_path_buf()
        } else {
            cwd.join(program_path)
        };
        if candidate.is_file() {
            return Ok(candidate);
        }
        if let Some(directory) = candidate.parent() {
            if let Some(name) = candidate.file_name() {
                if let Some(found) = exists_in(directory, name, &extensions) {
                    return Ok(found);
                }
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("could not find `{}`", candidate.display()),
        ));
    }

    if let Some(found) = exists_in(cwd, program, &extensions) {
        return Ok(found);
    }
    if let Some(path) = search_path {
        for directory in std::env::split_paths(path) {
            if directory.as_os_str().is_empty() {
                continue;
            }
            if let Some(found) = exists_in(&directory, program, &extensions) {
                return Ok(found);
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "could not find `{}` on PATH; check the service environment or pass an absolute path",
            program.to_string_lossy()
        ),
    ))
}

/// Opens `url` with the user's default browser, without waiting for it.
#[cfg(windows)]
pub fn open_url(url: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    // The caller only passes URLs built from validated hostnames and ports, so
    // no quoting can escape this `start` command.
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/S", "/C"])
        .raw_arg(format!("start \"\" \"{url}\""))
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the browser launcher exited with {status}"
        )))
    }
}

/// Opens `url` with the platform opener on non-Windows hosts (tests only).
#[cfg(not(windows))]
pub fn open_url(url: &str) -> io::Result<()> {
    let status = std::process::Command::new("xdg-open").arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the browser launcher exited with {status}"
        )))
    }
}

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
