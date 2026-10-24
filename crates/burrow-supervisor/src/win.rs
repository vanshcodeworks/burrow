//! All Win32 interaction for the supervisor.
//!
//! # Safety
//!
//! This is the only module in the workspace that contains `unsafe` code. Every
//! block carries a `SAFETY:` comment explaining the invariants it relies on.
//! The goal is to keep the surface small and auditable: safe wrappers around
//! job objects, process creation, pipes, and console control events.

use std::collections::BTreeMap;
use std::ffi::{c_void, OsStr, OsString};
use std::fs::File;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ,
    HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WIN32_ERROR,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
use windows::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectCpuRateControlInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_LIMIT_INFORMATION,
    JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
    JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};

use crate::error::{Error, Result};

/// A Windows Job Object owned by one service.
///
/// The handle is created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so dropping
/// the last handle to the job terminates every process still assigned to it.
pub(crate) struct Job {
    handle: HANDLE,
}

// SAFETY: a job handle is a kernel object handle that is valid process-wide.
// Ownership is exclusive to this `Job`; it is only transferred/closed on drop,
// and `CloseHandle` is safe to call from any thread. Shared access only reads
// the handle value or passes it to thread-safe Win32 APIs.
unsafe impl Send for Job {}
// SAFETY: see the `Send` impl above. Concurrent `active_processes` /
// `terminate` calls on the same job are serialized by Win32 itself.
unsafe impl Sync for Job {}

impl Job {
    /// Creates a job with the requested limits.
    pub(crate) fn create(memory_limit: Option<u64>, cpu_percent: Option<u8>) -> Result<Self> {
        // SAFETY: null attributes request a non-inheritable handle. The returned
        // handle is checked by the wrapper and owned by `Job` from here on.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|source| windows_error("CreateJobObjectW", source))?;
        let job = Self { handle };
        job.apply_limits(memory_limit, cpu_percent)?;
        Ok(job)
    }

