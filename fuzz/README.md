# Fuzz targets

These targets use [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) and
are **not** part of the workspace build. They require a nightly toolchain and a
locally installed `cargo-fuzz`:

```powershell
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

Run one target for a bounded time. Pass the working corpus first (libFuzzer
writes new units there) and the committed seeds second:

```powershell
cargo +nightly fuzz run config_parse fuzz/corpus/config_parse fuzz/seeds/config_parse -- -max_total_time=60
```

On Windows the AddressSanitizer build needs `clang_rt.asan_dynamic-x86_64.dll`
on `PATH` (it ships with the Visual Studio MSVC toolset). Without it the target
exits with `STATUS_DLL_NOT_FOUND` before the fuzzer starts. The MSVC ASan
runtime is used because `--sanitizer none` cannot link libFuzzer's coverage
symbols on this target.

Targets:

| Target | Entry point | What it must never do |
| --- | --- | --- |
| `config_parse` | `burrow_config::parse` | panic or loop on malformed TOML |
| `config_template` | `burrow_config::render_template` | panic on unmatched braces, Unicode, or unknown variables |
| `registry_parse` | `burrow_pkg::Registry::add_toml` | panic on malformed registry TOML |
| `extract_path` | `burrow_pkg::extract::validate_entry_path` | accept an absolute path or a `..` component |
| `state_parse` | `burrow_cli::state::RuntimeState::parse` | panic on truncated or hostile JSON |

Committed seeds live in `fuzz/seeds/<target>/`. The working corpus is
`fuzz/corpus/<target>/` and is not committed; new findings are written to
`fuzz/artifacts/<target>/`. Reproduce a finding with:

```powershell
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>
```
