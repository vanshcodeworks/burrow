# Store submission checklist

The manifests in this directory are ready to submit. Nothing here submits them.
A human must open the upstream pull requests and push the Chocolatey package.

## Before submitting anything

- [ ] The v0.2.0 GitHub Release still exists and has the same zip and SHA-256
- [ ] The hashes in the manifests match `SHA256SUMS.txt` on that release
- [ ] `burrow --version` from the published zip prints `burrow 0.2.0`

Published hash (verified by downloading the asset):

```
f33a6734a6ca42b09216ad6a6b146bf182b2448a67af13e15e51cee707233167  burrow-v0.2.0-windows-x86_64.zip
```

## winget (microsoft/winget-pkgs)

The required three-file layout is already staged at
[`winget/manifests/v/vanshcodeworks/Burrow/0.2.0/`](winget/manifests/v/vanshcodeworks/Burrow/0.2.0/).

1. Fork `microsoft/winget-pkgs` and clone your fork.
2. Copy the three files into
   `manifests/v/vanshcodeworks/Burrow/0.2.0/` in your fork.
3. Validate locally with `wingetcreate validate` if available:
   ```powershell
   wingetcreate validate .\manifests\v\vanshcodeworks\Burrow\0.2.0
   ```
   If `wingetcreate` is not installed, rely on the CI validation in the PR and
   watch the check results.
4. Open a PR. The title should be `vanshcodeworks.Burrow 0.2.0`.
5. Do not claim `winget install` works until the PR is merged and an actual
   install succeeds on a machine with a clean winget index.

Note: the locale file declares `License: MIT` with a `LicenseUrl` and states the
dual MIT OR Apache-2.0 licensing in the description, because the winget schema
does not accept a free-form SPDX expression in the `License` field.

## Scoop (ScoopInstaller/Main)

The bucket entry is [`scoop/burrow.json`](scoop/burrow.json).

1. Fork `ScoopInstaller/Main` and clone your fork.
2. Copy the file to `bucket/burrow.json`.
3. Validate with `scoop checkver burrow` after adding your bucket locally:
   ```powershell
   scoop bucket add vanshcodeworks <your fork url>
   scoop checkver burrow
   scoop install burrow
   ```
4. Open a PR adding only `bucket/burrow.json`.
5. Do not claim `scoop install burrow` works until the PR is merged.

## Chocolatey

1. Rebuild the package from the current source so it contains the fixed binary:
   ```powershell
   cargo build --release --locked -p burrow-cli
   Copy-Item target\release\burrow.exe packaging\chocolatey\tools\burrow.exe -Force
   choco pack packaging\chocolatey\burrow.nuspec
   ```
2. Add a real `docs/assets/burrow-icon.png` and point `iconUrl` at its raw
   GitHub URL; the community feed requires a valid icon.
3. Test locally:
   ```powershell
   choco install .\burrow.0.2.0.nupkg --source . -y
   choco uninstall burrow -y
   ```
4. Push with your own API key:
   ```powershell
   choco push .\burrow.0.2.0.nupkg --source https://push.chocolatey.org/ --api-key YOUR_KEY
   ```

## MSI

The WiX source is [`msi/burrow.wxs`](msi/burrow.wxs). Build with WiX Toolset v3:

```powershell
candle.exe packaging\msi\burrow.wxs -dBuildDir="target\release\"
light.exe burrow.wixobj -out burrow.msi -ext WixUIExtension
```

Then sign it with an EV certificate and verify installation on a clean machine.
Unsigned MSIs will trigger SmartScreen warnings.

## After publication

Update the status table in [`README.md`](../packaging/README.md) one channel at
a time, and only after an actual install from that channel succeeds. Update the
main README and `docs/getting-started.md` to match.
