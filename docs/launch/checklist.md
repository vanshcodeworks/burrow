# Launch checklist

**Draft for a human.** The agent must not tag, sign, publish, submit to a
package manager, or post. Replace `TODO(human)` only after the step actually
happens. Never invent a release URL, version, or hash.

## 1. Readiness gate (before any announcement)

- [ ] `cargo fmt --all --check` is green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is green.
- [ ] `cargo test --workspace --all-targets -- --test-threads=1` is green.
- [ ] `cargo audit` is green, or every warning is reviewed and documented.
- [ ] `cargo build --workspace --release` is green.
- [ ] A true clean-machine or clean-VM pass succeeds from a fresh checkout.
      This is still blocked here; see `docs/agent/NEEDS_HUMAN.md`.
- [ ] The compatibility probe has run on CI or another clean Windows runner.
- [ ] Windows 10, ARM64, standard-user, OneDrive/UNC, FAT/exFAT, antivirus,
      `LongPathsEnabled=0`, and 8.3-disabled rows are either verified or
      explicitly marked unverified in `docs/compatibility.md`.
- [ ] Podman parity is verified or explicitly marked unverified.
- [ ] The comparison and benchmark pages have been reviewed for accuracy and
      tone, including the Docker Compose caveats in `docs/benchmarks.md`.
- [ ] The version and tag are chosen (`v<workspace version>` unless a human
      decides otherwise).

## 2. Build and verify the release (human)

- [ ] From a clean checkout at the release commit, run the full gate above.
- [ ] Build the release binary:

  ```powershell
  cargo build --release -p burrow-cli --locked
  .\target\release\burrow.exe --version
  ```

- [ ] Stage the same files as `.github/workflows/release.yml`: `burrow.exe`,
      `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`.
- [ ] Create the zip named `burrow-<tag>-windows-x86_64.zip`.
- [ ] Write `SHA256SUMS.txt` with the zip hash.
- [ ] Optionally sign `burrow.exe` with the repository secrets. If no
      certificate is configured, record that the artifact is unsigned. Do not
      describe an unsigned build as signed.
- [ ] Extract the zip into a fresh directory and run the binary there.
- [ ] On a clean, non-elevated account, run `burrow init`, `burrow doctor`, and
      the `examples/node-postgres` quick start. Record the real result.
- [ ] Verify the artifact contains no `.burrow` data, secrets, logs, crash
      notes, or local paths beyond the intended README/licenses.
- [ ] Record the real hash, size, and signer status in the release notes.

## 3. Publish (human only)

- [ ] Tag the release commit `v<version>` and push the tag. Do not move or
      rewrite an existing tag.
- [ ] Run the release workflow (or the manual dispatch) and wait for it to
      succeed.
- [ ] Download the published zip and `SHA256SUMS.txt`; recompute the hash and
      compare it. Do not trust the workflow log alone.
- [ ] Run `burrow --version` from the downloaded artifact.
- [ ] Publish the GitHub release with the verified assets. Keep the generated
      notes or replace them with the reviewed announcement copy.
- [ ] If the release is bad, mark it as a pre-release or remove the asset and
      publish a short advisory. Do not force-push or rewrite history.

## 4. Package managers (human only)

- [ ] Fill the winget manifest with the real owner, version, URL, and SHA-256
      from `SHA256SUMS.txt`; see `packaging/README.md`.
- [ ] Run `winget validate --manifest .\packaging\winget` and
      `winget install --manifest .\packaging\winget` on a clean machine.
- [ ] Fill the Scoop manifest with the real owner, version, URL, and SHA-256;
      see `packaging/README.md`.
- [ ] Test the Scoop manifest locally, then submit it to a bucket if desired.
- [ ] Do not submit either manifest before the release asset exists and the
      hash has been verified.

## 5. Launch assets

- [ ] Capture the screenshots in [`screenshots.md`](screenshots.md) on a clean,
      non-elevated Windows 11 machine. Do not fabricate or edit terminal output.
- [ ] Review [`announcement.md`](announcement.md) and [`../demo.md`](../demo.md)
      for accuracy, tone, and the current benchmark numbers.
- [ ] Replace every `TODO(human)` in the announcement with the real repository
      URL and, if a release exists, the real release URL and hash.
- [ ] Re-read the "What it is not" section. Keep the Docker/sandbox/production
      boundaries.
- [ ] Have a second person review the claims if possible.

## 6. Publish the announcement (human only)

- [ ] Post the reviewed announcement to the chosen channel(s).
- [ ] Link the release, README, comparison, benchmark, and security pages.
- [ ] Do not post competitor performance numbers beyond the measured Docker
      Compose head-to-head.
- [ ] Watch the issue tracker and the discussion for the first day. Answer
      questions with links to the docs, not with new claims.

## 7. Post-launch verification

- [ ] Download the release from the public URL and verify the hash again.
- [ ] Install through each submitted package manager and run `burrow doctor`.
- [ ] Run the quick start from the public README on a clean machine.
- [ ] Record the real results in `docs/agent/LOG.md` and update
      `docs/agent/STATE.md` and `docs/agent/CLAIMS.md`.
- [ ] Open follow-up issues for every unverified row or failed step.

## Hard stops

- The agent never tags, force-pushes, rewrites history, signs, publishes,
  submits a package, or posts.
- Never fill a hash or URL from memory. Copy it from the verified artifact.
- Do not claim Windows 10, ARM64, clean-machine, standard-user, Podman, or
  signed-artifact support until the corresponding check has actually passed.
- Do not remove the benchmark caveats to make the launch copy shorter.