    fn apply_limits(&self, memory_limit: Option<u64>, cpu_percent: Option<u8>) -> Result<()> {
        let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let mut job_memory_limit = 0usize;
        let mut process_memory_limit = 0usize;

        if let Some(bytes) = memory_limit {
            if bytes == 0 {
                return Err(Error::InvalidSpec("memory limit must be greater than zero"));
            }
            let bytes = usize::try_from(bytes)
                .map_err(|_| Error::InvalidSpec("memory limit does not fit in usize"))?;
            flags |= JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            job_memory_limit = bytes;
            process_memory_limit = bytes;
        }

        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: flags,
                ..Default::default()
            },
            JobMemoryLimit: job_memory_limit,
            ProcessMemoryLimit: process_memory_limit,
            ..Default::default()
        };

        // SAFETY: `limits` is a fully initialized structure of the exact type
        // required by `JobObjectExtendedLimitInformation`, and the length
        // matches its size. The job handle is valid for the lifetime of `self`.
        unsafe {
            SetInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|source| windows_error("SetInformationJobObject(extended limits)", source))?;

        if let Some(percent) = cpu_percent {
            if !(1..=100).contains(&percent) {
                return Err(Error::InvalidSpec(
                    "CPU limit must be a whole number between 1 and 100 percent",
                ));
            }
            let cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE
                    | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    CpuRate: u32::from(percent) * 100,
                },
            };
            // SAFETY: `cpu` is fully initialized for the CPU rate control
            // information class and outlives the call. The union was initialized
            // with the `CpuRate` view selected by `ENABLE | HARD_CAP`.
            unsafe {
                SetInformationJobObject(
                    self.handle,
                    JobObjectCpuRateControlInformation,
                    ptr::from_ref(&cpu).cast(),
                    size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                )
            }
            .map_err(|source| windows_error("SetInformationJobObject(CPU rate)", source))?;
        }

        Ok(())
    }

    /// Terminates every process in the job.
    pub(crate) fn terminate(&self, exit_code: u32) -> Result<()> {
        // SAFETY: the job handle is valid and owned by `self`. Terminating a job
        // that is already empty is a successful no-op.
        unsafe { TerminateJobObject(self.handle, exit_code) }
            .map_err(|source| windows_error("TerminateJobObject", source))
    }

    /// The raw job handle, used when creating a process directly inside the job.
    pub(crate) fn handle(&self) -> HANDLE {
        self.handle
    }

    /// Returns the number of processes currently assigned to the job.
    pub(crate) fn active_processes(&self) -> Result<u32> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: `accounting` is a correctly sized, writable structure for the
        // requested information class and outlives the call.
        unsafe {
            QueryInformationJobObject(
                Some(self.handle),
                JobObjectBasicAccountingInformation,
                ptr::from_mut(&mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(|source| windows_error("QueryInformationJobObject", source))?;
        Ok(accounting.ActiveProcesses)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: `self.handle` was created by `CreateJobObjectW`, has not been
        // duplicated into another owner, and is closed exactly once here.
        // `KILL_ON_JOB_CLOSE` makes this the orphan-prevention guarantee.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// A process handle owned by the supervisor.
///
/// The dedicated wait thread and the `ManagedProcess` share this through an
/// `Arc`, so the handle stays valid until both are done.
pub(crate) struct ProcessHandle {
    handle: HANDLE,
    pid: u32,
}

// SAFETY: a process handle is a kernel object handle valid process-wide. It is
// only closed once, in `Drop`, after all `Arc` owners (including the wait
// thread) have finished. `WaitForSingleObject`/`GetExitCodeProcess` are
// thread-safe.
unsafe impl Send for ProcessHandle {}
// SAFETY: see the `Send` impl above.
unsafe impl Sync for ProcessHandle {}

impl ProcessHandle {
    /// The Windows process id.
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// Blocks until the process exits and returns its exit code.
    pub(crate) fn wait(&self) -> Result<u32> {
        // SAFETY: the handle refers to a process and is valid for `self`.
        let wait = unsafe { WaitForSingleObject(self.handle, INFINITE) };
        if wait != WAIT_OBJECT_0 {
            return Err(windows_error("WaitForSingleObject", last_error()));
        }
        self.exit_code()
    }

    fn exit_code(&self) -> Result<u32> {
        let mut code = 0u32;
        // SAFETY: the process handle is valid and signaled, and `code` is a
        // valid out-pointer.
        unsafe { GetExitCodeProcess(self.handle, &mut code) }
            .map_err(|source| windows_error("GetExitCodeProcess", source))?;
        Ok(code)
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by `CreateProcessW`, is owned by this
        // struct, and is closed exactly once.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Everything needed to create one child process.
pub(crate) struct SpawnRequest {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) env: BTreeMap<OsString, OsString>,
}

/// A spawned process and the read ends of its stdout/stderr pipes.
pub(crate) struct SpawnedProcess {
    pub(crate) process: ProcessHandle,
    pub(crate) stdout: File,
    pub(crate) stderr: File,
}

/// Creates `request` inside `job` atomically, so no descendant can escape the
/// job by racing a post-spawn assignment.
pub(crate) fn spawn_in_job(job: &Job, request: &SpawnRequest) -> Result<SpawnedProcess> {
    let resolved_program = resolve_program(
        request.program.as_os_str(),
        request.cwd.as_deref(),
        &request.env,
    );
    let program_wide = wide_null(resolved_program.as_os_str());
    if has_interior_nul(&program_wide) {
        return Err(Error::InvalidSpec("program path contains a NUL character"));
    }

    let mut command_line = build_command_line(resolved_program.as_os_str(), &request.args);
    if has_interior_nul(&command_line) {
        return Err(Error::InvalidSpec("arguments contain a NUL character"));
    }

    for key in request.env.keys() {
        if key.is_empty()
            || key.to_string_lossy().contains('=')
            || key.to_string_lossy().contains('\0')
            || key.as_os_str().encode_wide().any(|unit| unit == 0)
        {
            return Err(Error::InvalidSpec(
                "environment variable names must be non-empty and contain no '=' or NUL",
            ));
        }
    }
    for value in request.env.values() {
        if value.encode_wide().any(|unit| unit == 0) {
            return Err(Error::InvalidSpec(
                "environment variable values must not contain NUL",
            ));
        }
    }

    let environment = build_environment(&request.env);
    let cwd_wide = request
        .cwd
        .as_deref()
        .map(|path| wide_null(path.as_os_str()));
    if cwd_wide.as_ref().is_some_and(|wide| has_interior_nul(wide)) {
        return Err(Error::InvalidSpec(
            "working directory contains a NUL character",
        ));
    }

    // Inheritable write ends for stdout/stderr, non-inheritable read ends.
    let (stdout_read, stdout_write) = inheritable_pipe()?;
    let (stderr_read, stderr_write) = inheritable_pipe()?;
    let stdin = open_nul()?;

    let mut attributes = ProcThreadAttributes::new(2)?;
    let job_value = [job.handle()];
    attributes.set(&job_value, PROC_THREAD_ATTRIBUTE_JOB_LIST)?;
    let inherited = [stdin.raw(), stdout_write.raw(), stderr_write.raw()];
    attributes.set(&inherited, PROC_THREAD_ATTRIBUTE_HANDLE_LIST)?;

    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: size_of::<STARTUPINFOEXW>() as u32,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: stdin.raw(),
            hStdOutput: stdout_write.raw(),
            hStdError: stderr_write.raw(),
            ..Default::default()
        },
        lpAttributeList: attributes.list(),
    };

    let mut process_info = PROCESS_INFORMATION::default();
    let creation_flags =
        EXTENDED_STARTUPINFO_PRESENT | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT;
    let environment_ptr = environment
        .as_ref()
        .map(|block| block.as_ptr().cast::<c_void>());
    let cwd_ptr = cwd_wide
        .as_ref()
        .map_or(PCWSTR::null(), |wide| PCWSTR(wide.as_ptr()));

    // SAFETY: all pointers below reference initialized buffers that outlive this
    // call. `command_line` is mutable and NUL-terminated as required because
    // CreateProcessW may write to it. `startup` is a STARTUPINFOEXW while the
    // EXTENDED_STARTUPINFO_PRESENT flag is set, and its handle list contains only
    // handles marked inheritable. The job is attached atomically via
    // PROC_THREAD_ATTRIBUTE_JOB_LIST, so there is no post-spawn assignment race.
    let created = unsafe {
        CreateProcessW(
            PCWSTR(program_wide.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            creation_flags,
            environment_ptr,
            cwd_ptr,
            ptr::from_ref(&startup).cast::<STARTUPINFOW>(),
            &mut process_info,
        )
    };
    if let Err(source) = created {
        return Err(Error::Spawn {
            program: request.program.to_string_lossy().into_owned(),
            source,
        });
    }

    // SAFETY: `process_info.hThread` is an owned handle returned by
    // CreateProcessW. We never need it (the process is not created suspended),
    // so it is closed exactly once here.
    unsafe {
        let _ = CloseHandle(process_info.hThread);
    }

    let process = ProcessHandle {
        handle: process_info.hProcess,
        pid: process_info.dwProcessId,
    };

    // The child owns inherited copies of the write ends; closing ours lets the
    // read ends observe EOF when the child (and all descendants) exit.
    drop(stdout_write);
    drop(stderr_write);
    drop(stdin);

    Ok(SpawnedProcess {
        process,
        stdout: stdout_read.into_file(),
        stderr: stderr_read.into_file(),
    })
}

/// Delivers `CTRL_BREAK` to the process group whose id is `process_group_id`.
///
/// Children are created with `CREATE_NEW_PROCESS_GROUP`, so the group id equals
/// the child's pid. This requires the child to share a console with Burrow; if
/// not (detached/GUI parent), the call fails and the caller must force
/// termination instead.
pub(crate) fn send_ctrl_break(
    process_group_id: u32,
) -> std::result::Result<(), windows::core::Error> {
    // SAFETY: GenerateConsoleCtrlEvent has no pointer arguments. Targeting a
    // specific group id does not affect the caller's own group.
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, process_group_id) }
}

/// Owns a Win32 handle and closes it exactly once.
struct OwnedHandle(Option<HANDLE>);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self(Some(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0.expect("owned handle is present until consumed")
    }

    fn into_file(mut self) -> File {
        let handle = self
            .0
            .take()
            .expect("owned handle is present until consumed");
        // SAFETY: the raw handle is exclusively owned by `self` and is not closed
        // anywhere else, which is exactly the contract of `File::from_raw_handle`.
        unsafe { File::from_raw_handle(handle.0) }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            // SAFETY: this handle was returned by CreatePipe/CreateFileW and is
            // owned exclusively by this struct.
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

fn inheritable_pipe() -> Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    // SAFETY: the out-pointers are valid, and `attributes` outlives the call.
    unsafe { CreatePipe(&mut read, &mut write, Some(&attributes), 0) }
        .map_err(|source| windows_error("CreatePipe", source))?;

    let read = OwnedHandle::new(read);
    let write = OwnedHandle::new(write);

    // SAFETY: the read end is owned by us and must not leak into the child.
    unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
        .map_err(|source| windows_error("SetHandleInformation", source))?;

    Ok((read, write))
}

fn open_nul() -> Result<OwnedHandle> {
    let path = wide_null(OsStr::new("NUL"));
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    // SAFETY: `path` is NUL-terminated and lives for the call; `attributes` is a
    // valid security descriptor structure. The handle is inheritable so the
    // child can use it as stdin and is owned by the returned wrapper.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            Some(&attributes),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|source| windows_error("CreateFileW(NUL)", source))?;
    Ok(OwnedHandle::new(handle))
}

/// RAII wrapper for the opaque `PROC_THREAD_ATTRIBUTE_LIST` buffer.
struct ProcThreadAttributes {
    // `Vec<usize>` provides the pointer alignment the API requires.
    _buffer: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcThreadAttributes {
    fn new(count: u32) -> Result<Self> {
        let mut size = 0usize;
        // SAFETY: a sizing query with a null list is the documented first step.
        // It returns ERROR_INSUFFICIENT_BUFFER and fills `size`.
        let sized = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut size) };
        if let Err(error) = sized {
            if WIN32_ERROR::from_error(&error) != Some(ERROR_INSUFFICIENT_BUFFER) {
                return Err(windows_error(
                    "InitializeProcThreadAttributeList(size)",
                    error,
                ));
            }
        }
        if size == 0 {
            return Err(windows_error(
                "InitializeProcThreadAttributeList(size)",
                last_error(),
            ));
        }

        let words = size.div_ceil(size_of::<usize>());
        let mut buffer = vec![0usize; words];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());

        // SAFETY: `buffer` is pointer-aligned and at least `size` bytes, `list`
        // points into it, and `size` is the value reported by the sizing call.
        unsafe { InitializeProcThreadAttributeList(Some(list), count, None, &mut size) }
            .map_err(|source| windows_error("InitializeProcThreadAttributeList", source))?;

        Ok(Self {
            _buffer: buffer,
            list,
        })
    }

    fn list(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }

    fn set<T>(&mut self, value: &[T], attribute: u32) -> Result<()> {
        // SAFETY: `self.list` was initialized in `new`, `value` is a live slice
        // of the exact size passed, and the attribute constant comes from the
        // Windows SDK. The value is copied into the attribute list by Win32.
        unsafe {
            UpdateProcThreadAttribute(
                self.list,
                0,
                attribute as usize,
                Some(value.as_ptr().cast::<c_void>()),
                size_of_val(value),
                None,
                None,
            )
        }
        .map_err(|source| windows_error("UpdateProcThreadAttribute", source))
    }
}

