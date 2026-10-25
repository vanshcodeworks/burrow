# Compatibility

Burrow targets Windows 10 and Windows 11 on x86-64. This page records what has
actually been exercised on real hardware and what remains unverified. It is a
test log, not a guarantee: a row marked *unverified* has no evidence behind it
yet, and should be treated as unsupported until someone runs the probe there.

## Verified environment

| Item | Value |
| --- | --- |
| OS | Windows 11 Home Single Language 10.0.26200, build 26200 |
| Architecture | x86-64 (AMD64) |
| CPU / RAM | AMD Ryzen 7 4800H, 15.4 GB |
| Session | Non-elevated, medium integrity |
| PowerShell | 7.6.7 and Windows PowerShell 5.1.26100.9711 |
| Locale | `en-NZ` |
| `LongPathsEnabled` | `1` |
| Date | 2026-10-26 |
| Burrow | 0.1.0 release build from `target\release\burrow.exe` |

The account is a member of the local `Administrators` group but the probe ran
with a filtered, non-elevated token. A true standard-user account has not been
tested.

## Matrix

| Area | Result | Evidence |
| --- | --- | --- |
| Windows 11 x86-64 | **Verified** | Full lifecycle passed on build 26200. |
| Windows 10 x86-64 | **Unverified** | No Windows 10 machine was available. |
| Windows 11 ARM64 | **Unverified** | No ARM64 hardware was available. |
| Non-elevated session | **Verified** | `Elevated: False`; `doctor` passed. |
| True standard user (not in `Administrators`) | **Unverified** | The test account is an administrator running filtered. |
| ASCII project path | **Verified** | 64-character path; `up`, HTTP 200, `doctor`, `down` passed. |
| Path with spaces | **Verified** | 81-character path with two spaces. |
| Non-ASCII project path | **Verified** | 68-character path containing `é`, `测试`, and `Δ`. |
| Project path longer than 260 characters | **Verified** | 279-character path on NTFS with 8.3 short names; see below. |
| `LongPathsEnabled=0` | **Unverified** | Changing the machine-wide setting was out of scope. The long-path fix uses 8.3 short names, not this flag, but the combination was not run. |
| Volume with 8.3 names disabled | **Unverified** | Burrow returns an explicit error in that case; no such volume was available. |
| OneDrive-synced project directory | **Unverified** | OneDrive exists on the machine but was not used, to avoid syncing test files. |
| UNC or network-share project directory | **Unverified** | No network share was available. |
| FAT or exFAT volume | **Unverified** | No such volume was available. |
| Third-party antivirus or corporate lockdown | **Unverified** | Only the default Windows configuration was used. |
| PowerShell 7 launcher | **Verified** | Probe completed with PowerShell 7.6.7. |
| Windows PowerShell 5.1 launcher | **Verified** | Probe completed with 5.1.26100.9711. |
| Docker container fallback | **Unverified here** | Docker 24.0.7 is installed; this probe does not cover containers. |
| Podman container fallback | **Unverified** | Podman is not installed. |

## What the probe covers

[`scripts/compatibility.ps1`](../scripts/compatibility.ps1) runs a package-free
project through the real CLI in four directory shapes. It:

1. Creates a unique directory under `%TEMP%` and writes a `burrow.toml` plus a
   small PowerShell TCP listener. The listener binds `127.0.0.1:$env:PORT` and
   returns a minimal HTTP 200 response.
2. Runs `burrow --project <dir> up --detach --yes`.
3. Polls `burrow status --json` until the service is `healthy` (60-second
   budget), then makes a direct HTTP request to the assigned loopback port.
4. Runs `burrow doctor --json` and requires `ok = true`.
5. Runs `burrow down` and checks that `.burrow/state.json` is gone.
6. Repeats for ASCII, spaces, non-ASCII, and a path longer than 260 characters.

It downloads no packages, binds no privileged port, needs no administrator
rights, and removes its work root unless `-Keep` is passed or a check fails.
It does not exercise managed packages, the reverse proxy, snapshots, the
dashboard, or containers.

```powershell
# From a checkout, against the release binary
.\scripts\compatibility.ps1 -Burrow .\target\release\burrow.exe

# Keep the work root and emit machine-readable output
.\scripts\compatibility.ps1 -Burrow .\target\release\burrow.exe -Keep -Json
```

`-Burrow` defaults to `burrow` on `PATH`. `-WorkRoot <DIR>` places the test
projects elsewhere. Exit code `0` means every executed check passed; `1` means
at least one failed. A failed run keeps its work root and prints its path.

Output from the Windows 11 run on 2026-10-26 (ports are assigned per run):

```text
PASS path: ASCII                path 64 chars; healthy on port 40313, HTTP 200, doctor 9 ok / 0 warnings, stopped cleanly
PASS path: spaces               path 81 chars; healthy on port 46833, HTTP 200, doctor 9 ok / 0 warnings, stopped cleanly
PASS path: non-ASCII            path 68 chars; healthy on port 49143, HTTP 200, doctor 9 ok / 0 warnings, stopped cleanly
PASS path: long (> 260 chars)   path 279 chars; healthy on port 48259, HTTP 200, doctor 9 ok / 0 warnings, stopped cleanly

RESULT: 4 passed, 0 failed, 0 skipped
```

The script is deliberately pure ASCII: Windows PowerShell 5.1 reads a BOM-less
script as ANSI, so literal non-ASCII test paths would be mangled before they
are used.

The CI workflow runs the same probe on `windows-latest` after the release
build. That runner is a Windows Server image, not Windows 10 or Windows 11, so
its result is tracked separately from the rows above.

## Long paths

`CreateProcessW` rejects a working directory of 260 UTF-16 units or more with
`ERROR_DIRECTORY` (os error 267), and it rejects the `\\?\` verbatim prefix in
that argument. `cmd.exe` also treats a verbatim directory as a UNC path and
silently runs in `C:\Windows`. Burrow therefore:

- passes short paths through unchanged, after removing a verbatim prefix;
- resolves the existing 8.3 short name for a longer path, removes the prefix,
  and uses that short path as the process working directory and, when needed,
  as the application path;
- keeps `BURROW_PROJECT_DIR` and all user-facing output in the original long
  form.

If the short name cannot be resolved, or is still 260 units or more (for
example because 8.3 name generation is disabled on the volume), the start fails
before any process is created with:

```text
error: working directory `C:\...` is too long for a child process and its 8.3
short name could not be resolved (...); move the project to a path shorter
than 260 characters
```

The exact behavior and its limits are in
[ADR 0010](decisions/0010-long-path-process-working-directories.md).

## Reporting a result

If you run the probe on another machine, keep the JSON output:

```powershell
.\scripts\compatibility.ps1 -Json | Out-File -Encoding utf8 compatibility.json
```

It records the OS build, architecture, PowerShell version, elevation state,
`LongPathsEnabled`, locale, work root, and every result. Open an issue with that
file and the Burrow version from `burrow --version`. Do not attach a diagnostic
bundle without reviewing it first; `burrow doctor --report` is local and
redacted, but redaction is best-effort.

Rows marked unverified are the useful reports: Windows 10, ARM64, a true
standard-user account, OneDrive or network-share projects, and non-NTFS
volumes.
