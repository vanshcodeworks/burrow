# Configuration reference

A Burrow project is one `burrow.toml` file. Burrow reads it from the current
directory or from `--project <DIR>`. The file is strict TOML: unknown keys are
errors, so a typo fails before anything starts.

A second file, `burrow.local.toml`, is optional and merged over the base file
for per-developer overrides. Both files are validated together.

## Minimal project

```toml
[project]
name = "shop"

[services.web]
run = "npm run dev"

[services.db]
use = "postgres@16"
```

Every service needs exactly one of `run` or `use`. A project needs at least one
service. The full JSON Schema is available with `burrow schema` and is checked in
at [`burrow.schema.json`](burrow.schema.json).

## Top level

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `project` | table | yes | Project metadata. |
| `services` | table of tables | no (but at least one entry is required) | Services keyed by name. |

## `[project]`

| Key | Type | Required | Rules |
| --- | --- | --- | --- |
| `name` | string | yes | 1-63 characters; lowercase ASCII letters, digits, and hyphens only; must not start or end with a hyphen. |

The project name is used in status output, messages, and the
`BURROW_PROJECT` environment variable. It is not used as a container name or a
network identifier.

## `[services.<name>]`

Service names follow the same rule as project names: 1-63 characters, lowercase
ASCII letters, digits, and hyphens, with no leading or trailing hyphen. Names are
used as table keys, directory names under `.burrow/data/`, environment-variable
prefixes, and command arguments.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `use` | string | none | Registry reference such as `postgres@16`; mutually exclusive with `run`. |
| `run` | string | none | Command line to start the service; mutually exclusive with `use`. |
| `cwd` | path | project directory | Working directory, absolute or relative to the project directory. |
| `needs` | array of strings | `[]` | Services that must be healthy before this service starts. |
| `env` | table of strings | `{}` | Extra environment variables layered over Burrow's generated environment. |
| `port` | `"auto"` or integer | `"auto"` | Automatic stable port, or a fixed port from 1 to 65535. |
| `health` | table | none | Health check used before dependents start. See below. |
| `restart` | string | `"on-failure"` | `never`, `on-failure`, or `always`. |
| `memory` | string | none | Job Object memory cap, for example `"512MB"`. |
| `cpu` | string | none | Hard CPU cap from `"1%"` to `"100%"`. |
| `grace` | duration | `"10s"` | Graceful shutdown period before the process tree is terminated. |
| `seed` | path | none | SQL or other seed script for a managed package; relative to the project. |
| `route` | string | none | Reverse-proxy hostname such as `shop.localhost`. |
| `expose` | boolean | `false` | Allow non-loopback clients to reach this service through its route. |
| `idle_stop` | duration | none | Accepted and validated, but inert in this release (phase 2). |
| `watch` | array of strings | `[]` | Accepted, but inert in this release (phase 2). |

### `run` and `use`

`run` is split with the Windows `CommandLineToArgvW` quoting rules. There is no
implicit shell, so shell builtins and `&&` do not work. Use an explicit shell
when you need one:

```toml
run = 'cmd /D /S /C "npm install && npm run dev"'
```

Bare program names are resolved against the service `PATH`, then against the
service working directory. `.exe` files launch directly. `.cmd` and `.bat`
shims (for example `npm.cmd`) are wrapped through `cmd.exe /D /S /C`.
Relative paths containing a directory are resolved against `cwd`.

`use` names a package from the built-in registry. The version can be exact
(`postgres@16.15`), a numeric prefix that selects the newest matching release
(`postgres@16`), or `latest`. See the [registry guide](registry.md). `use` and
`run` cannot be combined. `burrow pkg add` writes this field for you.

### `cwd`

When omitted, the service starts in the project directory. A relative value is
joined to the project directory. The directory must exist before startup.
Templates are expanded in `cwd`.

### `needs` and startup order

Services are started in layers. Every service in a layer starts in parallel;
the next layer starts only after every health check in the previous layer
passes. A service without a `health` block is considered ready as soon as its
process starts.

Unknown names in `needs` and dependency cycles are configuration errors with
the offending path or cycle printed. Selecting services with
`burrow up <service>...` includes their transitive dependencies and nothing
else.

### `env`