impl Drop for ProcThreadAttributes {
    fn drop(&mut self) {
        // SAFETY: the list was successfully initialized in `new` and is
        // destroyed exactly once, before its backing buffer is freed.
        unsafe {
            DeleteProcThreadAttributeList(self.list);
        }
    }
}

fn windows_error(operation: &'static str, source: windows::core::Error) -> Error {
    Error::Windows { operation, source }
}

/// Reads the calling thread's last-error value.
fn last_error() -> windows::core::Error {
    // SAFETY: GetLastError has no preconditions and always succeeds.
    unsafe { GetLastError() }.into()
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = value.encode_wide().collect();
    wide.push(0);
    wide
}

fn has_interior_nul(wide: &[u16]) -> bool {
    wide.iter()
        .take(wide.len().saturating_sub(1))
        .any(|unit| *unit == 0)
}

/// Resolves a program name to an executable path.
///
/// `CreateProcessW` does not search `PATH` when `lpApplicationName` is set, so
/// bare names such as `node` or `cmd` are resolved here. Exact file names and
/// `<name>.exe` are considered; `.cmd`/`.bat` shims need a shell and are not
/// launched directly.
///
/// A named program with a directory component is resolved against the child's
/// working directory when one is configured, mirroring how a user reads a
/// relative command in a project file. Otherwise it is left for Windows to
/// resolve against Burrow's current directory.
fn resolve_program(
    program: &OsStr,
    cwd: Option<&Path>,
    env: &BTreeMap<OsString, OsString>,
) -> PathBuf {
    let program_path = Path::new(program);
    let has_directory = program.to_string_lossy().contains(['\\', '/']);

    if has_directory {
        return match cwd {
            Some(directory) if program_path.is_relative() => directory.join(program_path),
            _ => program_path.to_path_buf(),
        };
    }

    let path_value = env
        .iter()
        .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os("PATH"));
    let Some(path_value) = path_value else {
        return program_path.to_path_buf();
    };

    let has_extension = program_path.extension().is_some();
    for directory in std::env::split_paths(&path_value) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let candidate = directory.join(program_path);
        if candidate.is_file() {
            return candidate;
        }
        if !has_extension {
            let with_exe = candidate.with_extension("exe");
            if with_exe.is_file() {
                return with_exe;
            }
        }
    }

    program_path.to_path_buf()
}

