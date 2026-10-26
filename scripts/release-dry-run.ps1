#Requires -Version 5.1
<#
.SYNOPSIS
Build and verify a local Windows release zip and SBOM without publishing them.

.DESCRIPTION
Mirrors the staging and checksum steps in .github/workflows/release.yml:
build burrow-cli with --locked, stage burrow.exe plus the README and both
licenses, create burrow-<tag>-windows-x86_64.zip, generate a CycloneDX 1.5
SBOM with cargo-cyclonedx, write SHA256SUMS.txt for both files, then verify
the archive contents and run the packaged binary.

This script does not tag, sign, upload, or publish anything.

.PARAMETER Tag
Release tag to embed in the zip name. Defaults to v<workspace version>.

.PARAMETER OutputDir
Directory for the zip, SBOM, checksum file, and staging directory. Defaults to
target\release-dry-run.

.PARAMETER SkipBuild
Reuse target\release\burrow.exe instead of building it. The binary is still
checked against the workspace version.

.PARAMETER SkipSbom
Do not generate a CycloneDX SBOM. SHA256SUMS.txt then lists only the zip.
Use this only when cargo-cyclonedx is unavailable; the default pipeline
requires it.

.PARAMETER KeepStage
Keep the staging directory after a successful run.
#>
[CmdletBinding()]
param(
    [string]$Tag = "",
    [string]$OutputDir = "",
    [switch]$SkipBuild,
    [switch]$SkipSbom,
    [switch]$KeepStage
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Get-Cargo {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargo -and $cargo.Source) { return $cargo.Source }
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) { return $fallback }
    throw "cargo was not found on PATH or in $fallback"
}

function Get-CycloneDx {
    $cyclonedx = Get-Command cargo-cyclonedx -ErrorAction SilentlyContinue
    if ($cyclonedx -and $cyclonedx.Source) { return $cyclonedx.Source }
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo-cyclonedx.exe"
    if (Test-Path $fallback) { return $fallback }
    return $null
}

function Get-GeneratedSboms {
    param(
        [string]$Root,
        [string]$FileName
    )
    $crates = Join-Path $Root "crates"
    if (-not (Test-Path $crates)) { return @() }
    return @(Get-ChildItem -Path $crates -Recurse -Filter $FileName -File -ErrorAction SilentlyContinue)
}

$root = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path (Join-Path $root "Cargo.toml"))) {
    throw "Cargo.toml was not found at $root"
}

$cargoToml = Get-Content (Join-Path $root "Cargo.toml") -Raw
$versionMatch = [regex]::Match(
    $cargoToml,
    '(?m)^\[workspace\.package\]\s*$[\s\S]*?^version\s*=\s*"([^"]+)"'
)
if (-not $versionMatch.Success) {
    throw "Could not read [workspace.package] version from Cargo.toml"
}
$version = $versionMatch.Groups[1].Value

if ([string]::IsNullOrWhiteSpace($Tag)) {
    $Tag = "v$version"
}
if ($Tag -notmatch '^v[0-9A-Za-z][0-9A-Za-z._-]*$') {
    throw "Tag '$Tag' is not a safe release tag (expected a leading 'v' and letters, digits, '.', '_' or '-')"
}

$name = "burrow-$Tag-windows-x86_64"
$zipName = "$name.zip"
$sbomName = "$name.cdx.json"
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $root "target\release-dry-run"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $root $OutputDir
}
$OutputDir = [System.IO.Path]::GetFullPath($OutputDir)
if ($OutputDir -eq [System.IO.Path]::GetPathRoot($OutputDir)) {
    throw "OutputDir must not be a drive root"
}
if ($OutputDir -eq $root) {
    throw "OutputDir must not be the repository root"
}

$cargo = Get-Cargo
$cyclonedx = $null
if (-not $SkipSbom) {
    $cyclonedx = Get-CycloneDx
    if (-not $cyclonedx) {
        throw "cargo-cyclonedx was not found on PATH or in $env:USERPROFILE\.cargo\bin. Install it with 'cargo install cargo-cyclonedx --locked', or pass -SkipSbom."
    }
}
$binary = Join-Path $root "target\release\burrow.exe"

