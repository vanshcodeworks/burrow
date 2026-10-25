# Burrow compatibility probe.
#
# Runs a small, package-free project through the real CLI in several directory
# shapes. It creates everything under a unique temporary directory, stops each
# project before moving on, and removes the directory unless -Keep is passed.
#
# Usage:
#   .\scripts\compatibility.ps1
#   .\scripts\compatibility.ps1 -Burrow C:\path\to\burrow.exe -Keep
#   .\scripts\compatibility.ps1 -Json > compatibility.json
#
# Exit code: 0 when every executed check passes, 1 when any check fails.

[CmdletBinding()]
param(
    [string]$Burrow = "burrow",
    [string]$WorkRoot,
    [switch]$Keep,
    [switch]$Json
)

$ErrorActionPreference = "Stop"

function Write-Utf8File {
    param([string]$Path, [string]$Text)
    $target = if ($Path.Length -ge 240) { "\\?\$Path" } else { $Path }
    [System.IO.File]::WriteAllText($target, $Text, [System.Text.UTF8Encoding]::new($false))
}

function New-TestDirectory {
    param([string]$Path)
    $target = if ($Path.Length -ge 240) { "\\?\$Path" } else { $Path }
    New-Item -ItemType Directory -Path $target -Force | Out-Null
}

function Test-TestPath {
    param([string]$Path)
    $target = if ($Path.Length -ge 240) { "\\?\$Path" } else { $Path }
    return Test-Path -LiteralPath $target
}

function Remove-TestDirectory {
    param([string]$Path)
    $target = if ($Path.Length -ge 240) { "\\?\$Path" } else { $Path }
    Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction SilentlyContinue
}

function Invoke-Burrow {
    param([string[]]$Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $script:BurrowPath @Arguments 2>&1
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 1 }
        return [pscustomobject]@{
            ExitCode = $code
            Output = (($output | ForEach-Object { "$_" }) -join "`n")
        }
    }
    catch {
        return [pscustomobject]@{
            ExitCode = 1
            Output = $_.Exception.Message
        }
    }
    finally {
        $ErrorActionPreference = $previous
    }
}

function Get-HttpStatusLine {
    param([int]$Port)
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect("127.0.0.1", $Port)
        $client.ReceiveTimeout = 5000
        $stream = $client.GetStream()
        $request = "GET / HTTP/1.1`r`nHost: 127.0.0.1`r`nConnection: close`r`n`r`n"
        $bytes = [System.Text.Encoding]::ASCII.GetBytes($request)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush()
        $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::ASCII)
        return $reader.ReadLine()
    }
    finally {
        $client.Close()
    }
}

$serviceScript = @'
$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, [int]$env:PORT)
$listener.Start()
while ($true) {
    $client = $listener.AcceptTcpClient()
    $stream = $client.GetStream()
    $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::ASCII, $false, 1024, $true)
    while (($line = $reader.ReadLine()) -ne $null -and $line -ne "") { }
    $body = "burrow compatibility check"
    $response = "HTTP/1.1 200 OK`r`nContent-Type: text/plain`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body"
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($response)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
    $client.Close()
}
'@

function New-TestProject {
    param([string]$Directory, [string]$Name)
    New-TestDirectory $Directory
    Write-Utf8File (Join-Path $Directory "compat-service.ps1") $serviceScript
    $config = @"
[project]
name = "$Name"

[services.api]
run = 'powershell -NoProfile -ExecutionPolicy Bypass -File compat-service.ps1'
port = "auto"
health = { http = "/", timeout = "30s", interval = "250ms" }
"@
    Write-Utf8File (Join-Path $Directory "burrow.toml") $config
}

function Add-Result {
    param([string]$Name, [string]$Status, [string]$Detail)
    $script:results.Add([pscustomobject]@{
        name = $Name
        status = $Status
        detail = $Detail
    })
}

function Test-Lifecycle {
    param([string]$Label, [string]$Directory, [string]$Name)
    $projectDir = $Directory
    try {
        New-TestProject $projectDir $Name
    }
    catch {
        Add-Result $Label "skip" "could not create the test directory: $($_.Exception.Message)"
        return
    }

    $up = $null
    $stopped = $false
    try {
        $up = Invoke-Burrow @("--project", $projectDir, "up", "--detach", "--yes")
        if ($up.ExitCode -ne 0) {
            Add-Result $Label "fail" "up exited $($up.ExitCode): $($up.Output)"
            return
        }

        $state = $null
        $deadline = (Get-Date).AddSeconds(60)
        while ((Get-Date) -lt $deadline) {
            $status = Invoke-Burrow @("--project", $projectDir, "status", "--json")
            if ($status.ExitCode -eq 0) {
                try {
                    $candidate = $status.Output | ConvertFrom-Json
                    if ($candidate.services.api.status -eq "healthy") {
                        $state = $candidate
                        break
                    }
                }
                catch { }
            }
            Start-Sleep -Milliseconds 250
        }
        if ($null -eq $state) {
            Add-Result $Label "fail" "api did not become healthy within 60 s"
            return
        }

        $port = [int]$state.services.api.port
        $statusLine = Get-HttpStatusLine $port
        if ($statusLine -notmatch "^HTTP/1\.[01] 200") {
            Add-Result $Label "fail" "health endpoint returned '$statusLine' on port $port"
            return
        }

        $doctor = Invoke-Burrow @("--project", $projectDir, "doctor", "--json")
        if ($doctor.ExitCode -ne 0) {
            Add-Result $Label "fail" "doctor exited $($doctor.ExitCode): $($doctor.Output)"
            return
        }
        $doctorReport = $doctor.Output | ConvertFrom-Json
        if ($doctorReport.ok -ne $true) {
            Add-Result $Label "fail" "doctor reported failures: $($doctor.Output)"
            return
        }

        $down = Invoke-Burrow @("--project", $projectDir, "down")
        if ($down.ExitCode -ne 0) {
            Add-Result $Label "fail" "down exited $($down.ExitCode): $($down.Output)"
            return
        }
        $stopped = $true
        if (Test-TestPath (Join-Path $projectDir ".burrow\state.json")) {
            Add-Result $Label "fail" "down left .burrow/state.json behind"
            return
        }

        Add-Result $Label "pass" ("path {0} chars; healthy on port {1}, HTTP 200, doctor {2} ok / {3} warnings, stopped cleanly" -f $projectDir.Length, $port, $doctorReport.summary.ok, $doctorReport.summary.warnings)
    }
    finally {
        if ($null -ne $up -and -not $stopped) {
            Invoke-Burrow @("--project", $projectDir, "down") | Out-Null
        }
    }
}

