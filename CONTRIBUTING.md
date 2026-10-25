# Contributing to Burrow

Thanks for helping. Burrow targets Windows first; the M1 supervisor uses Win32
Job Objects directly.

## Prerequisites

- Windows 10 or 11, x86_64.
- Rust stable through rustup. The workspace MSRV is 1.85.
- MSVC build tools and the Windows SDK. In the Visual Studio Build Tools
  installer, select "Desktop development with C++".

## Before you open a pull request

Run the same checks as CI:

```powershell
.\scripts\check.ps1
```

Or run them individually:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets -- --test-threads=1
```

The supervisor integration tests spawn the test-only `burrow-testbin` binary,
which is a binary target inside the `burrow-supervisor` package. Cargo builds it
automatically for `cargo test -p burrow-supervisor` (and for the workspace test
run). The `--test-threads=1` setting is required because tests that exercise
console control events must not run concurrently.

## Fuzzing

The fuzz targets in [`fuzz/`](fuzz/) are not part of the workspace build. They
need a nightly toolchain and `cargo-fuzz`; see [`fuzz/README.md`](fuzz/README.md)
for the target list, the Windows ASan setup, and the corpus layout. Format them
with `cargo fmt --manifest-path fuzz/Cargo.toml`.

## Commit style

Use Conventional Commits, with one logical change per commit:

- `feat: add the config parser`
- `fix: keep the job handle open until the job is empty`
- `docs: record the job lifecycle decision`
- `test: cover forced termination`
- `chore: update CI actions`

## Accuracy rules

- Registry entries must use a SHA-256 taken from the upstream release and must
  record where it came from. If a hash is not verified, mark it `TODO(verify)`
  and make the code fail safe; never ship a placeholder hash.
- Do not add benchmark numbers, performance claims, or compatibility claims
  without a reproducible measurement or a recorded source. State the machine and
  method.
- Do not invent download URLs, versions, or crate APIs.

## Unsafe code

In product code, `unsafe` is confined to each crate's private `win` module:
`crates/burrow-supervisor/src/win.rs` and `crates/burrow-cli/src/win.rs`. Every
unsafe block needs a `// SAFETY:` comment explaining why its preconditions hold.
If you need a Win32 call, add a safe wrapper in that module instead of spreading
raw handles through the rest of the crate. Test-only observation helpers may call
Win32 query functions directly, but they follow the same SAFETY-comment rule.

## Documentation and community

- User documentation lives in [`docs/`](docs/). Start with
  [`getting-started.md`](docs/getting-started.md), then the
  [CLI](docs/cli.md), [configuration](docs/config-reference.md), and
  [registry](docs/registry.md) references.
- Security-sensitive changes should be checked against
  [`docs/security.md`](docs/security.md).
- Release and packaging drafts are in [`packaging/`](packaging/); see
  [`packaging/README.md`](packaging/README.md) before changing the release
  workflow.
- By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
- Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md),
  not in a public issue.

