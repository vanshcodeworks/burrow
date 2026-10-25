# Agent state (overwrite each cycle)

## Current focus

Tier A2 adversarial suite. A2.6 listener inspection is complete and green.
Done: orphan-kill (A2.1), half-written snapshot debris (A2.2 partial), corrupt
state/ports (A2.3 partial), port collisions (A2.4), secrets-scan (A2.5),
listener inspection (A2.6), junction rejection (A2.7 partial), and
interrupted-download resume (A2.8).

Next: A2.2 interrupted restore/seed/initdb recovery, A2.3 corrupt secrets and
`doctor` live-state preservation, then A2.7 hardlink/ADS coverage, A3 fuzzing,
A4 property tests, A6 `doctor --report`.

## Last cycle summary (2026-10-26, cycle 5)

- **A2.6 shipped**: `burrow_supervisor::listening_sockets(roots)` reads the
  IPv4/IPv6 IP Helper owner-PID TCP tables, walks a Toolhelp process snapshot,
  and attributes child listeners to the supervised root. `doctor` adds a
  `listeners` warning for non-loopback binds without `expose = true`; wildcard
  binds with `expose` and loopback-only services are accepted. ADR 0006 and
  security/CLI/troubleshooting docs updated.
- **Tests**: supervisor `tests/listeners.rs` (loopback + wildcard + IPv6,
  child attribution, listener gone after job close); CLI `tests/listeners.rs`
  (wildcard warns, exposed wildcard accepted, loopback accepted). Test-only
  `burrow-testbin bind <ADDRESS>` mode added.
- Full suite: **164 passed / 0 failed across 19 suites**.
- Release build green in 1m03s.

## Check suite status (last full run, cycle 5)

- `cargo fmt --all --check`: **green**
- `cargo clippy --workspace --all-targets -- -D warnings`: **green**
- `cargo test --workspace --all-targets -- --test-threads=1`: **green,
  164 tests / 19 suites**.
- `cargo audit`: **green** (3 allowed transitive warnings: `paste`
  unmaintained; `lru` unsound x2).
- `cargo build --workspace --release`: **green**, 1m03s.
- `cargo-deny` is still not installed; `cargo audit` is the dependency gate.

## Environment notes

- Windows 11 Home 10.0.26200 on an AMD Ryzen 7 4800H, 15.4 GB RAM.
- Rust 1.94.1 stable via `C:\Users\Asus\.cargo\bin`; workspace MSRV 1.85.
- Docker Desktop 24.0.7 running; Podman absent.
- cargo-audit 0.22.2 installed locally.
- Git Bash quirk: `taskkill //PID` on the command line; normal `/PID` args
  inside Rust Command.

## Known bugs / follow-ups

See `QUEUE.md`. Remaining A2 items, A3/A4/A6, and the untested Podman path are
the open verification gaps.