/// Builds a Windows command line using the `CommandLineToArgvW` quoting rules.
///
/// `CreateProcessW` performs its own parsing, so arguments containing spaces,
/// quotes, or trailing backslashes must be escaped exactly the way the C
/// runtime expects. The algorithm is the well-known `list2cmdline` one.
fn build_command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut line = Vec::new();
    append_quoted(&mut line, &program.encode_wide().collect::<Vec<u16>>());
    for argument in args {
        line.push(u16::from(b' '));
        append_quoted(&mut line, &argument.encode_wide().collect::<Vec<u16>>());
    }
    line.push(0);
    line
}

fn append_quoted(output: &mut Vec<u16>, argument: &[u16]) {
    let needs_quotes = argument.is_empty()
        || argument
            .iter()
            .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x22));

    if needs_quotes {
        output.push(u16::from(b'"'));
    }

    let mut backslashes = 0usize;
    for unit in argument {
        if *unit == u16::from(b'\\') {
            backslashes += 1;
            output.push(*unit);
        } else if *unit == u16::from(b'"') {
            // 2n+1 backslashes before a quote: n already emitted, so emit n+1.
            for _ in 0..=backslashes {
                output.push(u16::from(b'\\'));
            }
            output.push(*unit);
            backslashes = 0;
        } else {
            backslashes = 0;
            output.push(*unit);
        }
    }

    if needs_quotes {
        // Double trailing backslashes so they do not escape the closing quote.
        for _ in 0..backslashes {
            output.push(u16::from(b'\\'));
        }
        output.push(u16::from(b'"'));
    }
}

