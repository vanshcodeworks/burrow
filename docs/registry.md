# Registry authoring guide

Burrow installs managed services from a registry of TOML entries. Every entry
pins one exact upstream Windows x86-64 artifact by HTTPS URL, byte size, and
SHA-256. Burrow refuses to run an artifact whose hash does not match.

## How the registry is loaded

The built-in registry is compiled into `burrow.exe`. Each entry lives at
`registry/<name>.toml` in this repository and is embedded with `include_str!`
from [`crates/burrow-pkg/src/builtin.rs`](../crates/burrow-pkg/src/builtin.rs).
The shipped entries are `postgres`, `nats` (message queue), and `caddy`
(static file server); `registry/nats.toml` is a compact second reference next
to the PostgreSQL entry.
There is no network registry, no auto-update, and no user registry directory in
this release. `Registry::load_directory` exists as a library API for tests and
future use, but the CLI does not call it.

Adding a package therefore means changing the repository and rebuilding:

1. Add `registry/<name>.toml`.
2. Add it to `ENTRIES` in `crates/burrow-pkg/src/builtin.rs`.
3. Run the checks.
4. Update the [getting started guide](getting-started.md) only if the package is
   part of the curated examples.

This keeps one binary self-contained and makes every runnable artifact
reviewable in the same change as its hash.

## File format

```toml
schema = 1
name = "example"
description = "One-line description"
license = "MIT"
upstream = "https://example.com/project"
source = "https://example.com/project/downloads"

[versions."1.2.3"]
run = { program = "server", args = ["--port", "{{port}}"] }
url_template = "http://{{host}}:{{port}}"
health = { tcp = true }
default_port = 1234

[versions."1.2.3".platforms.windows-x86_64]
url = "https://example.com/project-1.2.3-windows-x64.zip"
sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
size = 12345678
archive = "zip"

[versions."1.2.3".platforms.windows-x86_64.binaries]
server = "bin/server.exe"
```

Unknown fields are rejected at every level, so a typo fails loudly.

## Top level

| Key | Type | Required | Rules |
| --- | --- | --- | --- |
| `schema` | integer | yes | Must be `1`. |
| `name` | string | yes | Package name: one or more lowercase ASCII letters, digits, `-`, or `_`. |
| `description` | string | yes | Must not be empty. |
| `license` | string | yes | Must not be empty. Use the upstream license identifier, for example `PostgreSQL` or `MIT`. |
| `upstream` | string | yes | Must start with `https://`. The project's page. |
| `source` | string | yes | Must start with `https://`. The page where the Windows build is published. |
| `unsupported_native` | string | no | A reason the package cannot run natively. When set, resolution fails with that reason. |
| `versions` | table | unless `unsupported_native` | One table per version. |

`unsupported_native` is for packages that only ship Linux containers or have no
trustworthy Windows build. Do not use it to hide a broken entry. Set it and
explain why, so users get an actionable error instead of a failed download.

## Versions

Version keys are numeric components separated by dots: `16`, `16.15`, `1.2.3`.
They are compared numerically, not lexicographically, so `1.10` is newer than
`1.2`. A version table accepts:

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `platforms` | table | yes for a runnable version | Artifacts keyed by platform. Only `windows-x86_64` is accepted. |
| `run` | command | required to start the service | Foreground command. |
| `stop` | command | no | Graceful stop command, run before the process tree is killed. |
| `init` | array of commands | no | Initialization steps, in order. |
| `seed` | command | no | Runs once after the first successful health check. |
| `url_template` | string | no | Value for `{{service.url}}`. Without it the URL is `http://127.0.0.1:<port>`. |
| `health` | table | no | `tcp = true` and/or `http = "/path"`. If both are set, HTTP wins. |
| `default_port` | integer | no | Metadata only. Burrow still allocates a port unless the project sets `port`. |

`run` is required when a project resolves `use = "<name>@<version>"`; parsing
alone does not reject a version without it.

### Commands

A command is a table with a binary role and an argument list:

```toml
init = [
  { program = "init", args = ["--data", "{{data_dir}}"], when = "data-empty" },
]
run = { program = "server", args = ["--host", "{{host}}", "--port", "{{port}}"] }
stop = { program = "stop", args = ["{{data_dir}}"] }
seed = { program = "seed", args = ["--file", "{{seed_file}}"] }
```

`program` names a role in the platform's `binaries` map. It is not a bare
executable name and it is not searched on `PATH`. Commands are executed directly
without a shell, so arguments are passed exactly as written.

`when` is optional on initialization steps. The only accepted value is
`data-empty` (the default): the step runs when the service data directory is
missing or empty. After all steps succeed, Burrow writes
`.burrow/data/<service>/.burrow-initialized.json`. A later start skips
initialization until `burrow reset <service>` removes the directory.

The seed command runs once, after the service's first successful health check,
and only when the project sets `seed`. Burrow records the script hash in
`.burrow/data/<service>/.burrow-seeded.json`. Editing the script does not re-run
it; `burrow reset <service>` does. A seed command without a configured seed file
is not run.

### Placeholders

Placeholders are replaced without invoking a shell. Unknown placeholders and
stray braces are errors.

| Placeholder | Value |
| --- | --- |
| `{{host}}` | Always `127.0.0.1`. |
| `{{port}}` | The service's allocated or fixed port. |
| `{{data_dir}}` | Absolute `.burrow/data/<service>` path. |
| `{{project_dir}}` | Absolute project directory. |
| `{{password}}` | The generated per-service secret. |
| `{{seed_file}}` | Absolute path of the project's seed file; only in the seed command. |

