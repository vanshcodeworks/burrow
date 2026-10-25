# Compares Burrow and Docker Compose on the same project and endpoint.
#
# The Docker side uses a generated Compose fixture that mirrors
# examples/node-postgres: the same server.js, schema, and seed. The only app
# change is that the container binds 0.0.0.0 so Docker can forward the port.
#
# Setup (image pull/build) is measured separately and excluded from startup.
# Startup is wall-clock time from launching the stack until the API returns
# HTTP 200 with the seeded row. Cold means an empty data directory/volume;
# warm means the data directory/volume is retained between runs.
#
# If the Docker daemon is not running, the Compose half is reported as skipped
# and the Burrow half still runs. The script never invents a number.
#
# Usage:
#   .\scripts\benchmark-stacks.ps1
#   .\scripts\benchmark-stacks.ps1 -Project examples\node-postgres -JsonPath out.json
#   .\scripts\benchmark-stacks.ps1 -WarmRuns 3

[CmdletBinding()]
param(
    [string]$Project = (Join-Path $PSScriptRoot "..\examples\node-postgres"),
    [string]$Burrow = (Join-Path $PSScriptRoot "..\target\release\burrow.exe"),
    [string]$CacheDir = (Join-Path $PSScriptRoot "..\target\benchmark-cache"),
    [int]$Port = 3000,
    [string]$ApiService = "api",
    [string]$HttpPath = "/api/todos",
    [string]$Expected = "Open http://api.localhost",
    [int]$WarmRuns = 2,
    [int]$TimeoutSeconds = 300,
    [string]$JsonPath,
    [switch]$Keep
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

function Get-TreeMemory {
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

function Wait-Http200 {
    param([string]$Url, [System.Diagnostics.Stopwatch]$Stopwatch)

    while ($Stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        try {
            $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
            if ($response.StatusCode -eq 200 -and $response.Content -match [regex]::Escape($Expected)) {
                return $response.Content
            }
        } catch {
            # The stack is still starting.
        }
        Start-Sleep -Milliseconds 100
    }
    throw "HTTP 200 with the expected body was not reached within $TimeoutSeconds seconds"
}

function Get-BurrowApiPort {
    param([string]$StatePath)

    $state = Get-Content $StatePath -Raw | ConvertFrom-Json
    $service = $state.services.PSObject.Properties[$ApiService]
    if (-not $service) { throw "state.json has no service named '$ApiService'" }
    return [int]$service.Value.port
}

function Get-DaemonWorkingSetMb {
    param([int]$TargetPid)

    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        $process = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
        if ($process -and $process.WorkingSet64 -gt 0) {
            return [math]::Round($process.WorkingSet64 / 1MB, 1)
        }
        $cim = Get-CimInstance Win32_Process -Filter "ProcessId=$TargetPid" -ErrorAction SilentlyContinue
        if ($cim -and $cim.WorkingSetSize -gt 0) {
            return [math]::Round($cim.WorkingSetSize / 1MB, 1)
        }
        Start-Sleep -Milliseconds 200
    }
    return $null
}

function Measure-Burrow {
    param([string]$Label)

    & $Burrow --project $BurrowProject down 2>&1 | Out-Null

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $upOutput = & $Burrow --project $BurrowProject up --detach --yes 2>&1
    $upExit = $LASTEXITCODE
    if ($upExit -ne 0) {
        throw "burrow up failed with exit code ${upExit}: $($upOutput -join "`n")"
    }

    $statePath = Join-Path $BurrowProject ".burrow\state.json"
    while (-not (Test-Path $statePath)) {
        if ($stopwatch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            throw "Burrow did not write state.json within $TimeoutSeconds seconds"
        }
        Start-Sleep -Milliseconds 50
    }

    $port = Get-BurrowApiPort -StatePath $statePath
    $content = Wait-Http200 -Url "http://127.0.0.1:$port$HttpPath" -Stopwatch $stopwatch
    $ready = $stopwatch.Elapsed.TotalSeconds

    Start-Sleep -Seconds 2
    $state = Get-Content $statePath -Raw | ConvertFrom-Json
    $daemonMb = Get-DaemonWorkingSetMb -TargetPid ([int]$state.pid)
    $tree = Get-TreeMemory -RootPid ([int]$state.pid)
    $serviceBytes = [int64]0
    $serviceProcesses = 0
    foreach ($service in $state.services.PSObject.Properties) {
        if ($null -eq $service.Value.pid) { continue }
        $serviceTree = Get-TreeMemory -RootPid ([int]$service.Value.pid)
        $serviceBytes += $serviceTree.Bytes
        $serviceProcesses += $serviceTree.Processes
    }

    & $Burrow --project $BurrowProject down 2>&1 | Out-Null

    return [pscustomobject]@{
        label                 = $Label
        http_200_seconds      = [math]::Round($ready, 2)
        daemon_working_set_mb = $daemonMb
        total_tree_mb         = [math]::Round($tree.Bytes / 1MB, 1)
        tree_processes        = $tree.Processes
        services_mb           = [math]::Round($serviceBytes / 1MB, 1)
        service_processes     = $serviceProcesses
        api_port              = $port
        response              = $content.Trim()
    }
}

function Measure-Compose {
    param([string]$Label)

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $upOutput = & docker compose -p $ComposeProject up -d 2>&1
    $upExit = $LASTEXITCODE
    $upReturn = $stopwatch.Elapsed.TotalSeconds
    if ($upExit -ne 0) {
        throw "docker compose up failed with exit code ${upExit}: $($upOutput -join "`n")"
    }

    $content = Wait-Http200 -Url "http://127.0.0.1:$Port$HttpPath" -Stopwatch $stopwatch
    $ready = $stopwatch.Elapsed.TotalSeconds

    Start-Sleep -Seconds 2
    $ids = @(& docker compose -p $ComposeProject ps -q)
    $stats = if ($ids.Count -gt 0) {
        & docker stats --no-stream --format "{{.Name}}|{{.MemUsage}}|{{.CPUPerc}}" @ids
    } else {
        @()
    }

    return [pscustomobject]@{
        label                     = $Label
        compose_up_return_seconds = [math]::Round($upReturn, 2)
        http_200_seconds          = [math]::Round($ready, 2)
        container_stats           = @($stats)
        response                  = $content.Trim()
    }
}

function Stop-Compose {
    param([switch]$RemoveVolume)

    $arguments = @("compose", "-p", $ComposeProject, "down", "--remove-orphans")
    if ($RemoveVolume) { $arguments += "-v" }
    & docker @arguments 2>&1 | Out-Null
}

function New-ComposeFixture {
    param([string]$Destination)

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Destination "db") | Out-Null
    Copy-Item (Join-Path $Project "server.js") -Destination $Destination -Force
    Copy-Item (Join-Path $Project "index.html") -Destination $Destination -Force
    Copy-Item (Join-Path $Project "package.json") -Destination $Destination -Force
    Copy-Item (Join-Path $Project "package-lock.json") -Destination $Destination -Force
    Copy-Item (Join-Path $Project "db\seed.sql") -Destination (Join-Path $Destination "db\seed.sql") -Force

    $serverPath = Join-Path $Destination "server.js"
    $server = Get-Content $serverPath -Raw
    $oldListen = 'server.listen(port, "127.0.0.1", () => {'
    $newListen = 'const host = process.env.HOST || "127.0.0.1";' + [Environment]::NewLine +
        'server.listen(port, host, () => {'
    if (-not $server.Contains($oldListen)) {
        throw "server.js no longer contains the expected listen call; update scripts\benchmark-stacks.ps1"
    }
    $server = $server.Replace($oldListen, $newListen)
    $server = $server.Replace(
        'console.log(`API listening on http://127.0.0.1:${port}`);',
        'console.log(`API listening on http://${host}:${port}`);'
    )
    Set-Content -Path $serverPath -Value $server -Encoding utf8

    $dockerfile = @'
FROM node:22.13.1-alpine

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev

COPY server.js index.html ./

EXPOSE 3000
CMD ["node", "server.js"]
'@
    Set-Content -Path (Join-Path $Destination "Dockerfile") -Value $dockerfile -Encoding utf8

    $compose = @"
name: $ComposeProject

services:
  db:
    image: postgres:16.15-alpine
    environment:
      POSTGRES_USER: burrow
      POSTGRES_PASSWORD: burrow
      POSTGRES_DB: burrow
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U burrow -d burrow"]
      interval: 300ms
      timeout: 5s
      retries: 200
    volumes:
      - db-data:/var/lib/postgresql/data
      - ./db/seed.sql:/docker-entrypoint-initdb.d/seed.sql:ro

  api:
    build:
      context: .
    environment:
      DATABASE_URL: postgres://burrow:burrow@db:5432/burrow
      PORT: "3000"
      HOST: "0.0.0.0"
    ports:
      - "127.0.0.1:${Port}:3000"
    depends_on:
      db:
        condition: service_healthy
    healthcheck:
      test:
        [
          "CMD",
          "node",
          "-e",
          "fetch('http://127.0.0.1:3000/health').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))",
        ]
      interval: 300ms
      timeout: 5s
      retries: 200

volumes:
  db-data:
"@
    Set-Content -Path (Join-Path $Destination "compose.yaml") -Value $compose -Encoding utf8

    $dockerignore = @'
node_modules
.burrow
db
README.md
'@
    Set-Content -Path (Join-Path $Destination ".dockerignore") -Value $dockerignore -Encoding utf8
}

