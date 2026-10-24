# Runs the same checks as the CI workflow. Requires Windows and Rust stable.
$ErrorActionPreference = 'Stop'

$cargo = (Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1).Source

if (-not $cargo) {
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path -LiteralPath $fallback -PathType Leaf)) {
        [Console]::Error.WriteLine(
            "error: cargo was not found on PATH or at $fallback. Install Rust stable first.")
        exit 1
    }
    $cargo = $fallback
}

function Invoke-Cargo {
    param([string[]] $Arguments)

    Write-Host "==> cargo $($Arguments -join ' ')"
    & $cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Invoke-Cargo -Arguments @('fmt', '--all', '--check')
Invoke-Cargo -Arguments @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
Invoke-Cargo -Arguments @('test', '--workspace', '--all-targets', '--', '--test-threads=1')

exit 0
