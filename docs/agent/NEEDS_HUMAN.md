# Needs human (log and continue)

- **Publishing, signing, and package-manager submission** (crates.io, GitHub
  release, winget, Scoop, signing certificates) remain human actions per the
  hard rules. The repository is public on `main`, but no release, tag, signature,
  or package submission exists. Nothing here tags, signs, uploads a release, or
  posts.
- **Public-repository review**: confirm the README, animated SVG asset, security
  policy, code of conduct contact, and packaging drafts are acceptable for
  public view. The pre-publication history was rewritten to
  `135618994+vanshcodeworks@users.noreply.github.com` before publication, so the
  private Gmail address is no longer in the commit metadata. The README now uses
  the Hallmark Furrow theme with one animated architecture diagram; the
  decorative hero and fake terminal SVG were removed.
- **Release review**: before publishing, review
  `scripts/release-dry-run.ps1`, `deny.toml`, and the release workflow. Confirm
  the published release contains the zip, `SHA256SUMS.txt`, and the CycloneDX
  1.5 SBOM, and that both hashes match. The dry-run's local hash is not a
  release hash, and the SBOM is unsigned.
- **Clean-machine, VM, or CI execution**: no VM, Windows Sandbox, or clean CI
  runner is available here. The compatibility probe is configured for CI but has
  not run remotely. Windows 10, ARM64, a true standard-user account,
  OneDrive/UNC, FAT/exFAT, corporate antivirus, `LongPathsEnabled=0`, and
  8.3-disabled volumes remain unverified.
- **Podman parity**: Podman is absent here, so the optional container fallback
  is verified with Docker only. A human with Podman can run the same fixture and
  compare flag behavior; skip and document any comparator that is unavailable.
- **Screenshot and recording capture**: `docs/launch/screenshots.md` has the
  shot list and capture commands, but no screenshots are committed. Capturing
  them needs a clean, non-elevated Windows 11 machine, a neutral checkout path,
  a 120x30 terminal, and a person to check that no secret or personal path is
  visible. Do not fabricate or retouch terminal output.
- **Review before publishing**: the sourced comparison, benchmark page, launch
  announcement, checklist, demo script, and captured screenshots should be
  reviewed by a human for accuracy and tone before they are posted. Only Docker
  Compose has a measured performance number; the other competitor statements
  remain documentation-based.

Local `cargo-audit` 0.22.2 is installed and working; the earlier note about a
failed local install is obsolete.
