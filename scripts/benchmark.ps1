# Measures Burrow startup, daemon memory, and total environment memory.
#
# This script reports real numbers from the machine it runs on. It never
# estimates and never invents a Docker comparison: when the Docker daemon is not
# running, the comparison is explicitly marked as skipped.
#
# Usage:
#   .\scripts\benchmark.ps1                       # warm start
#   .\scripts\benchmark.ps1 -Cold                 # fresh cache, cold then warm
#   .\scripts\benchmark.ps1 -Project examples\python-postgres
#
# The project must already have its dependencies installed and must be stopped.

[CmdletBinding()]
param(
    [string]$Project = (Join-Path $PSScriptRoot "..\examples\node-postgres"),
    [string]$Burrow = (Join-Path $PSScriptRoot "..\target\release\burrow.exe"),
    [string]$CacheDir = (Join-Path $PSScriptRoot "..\target\benchmark-cache"),
    [switch]$Cold,
    [int]$TimeoutSeconds = 600,
    [string]$JsonPath
)

$ErrorActionPreference = "Stop"

function Get-ProcessTreeIds {
    param([int]$RootPid)

    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $queue.Enqueue($RootPid)
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if (-not $seen.Add($current)) { continue }
        $children = Get-CimInstance Win32_Process -Filter "ParentProcessId=$current" |
            Select-Object -ExpandProperty ProcessId
        foreach ($child in $children) { $queue.Enqueue([int]$child) }
    }
    return $seen
}

function Get-ProcessTreeMemory {
    param([int]$RootPid)

    $total = [int64]0
    $count = 0
    foreach ($processId in (Get-ProcessTreeIds -RootPid $RootPid)) {
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($process) {
            $total += $process.WorkingSet64
            $count += 1
        }
    }
    return [pscustomobject]@{ Bytes = $total; Processes = $count }
}

function Wait-UntilReady {
    param([string]$StatePath, [System.Diagnostics.Stopwatch]$Stopwatch)

    while ($Stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if (Test-Path $StatePath) {
            try {
                $state = Get-Content $StatePath -Raw | ConvertFrom-Json
                if ($state.ready) { return $state }
            } catch {
                # The daemon writes state atomically, but a read can still race a
                # rename on some filesystems. Retry.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Burrow did not become ready within $TimeoutSeconds seconds"
}

function Invoke-Measurement {
    param([string]$Label, [string]$Cache)

    & $Burrow --project $Project down | Out-Null

    if ($Cache) {
        $env:BURROW_CACHE_DIR = $Cache
    } else {
        Remove-Item Env:BURROW_CACHE_DIR -ErrorAction SilentlyContinue
    }

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    & $Burrow --project $Project up --detach --yes | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "burrow up failed with exit code $LASTEXITCODE" }

    $statePath = Join-Path $Project ".burrow\state.json"
    $state = Wait-UntilReady -StatePath $statePath -Stopwatch $stopwatch
    $startup = $stopwatch.Elapsed.TotalSeconds

    # Let CPU sampling and process trees settle before reading memory.
    Start-Sleep -Seconds 2
    $state = Get-Content $statePath -Raw | ConvertFrom-Json
    $daemon = Get-Process -Id ([int]$state.pid)
    $daemonMb = [math]::Round($daemon.WorkingSet64 / 1MB, 1)
    $tree = Get-ProcessTreeMemory -RootPid ([int]$state.pid)
    $treeMb = [math]::Round($tree.Bytes / 1MB, 1)
    $serviceMemory = [int64]0
    $serviceProcesses = 0
    foreach ($service in $state.services.PSObject.Properties) {
        if ($null -eq $service.Value.pid) { continue }
        $serviceTree = Get-ProcessTreeMemory -RootPid ([int]$service.Value.pid)
        $serviceMemory += $serviceTree.Bytes
        $serviceProcesses += $serviceTree.Processes
    }

    & $Burrow --project $Project down | Out-Null
    $stopwatch.Stop()

    return [pscustomobject]@{
        label                 = $Label
        startup_seconds       = [math]::Round($startup, 2)
        daemon_working_set_mb = $daemonMb
        total_tree_mb         = $treeMb
        tree_processes        = $tree.Processes
        services_mb           = [math]::Round($serviceMemory / 1MB, 1)
        service_processes     = $serviceProcesses
        cache                 = $Cache
    }
}

if (-not (Test-Path $Burrow)) {
    throw "Burrow binary not found at $Burrow. Build it with: cargo build --release -p burrow-cli"
}
if (-not (Test-Path (Join-Path $Project "burrow.toml"))) {
    throw "$Project does not contain burrow.toml"
}
$Project = (Resolve-Path $Project).Path

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$computer = Get-CimInstance Win32_ComputerSystem
$os = Get-CimInstance Win32_OperatingSystem
$docker = $null
try {
    $server = docker info --format "{{.ServerVersion}}" 2>$null
    if ($LASTEXITCODE -eq 0 -and $server) { $docker = $server }
} catch {
    $docker = $null
}

$rustVersion = "unavailable"
try {
    $rustVersion = (& rustc --version 2>$null)
} catch {
    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\rustc.exe"
    if (Test-Path $candidate) {
        $rustVersion = (& $candidate --version 2>$null)
    } else {
        $rustVersion = "unavailable (rustc is not on PATH)"
    }
}

$machine = [pscustomobject]@{
    cpu             = $cpu.Name.Trim()
    cores           = $cpu.NumberOfCores
    logical_cores   = $cpu.NumberOfLogicalProcessors
    memory_gb       = [math]::Round($computer.TotalPhysicalMemory / 1GB, 1)
    os              = $os.Caption.Trim()
    os_version      = $os.Version
    rust            = $rustVersion
    burrow          = (& $Burrow --version)
    docker_daemon   = $docker
    docker_skipped  = ($null -eq $docker)
}

$measurements = @()
if ($Cold) {
    if (Test-Path $CacheDir) { Remove-Item -Recurse -Force $CacheDir }
    $measurements += Invoke-Measurement -Label "cold (empty cache)" -Cache $CacheDir
    $measurements += Invoke-Measurement -Label "warm (cache populated)" -Cache $CacheDir
} else {
    # An existing cache directory is reused so a second machine is not forced
    # to download 356 MB just to measure a warm start. Without one, Burrow uses
    # its normal per-user cache.
    $warmCache = if (Test-Path $CacheDir) { (Resolve-Path $CacheDir).Path } else { $null }
    $measurements += Invoke-Measurement -Label "warm" -Cache $warmCache
}

$report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    project      = $Project
    machine      = $machine
    measurements = $measurements
    notes        = @(
        "Startup is measured from launching 'burrow up --detach' until .burrow/state.json reports ready.",
        "daemon_working_set_mb is the Burrow daemon process only.",
        "total_tree_mb sums the daemon and every descendant process (services included).",
        "Docker comparisons are only reported when the Docker daemon is running."
    )
}

$report | ConvertTo-Json -Depth 6
if ($JsonPath) {
    $report | ConvertTo-Json -Depth 6 | Set-Content -Path $JsonPath -Encoding utf8
}
