<#
.SYNOPSIS
    Install Burrow from its published GitHub Release.

.DESCRIPTION
    Downloads the official Windows x86-64 release zip, verifies its SHA-256
    against the SHA256SUMS.txt published in the same release, installs
    burrow.exe to a per-user directory, and (unless -NoPath is set) adds that
    directory to your user PATH. No administrator rights are required.

.EXAMPLE
    # One-line install (review remote scripts before piping to iex):
    iwr https://raw.githubusercontent.com/vanshcodeworks/burrow/main/scripts/install.ps1 | iex

.EXAMPLE
    # Safer: download, review, then run
    iwr https://raw.githubusercontent.com/vanshcodeworks/burrow/main/scripts/install.ps1 -OutFile install.ps1
    notepad .\install.ps1
    .\install.ps1

.EXAMPLE
    .\scripts\install.ps1 -Version 0.2.0 -NoPath -InstallDir C:\tools\burrow
#>
[CmdletBinding()]
param(
    [string]$Version = '0.2.0',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'vanshcodeworks\Burrow\bin'),
    [switch]$NoPath
)

$ErrorActionPreference = 'Stop'

$repo = 'vanshcodeworks/burrow'
$tag = "v$Version"
$asset = "burrow-$tag-windows-x86_64.zip"
$baseUrl = "https://github.com/$repo/releases/download/$tag"
$zipUrl = "$baseUrl/$asset"
$sumsUrl = "$baseUrl/SHA256SUMS.txt"

function Get-Sha256([string]$Path) {
    (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$temp = Join-Path $env:TEMP "burrow-install-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force -Path $temp | Out-Null

try {
    Write-Host "Burrow $Version - downloading $asset ..."
    $zip = Join-Path $temp $asset
    Invoke-WebRequest -Uri $zipUrl -OutFile $zip -UseBasicParsing

    Write-Host 'Verifying SHA-256 against SHA256SUMS.txt from the same release ...'
    $sumsResponse = Invoke-WebRequest -Uri $sumsUrl -UseBasicParsing
    # GitHub serves release assets as application/octet-stream, so .Content is
    # a byte array rather than a string.
    if ($sumsResponse.Content -is [byte[]]) {
        $sumsText = [System.Text.Encoding]::UTF8.GetString($sumsResponse.Content)
    }
    else {
        $sumsText = [string]$sumsResponse.Content
    }
    $line = $sumsText -split '\r?\n' |
        Where-Object { $_ -match "\s+$([regex]::Escape($asset))$" } |
        Select-Object -First 1
    if (-not $line) {
        throw "SHA256SUMS.txt has no entry for $asset. Install aborted."
    }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    if ($expected -notmatch '^[0-9a-f]{64}$') {
        throw "Invalid SHA-256 value in SHA256SUMS.txt: $expected. Install aborted."
    }

    $actual = Get-Sha256 $zip
    if ($actual -ne $expected) {
        throw "Checksum mismatch. Expected $expected, got $actual. Install aborted."
    }
    Write-Host "SHA-256 verified: $actual"

    Write-Host 'Extracting ...'
    $extract = Join-Path $temp 'extract'
    Expand-Archive -Path $zip -DestinationPath $extract -Force
    $exe = Get-ChildItem -Path $extract -Recurse -Filter 'burrow.exe' -File |
        Select-Object -First 1
    if (-not $exe) {
        throw "burrow.exe was not found in $asset. Install aborted."
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Path $exe.FullName -Destination (Join-Path $InstallDir 'burrow.exe') -Force

    $alreadyOnPath = $false
    foreach ($entry in ($env:PATH -split ';')) {
        if ($entry -ieq $InstallDir) { $alreadyOnPath = $true; break }
    }

    if (-not $NoPath -and -not $alreadyOnPath) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ([string]::IsNullOrWhiteSpace($userPath)) {
            $newPath = $InstallDir
        }
        else {
            $newPath = $userPath.TrimEnd(';') + ';' + $InstallDir
        }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        $env:PATH = $env:PATH.TrimEnd(';') + ';' + $InstallDir
        Write-Host "Added $InstallDir to your user PATH."
    }

    $installed = Join-Path $InstallDir 'burrow.exe'
    Write-Host ''
    Write-Host "Burrow installed to $installed"
    & $installed --version

    $onPathNow = $false
    foreach ($entry in ($env:PATH -split ';')) {
        if ($entry -ieq $InstallDir) { $onPathNow = $true; break }
    }
    if (-not $onPathNow) {
        Write-Host "Restart your terminal, then run: burrow --version"
    }
}
finally {
    if (Test-Path $temp) {
        Remove-Item -Path $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
