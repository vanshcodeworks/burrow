# Needs human (log and continue)

- **cargo-audit local install fails** (build error, os error 3, on this
  machine). CI runs audit on windows-latest, so coverage exists; a local
  run needs a working install or a fixed toolchain. Not blocking.
- **Publishing** (crates.io, winget, Scoop, GitHub releases, signing
  certificates) remains a human action per the hard rules; nothing here
  publishes.
- **Hardware/OS matrix** (Windows 11 ARM64, corporate AV interference,
  OneDrive-synced project dirs, non-admin accounts on locked-down
  machines) cannot be exercised on this machine — needs a human with
  that hardware or CI runners.