if ($null -eq $WorkRoot -or $WorkRoot -eq "") {
    $WorkRoot = Join-Path $env:TEMP ("burrow-compat-" + [Guid]::NewGuid().ToString("N").Substring(0, 8))
}

$script:results = [System.Collections.Generic.List[object]]::new()
$script:BurrowPath = $Burrow

$command = Get-Command $Burrow -ErrorAction SilentlyContinue
if ($null -eq $command) {
    Write-Error "Burrow executable not found: $Burrow"
    exit 1
}
$script:BurrowPath = $command.Source
if (-not $script:BurrowPath) { $script:BurrowPath = $command.Name }

$version = Invoke-Burrow @("--version")
if ($version.ExitCode -ne 0) {
    Write-Error "burrow --version failed: $($version.Output)"
    exit 1
}

$os = $null
try {
    $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
}
catch { }
$osLabel = if ($null -ne $os) {
    "$($os.Caption) $($os.Version) build $($os.BuildNumber)"
} else {
    [System.Environment]::OSVersion.VersionString
}

$elevated = $false
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    $elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}
catch { }

$longPaths = $null
try {
    $longPaths = (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" -Name LongPathsEnabled -ErrorAction Stop).LongPathsEnabled
}
catch { }

$environment = [pscustomobject]@{
    os = $osLabel
    architecture = $env:PROCESSOR_ARCHITECTURE
    powershell = $PSVersionTable.PSVersion.ToString()
    elevated = $elevated
    long_paths_enabled = $longPaths
    culture = (Get-Culture).Name
    work_root = $WorkRoot
}

New-TestDirectory $WorkRoot

Test-Lifecycle "path: ASCII" (Join-Path $WorkRoot "ascii 01") "compatascii"
Test-Lifecycle "path: spaces" (Join-Path $WorkRoot "plain path with spaces 02") "compatspaces"
# Built from code points so this file stays ASCII: Windows PowerShell 5.1
# reads BOM-less scripts as ANSI and would mangle literal non-ASCII text.
$unicodeName = "caf" + [char]0x00E9 + " " + [char]0x6D4B + [char]0x8BD5 + " " +
    [char]0x0394 + " 03"
Test-Lifecycle "path: non-ASCII" (Join-Path $WorkRoot $unicodeName) "compatunicode"

$long = $WorkRoot
$index = 0
while ($long.Length -lt 250) {
    $long = Join-Path $long ("long-segment-" + ("x" * 20) + "-$index")
    $index++
}
$long = Join-Path $long "project"
Test-Lifecycle "path: long (> 260 chars)" $long "compatlong"

$summary = [pscustomobject]@{
    passed = @($script:results | Where-Object { $_.status -eq "pass" }).Count
    failed = @($script:results | Where-Object { $_.status -eq "fail" }).Count
    skipped = @($script:results | Where-Object { $_.status -eq "skip" }).Count
}

$report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    burrow = $script:BurrowPath
    version = ($version.Output -replace "^burrow\s+", "")
    environment = $environment
    results = $script:results
    summary = $summary
}

if ($Json) {
    $report | ConvertTo-Json -Depth 5
}
else {
    Write-Output "Burrow compatibility check"
    Write-Output "=========================="
    Write-Output "Burrow:     $($report.burrow)"
    Write-Output "Version:    $($report.version)"
    Write-Output "OS:         $($environment.os)"
    Write-Output "Arch:       $($environment.architecture)"
    Write-Output "PowerShell: $($environment.powershell)"
    Write-Output "Elevated:   $($environment.elevated)"
    Write-Output "LongPaths:  $($environment.long_paths_enabled)"
    Write-Output "Culture:    $($environment.culture)"
    Write-Output "Work root:  $WorkRoot"
    Write-Output ""
    foreach ($result in $script:results) {
        Write-Output ("{0,-4} {1,-26} {2}" -f $result.status.ToUpperInvariant(), $result.name, $result.detail)
    }
    Write-Output ""
    Write-Output ("RESULT: {0} passed, {1} failed, {2} skipped" -f $summary.passed, $summary.failed, $summary.skipped)
    if ($summary.failed -gt 0) {
        Write-Output "Set -Keep to inspect the work root."
    }
}

$keepWorkRoot = $Keep -or ($summary.failed -gt 0)
if (-not $keepWorkRoot) {
    foreach ($child in Get-ChildItem -LiteralPath $WorkRoot -Directory -ErrorAction SilentlyContinue) {
        Remove-TestDirectory $child.FullName
    }
    Remove-TestDirectory $WorkRoot
}
else {
    if ($Json) {
        [Console]::Error.WriteLine("Kept: $WorkRoot")
    }
    else {
        Write-Output "Kept: $WorkRoot"
    }
}

if ($summary.failed -gt 0) { exit 1 }
exit 0