if (-not $SkipBuild) {
    Write-Host "Building burrow-cli $version in release mode..."
    Push-Location $root
    try {
        & $cargo build --release -p burrow-cli --locked
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "Skipping the build; reusing $binary"
}

if (-not (Test-Path $binary)) {
    throw "Release binary was not found at $binary"
}

$binaryVersion = (& $binary --version 2>&1 | Out-String).Trim()
if ($binaryVersion -ne "burrow $version") {
    throw "Binary reports '$binaryVersion', expected 'burrow $version'"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$stage = Join-Path $OutputDir $name
$zip = Join-Path $OutputDir $zipName
$sbom = Join-Path $OutputDir $sbomName
$sums = Join-Path $OutputDir "SHA256SUMS.txt"
$verify = Join-Path $OutputDir ("verify-" + $PID)
$sbomTempBase = "burrow-sbom-$PID"

foreach ($path in @($stage, $zip, $sbom, $sums, $verify)) {
    if (Test-Path $path) {
        Remove-Item $path -Recurse -Force
    }
}
Get-GeneratedSboms -Root $root -FileName "$sbomTempBase.json" |
    ForEach-Object { Remove-Item $_.FullName -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

$files = @("burrow.exe", "README.md", "LICENSE-MIT", "LICENSE-APACHE")
foreach ($file in $files) {
    $source = if ($file -eq "burrow.exe") { $binary } else { Join-Path $root $file }
    if (-not (Test-Path $source)) {
        throw "Release input is missing: $source"
    }
    Copy-Item $source (Join-Path $stage $file)
}

$success = $false
try {
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal
    if (-not (Test-Path $zip)) {
        throw "Compress-Archive did not create $zip"
    }

    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()

    $sbomHash = $null
    $componentCount = $null
    if (-not $SkipSbom) {
        Write-Host "Generating a CycloneDX 1.5 SBOM for burrow-cli..."
        Push-Location $root
        try {
            & $cyclonedx cyclonedx `
                --manifest-path "crates\burrow-cli\Cargo.toml" `
                --format json `
                --spec-version 1.5 `
                --target x86_64-pc-windows-msvc `
                --override-filename $sbomTempBase
            if ($LASTEXITCODE -ne 0) {
                throw "cargo-cyclonedx failed with exit code $LASTEXITCODE"
            }
        } finally {
            Pop-Location
        }

        $generated = @(Get-GeneratedSboms -Root $root -FileName "$sbomTempBase.json")
        $cliSbom = $generated |
            Where-Object { $_.DirectoryName -eq (Join-Path $root "crates\burrow-cli") } |
            Select-Object -First 1
        if (-not $cliSbom) {
            throw "cargo-cyclonedx did not create an SBOM for burrow-cli"
        }

        $bom = Get-Content $cliSbom.FullName -Raw | ConvertFrom-Json
        if ($bom.bomFormat -ne "CycloneDX") {
            throw "SBOM bomFormat is '$($bom.bomFormat)', expected 'CycloneDX'"
        }
        if ($bom.specVersion -ne "1.5") {
            throw "SBOM specVersion is '$($bom.specVersion)', expected '1.5'"
        }
        if ($bom.metadata.component.name -ne "burrow-cli") {
            throw "SBOM root component is '$($bom.metadata.component.name)', expected 'burrow-cli'"
        }
        $componentCount = @($bom.components).Count
        if ($componentCount -lt 1) {
            throw "SBOM contains no components"
        }

        Move-Item $cliSbom.FullName $sbom -Force
        $generated |
            Where-Object { $_.FullName -ne $cliSbom.FullName } |
            ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
        $sbomHash = (Get-FileHash $sbom -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    $sumLines = @("$hash  $zipName")
    if (-not $SkipSbom) {
        $sumLines += "$sbomHash  $sbomName"
    }
    $sumLines | Set-Content $sums -Encoding ascii

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $entries = @($archive.Entries | ForEach-Object { $_.FullName } | Sort-Object)
    } finally {
        $archive.Dispose()
    }
    $expected = @($files | Sort-Object)
    $missing = @($expected | Where-Object { $_ -notin $entries })
    $extra = @($entries | Where-Object { $_ -notin $expected })
    if ($missing.Count -gt 0 -or $extra.Count -gt 0) {
        throw "Zip contents mismatch. Missing: $($missing -join ', '); extra: $($extra -join ', ')"
    }

    New-Item -ItemType Directory -Force -Path $verify | Out-Null
    Expand-Archive -Path $zip -DestinationPath $verify
    $packaged = Join-Path $verify "burrow.exe"
    if (-not (Test-Path $packaged)) {
        throw "Packaged burrow.exe was not found after extraction"
    }
    $packagedVersion = (& $packaged --version 2>&1 | Out-String).Trim()
    if ($packagedVersion -ne "burrow $version") {
        throw "Packaged binary reports '$packagedVersion', expected 'burrow $version'"
    }
    $help = (& $packaged --help 2>&1 | Out-String)
    if ($help -notmatch 'no Docker daemon or VM by default') {
        throw "Packaged help text is missing the current product summary"
    }
    if ($help -match 'fraction of the resources') {
        throw "Packaged help text still contains the unsupported resource claim"
    }

    $sumEntries = @{}
    foreach ($line in Get-Content $sums) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $parts = $line -split '\s+', 2
        if ($parts.Count -ne 2) {
            throw "Malformed SHA256SUMS.txt line: $line"
        }
        $sumEntries[$parts[1]] = $parts[0]
    }
    if ($sumEntries[$zipName] -ne $hash) {
        throw "SHA256SUMS.txt does not match the zip hash"
    }
    if (-not $SkipSbom -and $sumEntries[$sbomName] -ne $sbomHash) {
        throw "SHA256SUMS.txt does not match the SBOM hash"
    }
    if ($sumEntries.Count -ne $sumLines.Count) {
        throw "SHA256SUMS.txt has an unexpected number of entries"
    }

    $zipInfo = Get-Item $zip
    $success = $true
    Write-Host ""
    Write-Host "Release dry-run complete. Nothing was tagged, signed, uploaded, or published."
    Write-Host "  version:  $version"
    Write-Host "  tag:      $Tag"
    Write-Host "  zip:      $zip"
    Write-Host "  bytes:    $($zipInfo.Length)"
    Write-Host "  sha256:   $hash"
    Write-Host "  contents: $($expected -join ', ')"
    Write-Host "  binary:   $packagedVersion"
    if (-not $SkipSbom) {
        Write-Host "  sbom:     $sbom"
        Write-Host "  sbom sha: $sbomHash"
        Write-Host "  sbom cmps: $componentCount"
    } else {
        Write-Host "  sbom:     skipped (-SkipSbom)"
    }
} finally {
    Get-GeneratedSboms -Root $root -FileName "$sbomTempBase.json" |
        ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
    if (Test-Path $verify) {
        Remove-Item $verify -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($success -and -not $KeepStage) {
        Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}
