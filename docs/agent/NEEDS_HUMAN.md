# Needs human (log and continue)

- **Publishing, signing, and package-manager submission** (crates.io, GitHub
  release, winget, Scoop, signing certificates) remain human actions per the
  hard rules. Nothing here publishes, tags, pushes to `main`, or posts.
- **Clean-machine, VM, or CI execution**: no VM, Windows Sandbox, or clean CI
  runner is available here. The compatibility probe is configured for CI but has
  not run remotely. Windows 10, ARM64, a true standard-user account,
  OneDrive/UNC, FAT/exFAT, corporate antivirus, `LongPathsEnabled=0`, and
  8.3-disabled volumes remain unverified.
- **Podman parity and a Docker Compose head-to-head benchmark**: Docker Desktop
  is installed but Podman is absent, and the Docker daemon was not running for
  the recorded benchmark. A human with both engines can run a like-for-like
  comparison; skip and document any comparator that is unavailable.
- **Review before publishing**: the sourced comparison and any launch drafts
  should be reviewed by a human for accuracy and tone before they are posted.
  The comparison deliberately makes no competitor performance claims.

Local `cargo-audit` 0.22.2 is installed and working; the earlier note about a
failed local install is obsolete.