Servers must still bind to loopback themselves. For example, PostgreSQL's entry
passes `-c listen_addresses=127.0.0.1` in `run`. Burrow's loopback guarantee
covers its own proxy and control listeners, not an arbitrary upstream default.

### Health

```toml
health = { tcp = true }
health = { http = "/health" }
```

A package health block is used only when the service does not define its own
`health`. TCP checks accept any successful connection. HTTP checks accept any
response below 500. Package health uses the default timeout and retry settings
from the [configuration reference](config-reference.md#health).

## Artifacts and binaries

```toml
[versions."1.2.3".platforms.windows-x86_64]
url = "https://example.com/project-1.2.3-windows-x64.zip"
sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
size = 12345678
archive = "zip"

[versions."1.2.3".platforms.windows-x86_64.binaries]
server = "bin/server.exe"
init = "bin/init.exe"
```

| Key | Rules |
| --- | --- |
| `url` | Must start with `https://`. |
| `sha256` | Exactly 64 hexadecimal characters, case-insensitive; stored lowercase. |
| `size` | Exact archive size in bytes; must be greater than zero. |
| `archive` | `zip`, `tar.gz`/`tgz`, `tar`, or `raw`. For `raw`, the artifact is copied into the extracted tree as a single file named `bin`; set the binary role to `bin` (or a path that points at it). |
| `binaries` | At least one entry. Keys are roles used by commands; values are relative paths inside the extracted root. |

Binary paths use forward slashes. Absolute paths, drive letters, `..`, NUL,
trailing dots or spaces, and reserved Windows device names (`CON`, `NUL`,
`COM1`, ...) are rejected. Every `program` referenced by `init`, `run`, `stop`,
or `seed` must have a matching role in `binaries`.

The extractor is hardened independently of the registry:

- zip-slip and tar path traversal entries are rejected;
- symlinks, hard links, and special entries are rejected;
- total uncompressed size is capped at 8 GiB, one file at 4 GiB, entry count at
  250,000, path depth at 64 components;
- files are created with `create_new`, so an archive cannot follow a link
  planted by an earlier entry;
- extraction happens in a temporary directory, then the tree is renamed into
  the content-addressed cache.

## Choosing a version

A project reference can be:

| Reference | Selection |
| --- | --- |
| `postgres@16.15` | Exact version key. |
| `postgres@16` | Highest version whose numeric components start with `16`; `16.15` beats `16.9`. |
| `postgres@latest` | Highest version in the entry. |
| `postgres` | Same as `latest` in the resolver, but `burrow.toml` requires an `@` version. |

`burrow pkg add` writes the canonical resolved version, not the prefix. That
keeps a project reproducible when a new patch release is added to the registry.

## Verifying a new entry

Never copy a hash from a download page or another project. Download the exact
artifact yourself and compute both values:

```powershell
Invoke-WebRequest -Uri $url -OutFile .\package.zip
Get-FileHash -Algorithm SHA256 .\package.zip
(Get-Item .\package.zip).Length
```

Then:

1. Confirm the URL is served over HTTPS and is the upstream vendor's official
   download, not a mirror or a search result.
2. Confirm the license allows redistribution of the binary in this form, or
   that Burrow only downloads it from the vendor. Burrow does not redistribute
   package binaries.
3. Check that the archive's layout matches the `binaries` paths. Extracting it
   once by hand is the fastest way to catch a wrong path.
4. Add the file and register it in `builtin.rs`.
5. Extend the built-in registry test in
   [`crates/burrow-pkg/src/builtin.rs`](../crates/burrow-pkg/src/builtin.rs) with
   the exact size, hash, and a binary-role assertion.
6. Run at least one real end-to-end start:

```powershell
cargo test -p burrow-pkg
cargo run -p burrow-cli -- pkg add example@1.2.3
cargo run -p burrow-cli -- doctor
cargo run -p burrow-cli -- up
```

7. Run `burrow pkg verify` against the populated cache.

## Cache and references

The shared cache is at `%LOCALAPPDATA%\burrow\cache` unless `BURROW_CACHE_DIR`
is set:

```text
archives/<sha256>.<ext>          verified download
archives/<sha256>.<ext>.part     interrupted download, resumed with a Range request
packages/<sha256>/               extracted tree, shared by every project
packages/<sha256>/.burrow-package.json
refs/<hash-of-project-dir>.json  packages a project uses
locks/<sha256>.lock              cross-process download lock
tmp/                             half-extracted archives, removed on failure
```

A package is used only after its SHA-256 and size match the registry. A cached
archive is re-hashed before reuse; a mismatch discards it and downloads again.
Extracted trees are renamed into place atomically. Concurrent Burrow processes
coordinate through a lock file.

`burrow pkg gc` keeps packages referenced by a known project directory, even
when that project is stopped. It removes packages and archives that no reference
file mentions, and it drops reference files for directories that no longer
exist. `burrow pkg verify` re-hashes archives and validates extracted package
markers, but it does not hash every extracted file.

## Review checklist

- [ ] Official HTTPS source, stable URL, correct license and attribution.
- [ ] Exact size and SHA-256 computed from the downloaded artifact.
- [ ] `run` binds to loopback where the server supports it.
- [ ] `init`, `seed`, and `stop` are idempotent enough for the data markers.
- [ ] Every command's binary role exists on `windows-x86_64`.
- [ ] Health check proves the service is actually ready, not merely listening.
- [ ] Version prefix selection does the expected thing.
- [ ] New entry is registered in `builtin.rs` and has a registry test.
- [ ] A real project starts, initializes, seeds, stops, restarts, snapshots, and
      restores.
- [ ] Documentation states any remaining limitation.