if (-not (Test-Path $Burrow)) {
    throw "Burrow binary not found at $Burrow. Build it with: cargo build --release -p burrow-cli"
}
$Project = (Resolve-Path $Project).Path
if (-not (Test-Path (Join-Path $Project "burrow.toml"))) {
    throw "$Project does not contain burrow.toml"
}
foreach ($required in @("server.js", "index.html", "package.json", "package-lock.json", "db\seed.sql")) {
    if (-not (Test-Path (Join-Path $Project $required))) {
        throw "$Project is missing $required"
    }
}
if (-not (Test-Path (Join-Path $Project "node_modules"))) {
    throw "$Project is missing node_modules; run npm install first"
}

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
try {
    $listener.Start()
} catch {
    throw "TCP port $Port is already in use; pass a free port with -Port"
} finally {
    $listener.Stop()
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("burrow-stacks-" + $PID)
if (Test-Path $tempRoot) { Remove-Item -Recurse -Force $tempRoot }
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
$BurrowProject = Join-Path $tempRoot "burrow"
$ComposeDir = Join-Path $tempRoot "compose"
New-Item -ItemType Directory -Force -Path $BurrowProject | Out-Null
Copy-Item (Join-Path $Project "burrow.toml") -Destination $BurrowProject -Force
Copy-Item (Join-Path $Project "server.js") -Destination $BurrowProject -Force
Copy-Item (Join-Path $Project "index.html") -Destination $BurrowProject -Force
Copy-Item (Join-Path $Project "package.json") -Destination $BurrowProject -Force
Copy-Item (Join-Path $Project "package-lock.json") -Destination $BurrowProject -Force
Copy-Item (Join-Path $Project "node_modules") -Destination $BurrowProject -Recurse -Force
Copy-Item (Join-Path $Project "db") -Destination $BurrowProject -Recurse -Force

$ComposeProject = "burrow-bench-" + $PID
New-ComposeFixture -Destination $ComposeDir

$previousCache = $env:BURROW_CACHE_DIR
$cacheUsed = $null
if ($CacheDir) {
    if (-not (Test-Path $CacheDir)) {
        New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
    }
    $cacheUsed = (Resolve-Path $CacheDir).Path
    $env:BURROW_CACHE_DIR = $cacheUsed
}

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$computer = Get-CimInstance Win32_ComputerSystem
$os = Get-CimInstance Win32_OperatingSystem
$machine = [pscustomobject]@{
    cpu           = $cpu.Name.Trim()
    cores         = $cpu.NumberOfCores
    logical_cores = $cpu.NumberOfLogicalProcessors
    memory_gb     = [math]::Round($computer.TotalPhysicalMemory / 1GB, 1)
    os            = $os.Caption.Trim()
    os_version    = $os.Version
}

$burrowVersion = (& $Burrow --version)
$burrowMeasurements = @()
$dockerReport = [pscustomobject]@{ skipped = $true; reason = "Docker daemon is not running" }

Push-Location $ComposeDir
try {
    $dockerInfo = & docker info 2>&1
    $dockerAvailable = ($LASTEXITCODE -eq 0 -and ($dockerInfo -join "`n") -match "Server Version")
    if (-not $dockerAvailable) {
        $dockerReport = [pscustomobject]@{
            skipped = $true
            reason  = "Docker daemon is not running"
        }
    } else {
        $serverVersion = (& docker version --format "{{.Server.Version}}")
        $composeVersion = (& docker compose version --short)
        $serverInfo = (& docker info --format "{{json .}}") | ConvertFrom-Json

        $setup = [pscustomobject]@{}
        $pullWatch = [System.Diagnostics.Stopwatch]::StartNew()
        & docker compose -p $ComposeProject pull db 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "docker compose pull db failed" }
        $pullWatch.Stop()
        $setup | Add-Member -NotePropertyName pull_db_seconds -NotePropertyValue ([math]::Round($pullWatch.Elapsed.TotalSeconds, 2))

        $buildWatch = [System.Diagnostics.Stopwatch]::StartNew()
        & docker compose -p $ComposeProject build api 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "docker compose build api failed" }
        $buildWatch.Stop()
        $setup | Add-Member -NotePropertyName build_api_seconds -NotePropertyValue ([math]::Round($buildWatch.Elapsed.TotalSeconds, 2))

        $postgresDigest = (& docker image inspect postgres:16.15-alpine --format "{{index .RepoDigests 0}}")
        $postgresImageId = (& docker image inspect postgres:16.15-alpine --format "{{.Id}}")
        $postgresImageSize = (& docker image inspect postgres:16.15-alpine --format "{{.Size}}")
        $apiImage = (& docker image inspect "$ComposeProject-api" --format "{{.Id}}")
        $apiImageSize = (& docker image inspect "$ComposeProject-api" --format "{{.Size}}")

        Stop-Compose -RemoveVolume
        $volume = @(& docker volume ls --filter "name=$ComposeProject" --format "{{.Name}}")
        if ($volume.Count -ne 0) { throw "Compose volume still exists before the cold run" }

        $composeMeasurements = @()
        $composeMeasurements += Measure-Compose -Label "cold (empty volume, images cached)"
        Stop-Compose
        for ($run = 1; $run -le $WarmRuns; $run++) {
            $composeMeasurements += Measure-Compose -Label "warm $run (persistent volume, images cached)"
            Stop-Compose
        }
        Stop-Compose -RemoveVolume

        $dockerReport = [pscustomobject]@{
            skipped          = $false
            client_version   = (& docker version --format "{{.Client.Version}}")
            server_version   = $serverVersion
            compose_version  = $composeVersion
            os_type          = $serverInfo.OSType
            architecture     = $serverInfo.Architecture
            cpus             = $serverInfo.NCPU
            memory_bytes     = $serverInfo.MemTotal
            kernel_version   = $serverInfo.KernelVersion
            image_setup      = $setup
            postgres_digest  = $postgresDigest
            postgres_image_id = $postgresImageId
            postgres_image_size = $postgresImageSize
            api_image_id     = $apiImage
            api_image_size   = $apiImageSize
            measurements     = @($composeMeasurements)
        }
    }

    # Burrow runs after Docker setup so a Docker failure cannot lose these samples.
    $burrowMeasurements += Measure-Burrow -Label "cold data (no .burrow data, package cache warm)"
    for ($run = 1; $run -le $WarmRuns; $run++) {
        $burrowMeasurements += Measure-Burrow -Label "warm $run (data retained, package cache warm)"
    }
} finally {
    & $Burrow --project $BurrowProject down 2>&1 | Out-Null
    if ($dockerAvailable) { Stop-Compose -RemoveVolume }
    if ($previousCache) {
        $env:BURROW_CACHE_DIR = $previousCache
    } else {
        Remove-Item Env:BURROW_CACHE_DIR -ErrorAction SilentlyContinue
    }
    Pop-Location
    if (-not $Keep) {
        Remove-Item -Recurse -Force $tempRoot -ErrorAction SilentlyContinue
    }
}