/// Builds a UTF-16 environment block from the parent environment plus overrides.
///
/// Returns `None` when there are no overrides, which tells `CreateProcessW` to
/// inherit the parent environment unchanged.
fn build_environment(overrides: &BTreeMap<OsString, OsString>) -> Option<Vec<u16>> {
    if overrides.is_empty() {
        return None;
    }

    let mut entries: Vec<(String, OsString)> = std::env::vars_os()
        .map(|(key, value)| (key.to_string_lossy().into_owned(), value))
        .collect();

    for (key, value) in overrides {
        let name = key.to_string_lossy();
        if let Some(existing) = entries
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            existing.1 = value.clone();
        } else {
            entries.push((name.into_owned(), value.clone()));
        }
    }

    entries.sort_by(|left, right| {
        left.0
            .to_ascii_uppercase()
            .cmp(&right.0.to_ascii_uppercase())
    });

    let mut block = Vec::new();
    for (key, value) in entries {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            continue;
        }
        block.extend(key.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    // Double NUL terminates the block.
    block.push(0);
    Some(block)
}

#[cfg(test)]
mod tests {
    use super::{append_quoted, build_command_line, build_environment, resolve_program};
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};

    fn command_line(program: &OsStr, args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let wide = build_command_line(program, &args);
        String::from_utf16(&wide[..wide.len() - 1]).expect("command line is valid UTF-16")
    }

    #[test]
    fn plain_arguments_are_not_quoted() {
        assert_eq!(
            command_line(OsStr::new(r"C:\burrow\testbin.exe"), &["exit", "7"]),
            r"C:\burrow\testbin.exe exit 7"
        );
    }

    #[test]
    fn program_with_spaces_is_quoted() {
        assert_eq!(
            command_line(OsStr::new(r"C:\Program Files\burrow\testbin.exe"), &[]),
            r#""C:\Program Files\burrow\testbin.exe""#
        );
    }

    #[test]
    fn empty_argument_becomes_empty_quotes() {
        assert_eq!(command_line(OsStr::new("app.exe"), &[""]), r#"app.exe """#);
    }

    #[test]
    fn quotes_inside_arguments_are_escaped() {
        assert_eq!(
            command_line(OsStr::new("app.exe"), &[r#"say "hi""#]),
            r#"app.exe "say \"hi\"""#
        );
    }

    #[test]
    fn trailing_backslashes_before_closing_quote_are_doubled() {
        assert_eq!(
            command_line(OsStr::new("app.exe"), &[r"C:\path with space\"]),
            r#"app.exe "C:\path with space\\""#
        );
    }

    #[test]
    fn append_quoted_preserves_tabs() {
        let mut output = Vec::new();
        append_quoted(&mut output, &[u16::from(b'a'), 0x09, u16::from(b'b')]);
        assert_eq!(output.first(), Some(&u16::from(b'"')));
        assert_eq!(output.last(), Some(&u16::from(b'"')));
    }

    #[test]
    fn no_overrides_means_inherit_the_environment() {
        assert!(build_environment(&BTreeMap::new()).is_none());
    }

    #[test]
    fn overrides_are_present_in_a_double_nul_terminated_block() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            OsString::from("BURROW_SUPERVISOR_TEST"),
            OsString::from("present"),
        );

        let block = build_environment(&overrides).expect("overrides produce a block");
        assert_eq!(block.last(), Some(&0));

        let entries: Vec<String> = block
            .split(|unit| *unit == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf16(part).expect("block is valid UTF-16"))
            .collect();
        assert!(
            entries
                .iter()
                .any(|entry| entry == "BURROW_SUPERVISOR_TEST=present"),
            "missing override in {entries:?}"
        );
    }

    #[test]
    fn bare_program_names_are_resolved_on_path_with_exe_suffix() {
        let temp = tempfile::tempdir().expect("temp dir");
        let executable = temp.path().join("burrow-fake-tool.exe");
        std::fs::write(&executable, b"not a real executable").expect("write fake executable");

        let mut env = BTreeMap::new();
        env.insert(
            OsString::from("PATH"),
            temp.path().as_os_str().to_os_string(),
        );

        assert_eq!(
            resolve_program(OsStr::new("burrow-fake-tool"), None, &env),
            executable
        );
    }

    #[test]
    fn explicit_program_paths_are_not_searched_on_path() {
        let resolved = resolve_program(OsStr::new(r".\tools\app.exe"), None, &BTreeMap::new());
        assert_eq!(resolved, PathBuf::from(r".\tools\app.exe"));
    }

    #[test]
    fn relative_program_paths_are_resolved_against_the_child_cwd() {
        let resolved = resolve_program(
            OsStr::new(r"bin\app.exe"),
            Some(Path::new(r"C:\project")),
            &BTreeMap::new(),
        );
        assert_eq!(resolved, PathBuf::from(r"C:\project\bin\app.exe"));
    }
}
