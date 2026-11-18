# Packaging and Distribution

Burrow is distributed as a single executable, `burrow.exe`. There is no installer
service, background updater, or machine-wide agent.

## Current status

**Version:** v0.1.0. The source code and GitHub Release are real; the store
manifests below are drafts awaiting submission/review.

| Channel | Status | How to install |
|---------|--------|----------------|
| Source checkout | ✅ Supported | `cargo install --path crates\burrow-cli --locked` |
| GitHub Release zip | ✅ Supported | Download `burrow-v0.1.0-windows-x86_64.zip` from the [v0.1.0 release](https://github.com/vanshcodeworks/burrow/releases/tag/v0.1.0) and verify its SHA-256 |
| winget | 📝 Draft | `winget install vanshcodeworks.Burrow` — **not published yet** (pending PR to `microsoft/winget-pkgs`) |
| Scoop | 📝 Draft | `scoop install burrow` — **not published yet** (pending PR to a Scoop bucket) |
| Chocolatey | 📝 Draft | `choco install burrow` — **not published yet** (pending review; requires the maintainer's API key) |
| MSI (WiX) | 📝 Source only | `packaging/msi/burrow.wxs` — build locally with WiX Toolset v3; unsigned |

Do not present a draft channel as published. A channel becomes "published" only
after the manifest is accepted upstream and an actual install succeeds.

## Release artifacts

| File | SHA-256 |
|------|---------|
| `burrow-v0.1.0-windows-x86_64.zip` | `466813f2cef009a9000b0b4641872621cb2cc680851a6b20c7809575edaef480` |
| `burrow-v0.1.0-windows-x86_64.cdx.json` (CycloneDX SBOM) | `75d139f1e0833fbed89b26dd989dfcda30b2e8dd5583685ece3d36cb62159432` |

Source of truth: `SHA256SUMS.txt` on the
[v0.1.0 release](https://github.com/vanshcodeworks/burrow/releases/tag/v0.1.0).
Both hashes were independently re-verified by downloading the published assets
and hashing them locally. The checksums in the manifests below are copied from
those published assets. If the release is rebuilt, every manifest checksum must
be regenerated from the new asset — never from a local dry run.

## Releasing

1. Make sure the full local gate is green: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --locked --all`, `cargo audit`, `cargo deny check`.
2. Tag the commit: `git tag v0.1.0 && git push origin v0.1.0`.
3. The `.github/workflows/release.yml` workflow builds the zip, SBOM and
   `SHA256SUMS.txt`, optionally signs the executable, and creates the GitHub
   Release. Watch it to completion and confirm the assets exist.
4. Copy the published asset's SHA-256 into the winget, Scoop and Chocolatey
   manifests, then open the upstream PRs / submit the Chocolatey package.
5. Build the MSI locally (see below).

## Manifests

### winget

`packaging/winget/vanshcodeworks.Burrow.yaml` — singleton manifest, schema 1.6.
Portable zip with `burrow.exe` aliased to `burrow`. Pending PR to
`microsoft/winget-pkgs`.

### Scoop

`packaging/scoop/burrow.json` — 64bit zip with `bin: burrow.exe`, `checkver`
and `autoupdate` pointing at GitHub. Pending PR to a Scoop bucket.

### Chocolatey

- `packaging/chocolatey/burrow.nuspec` — package metadata
- `packaging/chocolatey/tools/chocolateyinstall.ps1` — verifies SHA-256, extracts `burrow.exe` into `$env:ChocolateyInstall\bin`
- `packaging/chocolatey/tools/burrow.exe` — local build input for `choco pack` (not committed)

Build and (with the maintainer's API key) push:

```powershell
Copy-Item target\release\burrow.exe packaging\chocolatey\tools\burrow.exe -Force
choco pack packaging\chocolatey\burrow.nuspec
choco push .\burrow.0.1.0.nupkg --source https://push.chocolatey.org/ --api-key YOUR_KEY
```

Before submitting, add a real `docs/assets/burrow-icon.png` and point `iconUrl`
at its raw GitHub URL — the community feed requires a valid icon.

### MSI (WiX Toolset v3)

```powershell
candle.exe packaging\msi\burrow.wxs -dBuildDir="target\release\"
light.exe burrow.wixobj -out burrow.msi -ext WixUIExtension
```

Installs per-user to `%LOCALAPPDATA%\vanshcodeworks\Burrow`, adds a Start Menu
shortcut, and writes an ARP entry. Unsigned until a code-signing certificate is
configured.

## Signing

Add these repository secrets to enable signing in the release workflow:

| Secret | Description |
|--------|-------------|
| `WINDOWS_CERTIFICATE_BASE64` | Base64-encoded `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | PFX password |

Without them the binaries and MSI are unsigned; SmartScreen may warn on first
run. EV certificates and established publisher reputation require human action.

## Verification

```powershell
# Verify the release asset
Get-Content SHA256SUMS.txt | ForEach-Object {
    $expected, $name = $_ -split '\s+', 2
    $actual = (Get-FileHash ".\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
}

.\burrow.exe --version
# burrow 0.1.0
```

## What is not provided

- No automatic updater (the user controls updates)
- No machine-wide install (per-user only)
- No Windows service (Burrow is a CLI tool)
- No crates.io publish (all workspace crates set `publish = false`)
- No signed SBOM/provenance attestation (the CycloneDX JSON is unsigned)
- No shell completion generation command yet

## Branding

All manifests use **vanshcodeworks** as publisher/author/manufacturer and point
to <https://github.com/vanshcodeworks/burrow>.
