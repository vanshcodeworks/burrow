# Packaging and distribution

Burrow is distributed as one executable, `burrow.exe`. There is no installer,
service, or background update process.

## Current status

This repository does not have a public Git remote or a published release yet.
The files in this directory are drafts:

- [`winget/`](winget/) is a Windows Package Manager singleton manifest.
- [`scoop/burrow.json`](scoop/burrow.json) is a Scoop manifest.
- Every `REPLACE_WITH_OWNER` and `REPLACE_WITH_SHA256` value is a placeholder.
  The manifests will not install anything until those values are filled from a
  real release.

The only install path that works today is from a checkout:

```powershell
cargo install --path crates\burrow-cli --locked
```

`cargo install` builds with the release profile and places `burrow.exe` in
`%USERPROFILE%\.cargo\bin`.

## Release artifacts

The [`release` workflow](../.github/workflows/release.yml) runs on a `v*` tag or
by manual dispatch. It:

1. Runs the workspace test suite.
2. Builds `burrow-cli` in release mode with `--locked`.
3. Optionally signs `burrow.exe` when signing secrets are configured.
4. Stages `burrow.exe`, `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`.
5. Produces `burrow-<tag>-windows-x86_64.zip`.
6. Writes `SHA256SUMS.txt` with the SHA-256 of the zip.
7. Uploads the files as a workflow artifact and attaches them to a GitHub
   release.

No signing certificate is configured for this repository, so current release
builds are **unsigned**. To enable signing, add these repository secrets:

| Secret | Contents |
| --- | --- |
| `WINDOWS_CERTIFICATE_BASE64` | Base64-encoded `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for the `.pfx` |

The workflow imports the certificate into the ephemeral runner's certificate
store and calls `signtool` with a DigiCert timestamp URL. If the secrets are
absent, the workflow prints that it is publishing an unsigned build and
continues. It never invents a signature.

Verify a downloaded zip against `SHA256SUMS.txt` before extracting:

```powershell
$expected = (Get-Content .\SHA256SUMS.txt).Split()[0]
$actual = (Get-FileHash .\burrow-v0.1.0-windows-x86_64.zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "checksum mismatch" }
```

## Filling the winget draft

1. Publish a release and download `SHA256SUMS.txt`.
2. Replace `REPLACE_WITH_OWNER` with the GitHub owner in
   [`winget/REPLACE_WITH_OWNER.Burrow.yaml`](winget/REPLACE_WITH_OWNER.Burrow.yaml).
3. Set `PackageVersion` and the `v...` segment of `InstallerUrl` to the release
   tag.
4. Replace `InstallerSha256` with the hash from `SHA256SUMS.txt`.
5. Rename the file to `<PackageIdentifier>.yaml` if the identifier changes.
6. Validate and test it with the winget tooling:

   ```powershell
   winget validate --manifest .\packaging\winget
   winget install --manifest .\packaging\winget
   ```

The manifest uses `InstallerType: zip` with a nested portable installer. It
installs `burrow.exe` as a portable command named `burrow`; it does not create a
service or write machine-wide state.

## Filling the Scoop draft

1. Publish a release and download `SHA256SUMS.txt`.
2. Replace `REPLACE_WITH_OWNER` and `REPLACE_WITH_SHA256` in
   [`scoop/burrow.json`](scoop/burrow.json).
3. Set `version` and the `v$version` URL pattern to the release tag.
4. Test the manifest locally with Scoop, then submit it to a bucket if desired.

Scoop installs the zip, exposes `burrow.exe` on `PATH`, and uses the same
`SHA256SUMS.txt` hash. The `checkver` and `autoupdate` blocks assume a GitHub
repository at the placeholder URL.

## Versioning

The workspace version is `0.1.0`. Release tags should be `v<version>`, for
example `v0.1.0`. The zip name embeds the tag, so the workflow, winget manifest,
and Scoop manifest must all use the same tag and hash.

## What is not provided

- No MSI or EXE installer.
- No automatic updater.
- No machine-wide installation or Windows service.
- No package published to crates.io; the workspace sets `publish = false`.
- No signed artifact until a certificate is configured.
