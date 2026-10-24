# 0001 - Workspace layout

- Status: Accepted
- Date: 2026-10-24

## Context

Burrow spans configuration parsing, package management, Windows process
supervision, networking, data management, a CLI, and a TUI. Keeping these in one
crate would mix dependencies and make it hard to test the Windows supervisor in
isolation. The project also needs a consistent Rust edition, MSRV, license, and
set of CI gates from the first commit.

## Decision

1. Use the seven crates from the design: `burrow-config`, `burrow-pkg`,
   `burrow-supervisor`, `burrow-net`, `burrow-data`, `burrow-cli`, and
   `burrow-tui`. The `burrow-supervisor` package also provides a test-only
   `burrow-testbin` binary target: a real child process for supervisor tests
   that is not a product crate and is never shipped.
2. Use edition 2021 for every crate through workspace inheritance.
3. Set the MSRV to 1.85. The `windows` 0.62.2 crate requires Rust 1.82, but the
   current `toml` 1.x release selected by Cargo requires Rust 1.85; this was
   verified against the crate metadata, and the root workspace declares
   `rust-version = "1.85"`.
4. Keep the supervisor Windows-only in M1. No cross-platform process
   abstraction is introduced until there is a concrete requirement for one; CI
   runs on `windows-latest`.
5. Confine product `unsafe` to `crates/burrow-supervisor/src/win.rs`. Win32
   calls and handle ownership live there behind a safe API, and every unsafe
   block needs a safety comment. Test-only observation helpers follow the same
   safety-comment rule.
6. Gate changes with `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`, and
   `cargo test --workspace --all-targets -- --test-threads=1`. CI also builds the
   release binary and runs `cargo audit`.

## Consequences

- Shared dependency versions and package metadata are changed once in the root
  `Cargo.toml`.
- All crates release together, which is appropriate for a single application.
- Non-Windows contributors cannot run the supervisor tests in M1.
- Keeping the product unsafe surface in one module makes the safety review
  tractable; new Win32 functionality must be added there rather than spread
  across the supervisor.
- The CLI can grow command handlers incrementally without changing the public
  library surface of the other crates.