Values are strings and may use [template variables](#template-variables).
Burrow's generated variables are applied after `env`, so `env` cannot override
`PORT`, `HOST`, `BURROW_SERVICE`, `BURROW_PROJECT`, `BURROW_PROJECT_DIR`,
`BURROW_DATA_DIR`, or `BURROW_SERVICE_PASSWORD`.

### `port`

`"auto"` asks Burrow to allocate a free port in 40000-49999 and remember it in
`.burrow/ports.json`. The same port is reused on the next run when it is still
free on `127.0.0.1`. A fixed port must be free at startup and is never silently
replaced. All services in a project get distinct ports.

### `health`

Set exactly one of `http`, `tcp`, or `cmd`:

```toml
health = { http = "/health", timeout = "120s", interval = "500ms", retries = 240 }
health = { tcp = true }
health = { cmd = "curl -fsS http://127.0.0.1:{{api.port}}/ready" }
```

| Key | Default | Meaning |
| --- | --- | --- |
| `http` | none | Path for a plain HTTP GET on the service port. Any response below 500 counts as healthy. Must start with `/`. |
| `tcp` | none | When `true`, a successful TCP connection to the service port is healthy. `false` is rejected. |
| `cmd` | none | Command run through `cmd.exe /D /S /C`; exit code 0 is healthy. Runs in the service `cwd` with the service environment. |
| `timeout` | `"60s"` | Total time budget. Must be greater than zero. |
| `interval` | `"500ms"` | Delay between attempts. Must be greater than zero. |
| `retries` | `120` | Maximum attempts. Must be at least 1. The timeout also applies. |

Both limits apply: Burrow stops at whichever of `retries` or `timeout` is
exhausted first. A single attempt is also bounded by the interval (minimum
250 ms). A failure names the last underlying error and points at the service log.

Packages can advertise their own health check. A service-level `health` block
overrides it.

### `restart`

| Value | Behaviour |
| --- | --- |
| `never` | Leave the service stopped after any exit. |
| `on-failure` | Restart after a non-zero exit. |
| `always` | Restart after any exit, including a zero exit. |

Restarts use exponential backoff starting at 500 ms and capped at 30 s. Five
failures within 60 seconds trip the crash-loop breaker: the service becomes
`failed`, stops retrying, and reports the failure in `burrow status`. A run that
stays up for at least 30 seconds resets the consecutive-failure count. A manual
`burrow restart` also resets the breaker.

### `memory` and `cpu`

`memory` sets Job Object committed-memory limits, both job-wide and per process.
Units are binary: `B`, `KB`/`KiB`, `MB`/`MiB`, `GB`/`GiB`, `TB`/`TiB`, case
insensitive, spaces allowed. The value must be greater than zero.

`cpu` is a whole percentage from 1 to 100 with a `%` suffix, for example
`"50%"`. It enables a hard CPU rate cap.

These limits contain accidental resource use. They are not a security boundary.

### `grace`

How long a service gets to exit after `CTRL_BREAK` before Burrow terminates its
Job Object. Any duration accepted by `humantime` works: `"500ms"`, `"10s"`,
`"2m"`, `"1h"`.

### `seed`

`seed` names a file inside the project. It is passed to the package's seed
command (for PostgreSQL, `psql -f`) after the service is healthy for the first
time. Absolute paths and any `..` component are rejected. The file must exist
when the project is resolved.

Seeding runs exactly once per data directory. The SHA-256 of the script is
recorded in `.burrow/data/<service>/.burrow-seeded.json`; editing the script
does not re-run it, because that could corrupt existing data. Use
`burrow reset <service>` to start over. A `seed` on a service that uses `run`
instead of `use` has no effect.

### `route` and `expose`

`route` is a hostname containing at least one dot, using lowercase letters,
digits, hyphens, and dots; it cannot start or end with a dot. Routes must be
unique inside a project. Burrow starts one HTTP/1.1 reverse proxy and matches
the request `Host` header against these values. WebSocket upgrades are tunnelled.

The proxy prefers port 80 so `http://shop.localhost` needs no port suffix. If 80
is unavailable it uses a stable fallback port, and generated route URLs include
that port. `api.localhost` resolves automatically on many Windows systems; if it
does not resolve on yours, use the service's direct `http://127.0.0.1:<port>`
URL or add a hosts entry. See [troubleshooting](troubleshooting.md).

The proxy binds `127.0.0.1` by default. If any routed service sets
`expose = true`, the proxy binds all interfaces, and only routes marked
`expose = true` are reachable from other machines. Burrow prints a warning
before startup. Services themselves are not rebound by Burrow: `expose` only
controls the proxy. Your command must still listen on an interface and port
that make sense for your use case.

### `idle_stop` and `watch`

Both keys are parsed and validated so existing files stay forward compatible.
`idle_stop` must be a non-zero duration. Neither has behaviour in this release;
see the [roadmap](roadmap.md) for phase 2.

## Template variables

Templates are expanded in `run`, `cwd`, `env` values, `health.http`, and
`health.cmd`. Syntax is `{{name}}`; the shorter and longer forms are equivalent:

| Variable | Value |
| --- | --- |
| `{{project.name}}` | `project.name` |
| `{{project.dir}}` | Absolute project directory |
| `{{<service>.host}}` or `{{service.<service>.host}}` | Always `127.0.0.1` |
| `{{<service>.port}}` | Allocated or fixed port |
| `{{<service>.url}}` | Package connection URL when the service uses `use`, otherwise `http://127.0.0.1:<port>` |
| `{{<service>.password}}` | Generated per-service secret |

`<service>` is a service name from the same file. Unknown names, unknown
fields, empty variables, unclosed `{{`, and stray `}}` are errors. Variables are
resolved before the process starts, so `needs` is not required just to reference
another service's port.

Example:

```toml
[services.db]
use = "postgres@16"

[services.api]
run = "uvicorn main:app --port {{api.port}}"
needs = ["db"]
env = { DATABASE_URL = "{{db.url}}" }
```

## Generated environment

Every service receives these variables in addition to `env`:

| Variable | Value |
| --- | --- |
| `PORT` | Allocated or fixed port |
| `HOST` | `127.0.0.1` |
| `BURROW_SERVICE` | Service name |
| `BURROW_PROJECT` | Project name |
| `BURROW_PROJECT_DIR` | Absolute project directory |
| `BURROW_DATA_DIR` | `.burrow/data/<service>` |
| `BURROW_SERVICE_PASSWORD` | Per-service generated secret |
| `BURROW_PACKAGE` | `name@version` for a `use` service |
| `BURROW_PACKAGE_ROOT` | Extracted package root for a `use` service |

Package services also get the package's `bin` directories prepended to `PATH`.
Secrets are generated on first use and stored in `.burrow/secrets`; they are
never printed except by the explicit `burrow env <service>` command.

`burrow env` without a service prints a different, namespaced view:

- `BURROW_PROJECT`, `BURROW_PROJECT_DIR`
- `BURROW_<SERVICE>_HOST`, `_PORT`, `_URL`
- `BURROW_<SERVICE>_ROUTE_URL` when the service has a route and the proxy is
  assigned

`<SERVICE>` is the uppercase service name with every non-alphanumeric character
replaced by `_`.

## `burrow.local.toml`

The local file is deep-merged over `burrow.toml`:

- Tables merge key by key.
- Scalars and arrays replace the base value.
- There is no way to delete a key; set it to a harmless value instead.
- The result must still be a valid configuration, so the project name and
  service names usually stay in the base file.

Typical uses are a different fixed port, a larger memory limit, or a local
`env` override:

```toml
[services.db]
memory = "1GB"

[services.api.env]
RUST_LOG = "debug"
```

`burrow.local.toml` and `.burrow/` are ignored by the repository's
`.gitignore`. Keep them out of version control: they are per-machine files, and
`.burrow/` contains generated credentials.

## Validation summary

Burrow rejects the following before starting anything:

- Missing `project` or `project.name`; invalid project or service names.
- No services; unknown fields at any level.
- A service with both `run` and `use`, or with neither.
- A malformed `use` reference (missing `@`, empty version, whitespace in the
  version).
- An empty `run` or empty health command.
- A health block with zero or multiple checks; `tcp = false`; an HTTP path that
  does not start with `/`; zero timeout or interval; zero retries.
- `memory` with no unit, zero, or an unknown unit; `cpu` outside 1-100 or
  without `%`.
- Invalid `grace` or `idle_stop` durations.
- Absolute seed paths or paths containing `..`; a seed file that does not exist.
- A route without a dot, with invalid characters, or duplicated across services.
- Unknown `needs` targets and dependency cycles.
- Template syntax errors, unknown services, and unknown fields.
- A `cwd` that does not exist.

## Complete example

```toml
[project]
name = "shop"

[services.db]
use = "postgres@16"
seed = "db/seed.sql"
memory = "512MB"

[services.api]
run = "node server.js"
needs = ["db"]
cwd = "api"
route = "api.localhost"
env = { DATABASE_URL = "{{db.url}}", PORT = "{{api.port}}" }
health = { http = "/health", timeout = "60s", interval = "500ms" }
restart = "on-failure"
memory = "256MB"
cpu = "50%"

[services.web]
run = "npm run dev"
needs = ["api"]
route = "web.localhost"
env = { PORT = "{{web.port}}", VITE_API_URL = "{{api.url}}" }
health = { tcp = true, timeout = "120s" }
```

See [`examples/node-postgres`](../examples/node-postgres) and
[`examples/python-postgres`](../examples/python-postgres) for runnable projects.