$report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    project      = $Project
    endpoint     = "http://127.0.0.1:$Port$HttpPath"
    machine      = $machine
    burrow       = [pscustomobject]@{
        version      = $burrowVersion
        cache_dir    = $cacheUsed
        measurements = @($burrowMeasurements)
    }
    docker       = $dockerReport
    notes        = @(
        "Startup is wall-clock time from launching the stack until the API returns HTTP 200 with the seeded row.",
        "Burrow cold data starts with no .burrow data but a warm package cache; the Compose cold run starts with an empty volume but cached images.",
        "Image pull/build and Burrow package download are setup, not startup, and are reported separately or excluded.",
        "The API build may reuse Docker's build layer cache; the build time is not necessarily a cold build.",
        "Compose container stats are container memory only; they exclude the Docker Desktop VM and daemon overhead.",
        "The Compose fixture binds the API to 0.0.0.0 inside the container; the native example binds loopback.",
        "Compose runs before Burrow and Docker Desktop's VM stays running, so measurement order is not randomized.",
        "This is a single-machine sample, not a statistical distribution."
    )
}

$report | ConvertTo-Json -Depth 12
if ($JsonPath) {
    $report | ConvertTo-Json -Depth 12 | Set-Content -Path $JsonPath -Encoding utf8
}
