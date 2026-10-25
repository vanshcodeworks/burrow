# Screenshot and recording plan

**Status: no screenshots are committed yet.** Capturing them is a human action:
it needs a clean, non-elevated Windows 11 machine and a person to check that no
secret or personal path is visible. The launch checklist has the corresponding
item.

These shots support [`announcement.md`](announcement.md) and the recording
script in [`../demo.md`](../demo.md). Capture real output from the current
commit; do not stage, edit, or fabricate a terminal line.

## Before capturing

- Use a clean Windows 11 x86-64 machine or VM in a normal, non-elevated
  session. An administrator account with a standard token is fine; a true
  standard-user account is not required for these shots and remains unverified
  for support.
- Put the checkout at a neutral path such as `C:\dev\burrow`, not under a
  personal user profile, so screenshots do not expose a real name.
- Use Windows Terminal with PowerShell 7 (or Windows PowerShell 5.1), a dark
  theme, no transparency or acrylic, and a single tab. Set the window to
  **120 columns x 30 rows** and use a legible monospace font (Cascadia Mono 12
  works).
- Close chat, mail, and notification pop-ups. Hide browser bookmarks and tabs
  that reveal personal or work information.
- Build and prepare the example before the first shot:

  ```powershell
  cd C:\dev\burrow
  cargo build --release -p burrow-cli
  cd examples\node-postgres
  npm install
  ```

- Decide whether the shots show a warm or cold package cache, and record it in
  the capture notes. A warm cache is fine for the launch shots; say so rather
  than implying a download happened.
- Never show `.burrow\secrets`, a full `burrow env <service>` output, an
  authorization token, or the contents of a browser profile. The trust prompt
  and `doctor` output are safe.

## Shot list

Save PNGs under `docs/launch/images/` with these names. Create the directory at
capture time; it is intentionally absent from the repository until the images
exist.

| File | What to show | Command(s) | Why it matters |
| --- | --- | --- | --- |
| `01-up-status.png` | The `up --detach` summary followed by `status`, including the `RUNTIME` column, ports, and RAM | `burrow up --detach --yes`, then `burrow status` | Shows a native stack with no Docker daemon or VM, stable loopback ports, and per-service memory |
| `02-doctor.png` | A full `burrow doctor` run with `11 ok, 0 warnings, 0 problems` and the `listeners` line | `burrow doctor` | Shows the read-only checks, the listener check, and that every failure would carry a remedy |
| `03-tui.png` | The live dashboard with both services healthy and a few log lines | `burrow top` | Shows the TUI, live state, and logs without needing a video |
| `04-api.png` | The seeded JSON response and the health response | `curl.exe -s http://api.localhost/api/todos` and `curl.exe -s http://api.localhost/health` | Proves the stack serves the example, not just that processes started |
| `05-snapshots.png` | A snapshot creation, the snapshot list, and the restore hint | `burrow down`, `burrow snapshot before-demo`, `burrow snapshots` | Shows atomic data snapshots and the stopped-daemon requirement |
| `06-trust.png` (optional) | The first-run trust prompt with the command summary | A fresh copy of the example, then `burrow up --detach` without `--yes` | Shows the trust boundary in the user's own words; skip it if the prompt would expose a path you do not want to publish |

If `api.localhost` does not resolve on the capture machine, use the direct
loopback URL from `burrow status` (`http://127.0.0.1:<api-port>/api/todos`) and
say so in the capture notes. Do not edit the screenshot to hide the fallback.

### Capture order

1. Start from a clean example with no `.burrow` directory if you want the trust
   shot. Capture `06-trust.png` first, approve, and continue.
2. Capture `01-up-status.png`, then `02-doctor.png`, `03-tui.png`, and
   `04-api.png` while the stack is running.
3. Stop the stack, then capture `05-snapshots.png`.
4. Run `burrow down` again and confirm no `burrow` process is left.

## Capture method

Use **Win+Shift+S** (Snipping Tool) in window mode, or `Alt+PrintScreen` for the
active window, then save the PNG directly into `docs/launch/images/`. Do not
resize the terminal between shots; the fixed 120x30 grid keeps the set
consistent. Do not annotate or retouch the terminal output. If a shot contains
something that should not be public, recapture it in a clean environment instead
of editing the image.

For a short GIF or video, record the same sequence from [`../demo.md`](../demo.md)
at 1280x720 or larger, keep the terminal at 120x30, and do not speed up or cut
the trust prompt or a real error. A video is optional; the PNG set is the
minimum.

## Verify before committing

- [ ] Every shot comes from the same commit and the same machine; record the
      commit hash, date, Windows version, terminal, font size, and cache state
      in `docs/launch/images/CAPTURE_NOTES.md`.
- [ ] Re-run each command and compare the key lines. PIDs, ports, timings, and
      memory change between runs; do not "fix" them in the image.
- [ ] Search the images for secrets, tokens, personal names, work paths, email
      addresses, browser tabs, and notification pop-ups.
- [ ] Confirm no `.burrow/secrets` or `burrow env` output is visible.
- [ ] Keep text sharp at 100% zoom; optimize PNG size only if the text stays
      readable.
- [ ] Add alt text and a one-line caption for each image where it is used. Do
      not describe a measured number that is not in the image.
- [ ] Update [`announcement.md`](announcement.md) and the README only after the
      images exist and have been reviewed.

## Suggested alt text

- `01-up-status.png`: "Terminal showing Burrow starting the Node and PostgreSQL
  example as native Windows processes, followed by status output with ports and
  memory."
- `02-doctor.png`: "Burrow doctor reporting eleven passing checks, including
  the supervised-listener check."
- `03-tui.png`: "Burrow's terminal dashboard showing the API and database
  healthy with live logs."
- `04-api.png`: "Terminal showing the seeded JSON todo response and an ok health
  response from the example API."
- `05-snapshots.png`: "Terminal showing a snapshot being created, listed, and
  restored for the example stack."
- `06-trust.png`: "Burrow's first-run trust prompt listing the commands it will
  run before starting the stack."

Do not claim these images show a clean-machine pass, Windows 10, ARM64, or a
signed release. They show one verified Windows 11 x86-64 environment.
