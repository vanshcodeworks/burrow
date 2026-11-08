# Packaging and Distribution

Burrow is distributed as one executable, `burrow.exe`. There is no installer,
service, or background update process.

## Current Status

**Released:** v0.1.0 (tagged as `v0.1.0`)

| Installer | Status | Link |
|-----------|--------|------|
| `cargo install` | ✅ Official | `cargo install --path crates\burrow-cli --locked` |
| **GitHub Releases** | ✅ Official | [v0.1.0](https://github.com/vanshcodeworks/burrow/releases/tag/v0.1.0) |
| **winget** | ✅ Published | `winget install vanshcodeworks.Burrow` |
| **Scoop** | ✅ Published | `scoop install burrow` |
| **Chocolatey** | 📝 Ready to submit | `choco install burrow` (pending review) |
| **MSI** | 📝 Source ready | `packaging/msi/burrow.wxs` (build with WiX) |

## Release Artifacts (v0.1.0)

| File | SHA-256 |
|------|---------|
| `burrow-v0.1.0-windows-x86_64.zip` | `2667ad52d3ce8df4f3920de8c6c5a30114a4cd1d9a075e204a59d6ec068ad199` |
| `burrow-v0.1.0-windows-x86_64.cdx.json` (SBOM) | `b56e937e0c3a08dd19c0d81843a84be06e4e3f7d33061e918cfbb62ee3609d59` |

Full checksums: [`SHA256SUMS.txt`](https://github.com/vanshcodeworks/burrow/releases/download/v0.1.0/SHA256SUMS.txt)

## Quick Install

```powershell
# winget (Windows Package Manager)
winget install vanshcodeworks.Burrow

# Scoop
scoop install burrow

# Chocolatey (after package approval)
choco install burrow

# Cargo (from source)
cargo install --path crates\burrow-cli --locked

# Manual download
# Download burrow-v0.1.0-windows-x86_64.zip from GitHub Releases
```

## Package Managers

### winget (Windows Package Manager)
```yaml
# packaging/winget/vanshcodeworks.Burrow.yaml
PackageIdentifier: vanshcodeworks.Burrow
PackageVersion: 0.1.0
InstallerType: zip
NestedInstallerType: portable
InstallerSha256: 2667ad52d3ce8df4f3920de8c6c5a30114a4cd1d9a075e204a59d6ec068ad199
```

### Scoop
```json
// packaging/scoop/burrow.json
{
  "version": "0.1.0",
  "architecture": {
    "64bit": {
      "url": "https://github.com/vanshcodeworks/burrow/releases/download/v0.1.0/burrow-v0.1.0-windows-x86_64.zip",
      "hash": "2667ad52d3ce8df4f3920de8c6c5a30114a4cd1d9a075e204a59d6ec068ad199"
    }
  },
  "bin": "burrow.exe"
}
```

### Chocolatey
```xml
<!-- packaging/chocolatey/burrow.nuspec -->
<package>
  <metadata>
    <id>burrow</id>
    <version>0.1.0</version>
    <title>Burrow</title>
    <authors>vanshcodeworks</authors>
    <owners>vanshcodeworks</owners>
    <projectUrl>https://github.com/vanshcodeworks/burrow</projectUrl>
    <tags>developer-tools local-development postgresql redis mongodb nats caddy</tags>
  </metadata>
  <files>
    <file src="burrow.exe" target="tools\burrow.exe" />
  </files>
</package>
```
Install script: `packaging/chocolatey/tools/chocolateyinstall.ps1`

### MSI Installer (WiX)
```xml
<!-- packaging/msi/burrow.wxs -->
<!-- Build with: candle.exe burrow.wxs && light.exe burrow.wixobj -out burrow.msi -->
```
Build requirements: WiX Toolset v3.11+
```powershell
candle.exe burrow.wxs -dBuildDir="..\..\target\release\" -dProjectDir="."
light.exe burrow.wixobj -out burrow.msi
```

## Release Process

1. **Tag a release:**
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

2. **GitHub Actions workflow** (`.github/workflows/release.yml`) automatically:
   - Runs full test suite
   - Builds `burrow.exe` in release mode
   - Generates `burrow-v0.1.0-windows-x86_64.zip`
   - Generates CycloneDX 1.5 SBOM (`burrow-v0.1.0-windows-x86_64.cdx.json`)
   - Creates `SHA256SUMS.txt`
   - Creates GitHub Release with assets
   - Updates winget/Scoop manifests with real SHA256

3. **Manual steps after release:**
   - Submit Chocolatey package: `choco push burrow.0.1.0.nupkg`
   - Build MSI: `candle.exe burrow.wxs && light.exe burrow.wixobj -out burrow.msi`
   - Verify all installers work

## Local Dry Run

```powershell
# Build and stage locally (no upload)
.\scripts\release-dry-run.ps1

# Output: target/release-dry-run/
#   burrow-v0.1.0-windows-x86_64.zip
#   burrow-v0.1.0-windows-x86_64.cdx.json
#   SHA256SUMS.txt
```

## Signing (Optional)

Add these GitHub repository secrets to enable code signing:
| Secret | Description |
|--------|-------------|
| `WINDOWS_CERTIFICATE_BASE64` | Base64-encoded `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | PFX password |

The workflow uses `signtool` with DigiCert timestamp server.

## Verification

```powershell
# Verify downloaded zip
Get-Content SHA256SUMS.txt | ForEach-Object {
    $expected, $name = $_ -split '\s+', 2
    $actual = (Get-FileHash ".\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
}

# Verify binary
.\burrow.exe --version
# burrow 0.1.0
```

## What Is Not Provided

- ❌ No automatic updater (by design — user controls updates)
- ❌ No machine-wide installation (per-user only)
- ❌ No Windows service (Burrow is a CLI tool)
- ❌ No crates.io publish (workspace `publish = false`)
- ❌ No signed SBOM/provenance (unsigned CycloneDX JSON asset)

## Branding

All installers use **vanshcodeworks** branding:
- **Publisher**: vanshcodeworks
- **Repository**: https://github.com/vanshcodeworks/burrow
- **Icon**: `docs/assets/burrow-icon.ico` (to be added)
- **License**: MIT OR Apache-2.0