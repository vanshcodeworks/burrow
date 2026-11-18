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
$checksum64 = '466813f2cef009a9000b0b4641872621cb2cc680851a6b20c7809575edaef480'
$checksumType64 = 'sha256'
# Draft manifest: url64/checksum64 match the published v0.1.0 GitHub Release asset.

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

} finally {
    # Cleanup
    if (Test-Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}