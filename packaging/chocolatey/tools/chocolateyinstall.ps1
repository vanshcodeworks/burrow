<#
.SYNOPSIS
    Chocolatey install script for Burrow
.DESCRIPTION
    Installs Burrow - Native Windows local development environment manager
    https://github.com/vanshcodeworks/burrow
#>

$ErrorActionPreference = 'Stop'

$packageName = 'burrow'
$url64 = 'https://github.com/vanshcodeworks/burrow/releases/download/v0.1.0/burrow-v0.1.0-windows-x86_64.zip'
$checksum64 = '2667ad52d3ce8df4f3920de8c6c5a30114a4cd1d9a075e204a59d6ec068ad199'
$checksumType64 = 'sha256'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

# Download and extract
$tempDir = Join-Path $env:TEMP "burrow-install-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $tempDir | Out-Null

try {
    $zipPath = Join-Path $tempDir "burrow.zip"
    Write-Host "Downloading Burrow v0.1.0..."
    Invoke-WebRequest -Uri $url64 -OutFile $zipPath -UseBasicParsing

    Write-Host "Verifying checksum..."
    $actualHash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLower()
    if ($actualHash -ne $checksum64.ToLower()) {
        throw "Checksum mismatch! Expected: $checksum64, Got: $actualHash"
    }

    Write-Host "Extracting..."
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

    # Install to Chocolatey bin
    $installDir = Join-Path $env:ChocolateyInstall 'bin'
    if (-not (Test-Path $installDir)) {
        New-Item -ItemType Directory -Path $installDir | Out-Null
    }

    $exePath = Join-Path $tempDir 'burrow.exe'
    if (-not (Test-Path $exePath)) {
        throw "burrow.exe not found in extracted archive"
    }

    Copy-Item -Path $exePath -Destination (Join-Path $installDir 'burrow.exe') -Force

    # Verify installation
    if (Get-Command burrow -ErrorAction SilentlyContinue) {
        Write-Host "Burrow installed successfully!"
        burrow --version
    } else {
        Write-Warning "Burrow installed but not found in PATH. You may need to restart your shell."
    }

    # Install shell completions if available
    try {
        $completionsDir = Join-Path $env:ChocolateyInstall 'completions'
        if (-not (Test-Path $completionsDir)) {
            New-Item -ItemType Directory -Path $completionsDir | Out-Null
        }
        & burrow completions powershell | Out-File (Join-Path $completionsDir 'burrow.ps1') -Encoding UTF8
        Write-Host "PowerShell completions installed to $completionsDir"
    } catch {
        Write-Warning "Could not install completions: $_"
    }

} finally {
    # Cleanup
    if (Test-Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}