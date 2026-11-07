# Packaging and distribution

Burrow is distributed as one executable, `burrow.exe`. There is no installer,
service, or background update process.

## Current status

This repository does not have a published release yet. The files in this
directory are drafts:

- [`winget/`](winget/) is a Windows Package Manager singleton manifest.
- [`scoop/burrow.json`](scoop/burrow.json) is a Scoop manifest.
- The `REPLACE_WITH_SHA256` values are placeholders. The manifests will not
  install anything until those values are filled from a real release.

The only supported install path today is from a checkout:

```powershell
cargo install --path crates\burrow-cli --locked
```

`cargo install` builds with the release profile and places `burrow.exe` in
`%USERPROFILE%\.cargo\bin`. A zip produced by the local dry-run below also
runs, but it is not a published release.

## Release artifacts

The [`release` workflow](../.github/workflows/release.yml) runs on a `v*` tag or
by manual dispatch. It:

1. Runs the workspace test suite.
2. Checks the dependency policy with `cargo deny check`.
3. Builds `burrow-cli` in release mode with `--locked`.
4. Optionally signs `burrow.exe` when signing secrets are configured.
5. Stages `burrow.exe`, `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`.
6. Produces `burrow-<tag>-windows-x86_64.zip`.
7. Generates a CycloneDX 1.5 SBOM named
   `burrow-<tag>-windows-x86_64.cdx.json` with `cargo-cyclonedx`.
8. Writes `SHA256SUMS.txt` with the SHA-256 of the zip and the SBOM.
9. Uploads the files as a workflow artifact and attaches them to a GitHub
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

Verify every downloaded file against `SHA256SUMS.txt` before extracting:

```powershell
Get-Content .\SHA256SUMS.txt | ForEach-Object {
    $expected, $name = $_ -split '\s+', 2
    $actual = (Get-FileHash ".\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
}
```

The zip is the first line, so the winget and Scoop instructions below can still
use the first hash.

## Local dry-run

[`scripts/release-dry-run.ps1`](../scripts/release-dry-run.ps1) mirrors the
release staging steps without tagging, signing, uploading, or publishing
anything:

```powershell
.\scripts\release-dry-run.ps1
```

It builds `burrow-cli` with `--locked`, stages the same four files as the
release workflow, creates `burrow-<tag>-windows-x86_64.zip`, generates the
CycloneDX 1.5 SBOM with `cargo-cyclonedx`, writes `SHA256SUMS.txt` for both
files, extracts the archive, checks the zip contents and checksums, and runs
the packaged binary's `--version` and `--help`. Output goes to
`target\release-dry-run` by default; use `-OutputDir` to change it or
`-SkipBuild` to reuse `target\release\burrow.exe`.

`cargo-cyclonedx` is required unless you pass `-SkipSbom`; install it with:

```powershell
cargo install cargo-cyclonedx --locked
```

The hash printed by a dry-run is **local only**. A rebuild can change the zip,
and the SBOM's serial number and timestamp change on every generation, so do
not paste these hashes into the winget or Scoop manifests, a release
announcement, or any public claim. Only a hash from the published release's
`SHA256SUMS.txt` is valid there.

## SBOM

Release builds include a CycloneDX 1.5 JSON SBOM generated from `Cargo.lock` by
`cargo-cyclonedx 0.5.9`. It describes `burrow-cli`, its workspace crates, and
the transitive Rust dependencies for `x86_64-pc-windows-msvc`. It is a
dependency inventory, not a vulnerability scan or a signed attestation. The
SBOM is published as a separate asset and hashed in `SHA256SUMS.txt`; it is not
inside the zip.

## Filling the winget draft

1. Publish a release and download `SHA256SUMS.txt`.
2. Confirm the owner and repository in
   [`winget/vanshcodeworks.Burrow.yaml`](winget/vanshcodeworks.Burrow.yaml).
3. Set `PackageVersion` and the `v...` segment of `InstallerUrl` to the release
   tag.
4. Replace `InstallerSha256` with the hash from `SHA256SUMS.txt`.
5. Rename the file if the `PackageIdentifier` changes.
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
2. Replace `REPLACE_WITH_SHA256` in
   [`scoop/burrow.json`](scoop/burrow.json).
3. Set `version` and the `v$version` URL pattern to the release tag.
4. Test the manifest locally with Scoop, then submit it to a bucket if desired.

Scoop installs the zip, exposes `burrow.exe` on `PATH`, and uses the same
`SHA256SUMS.txt` hash. The `checkver` and `autoupdate` blocks point at the
public repository URL.

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
- No signed SBOM or provenance attestation; the CycloneDX file is an unsigned
  JSON asset.
