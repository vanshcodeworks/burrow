# Migrating from Docker Compose

This guide maps a `docker-compose.yml` to a `burrow.toml`. It is for Windows
developers whose compose file mainly starts a database, a queue, and an app on
their own machine — not for production or multi-host deployments.

## What maps to what

| Docker Compose | Burrow |
| --- | --- |
| `services:` | `[services.<name>]` |
| `image: postgres:16` | `use = "postgres@16"` (built-in) or `use = "container:postgres:16"` |
| `build: .` + `command:` | `run = "..."` (Burrow starts commands; it does not build images) |
| `ports: - "5432:5432"` | Automatic stable port from 40000–49999; `{{name.port}}` in your app's env |
| `environment:` | `env = { ... }` plus auto-generated `BURROW_*` variables |
| `depends_on:` | `needs = ["db"]` (waits for **health**, not just process start) |
| `healthcheck:` | `health = { http = "/health" }` or `health = { tcp = true }` |
| `volumes:` for seed SQL | `seed = "db/init.sql"` (runs once after first healthy start) |
| `container_name` / `hostname` | Not needed; use `route = "api.localhost"` for a stable URL |
| ` networks:` | Not needed; every service is on loopback with generated URLs |
| `docker compose up` | `burrow up` |
| `docker compose down` | `burrow down` |
| `docker compose logs -f` | `burrow logs <service>` / `burrow top` |

## A before-and-after example

**docker-compose.yml**

```yaml
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: dev
    ports:
      - "5432:5432"
    volumes:
      - ./db/init.sql:/docker-entrypoint-initdb.d/init.sql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s

  api:
    build: .
    command: npm start
    environment:
      DATABASE_URL: postgres://postgres:dev@db:5432/dev
    ports:
      - "3000:3000"
    depends_on:
      db:
        condition: service_healthy
```

**burrow.toml**

```toml
[project]
name = "myapp"

[services.db]
use = "postgres@16"
seed = "db/init.sql"
health = { tcp = true }

[services.api]
run = "npm start"
needs = ["db"]
env = { DATABASE_URL = "{{db.url}}" }
route = "api.localhost"
```

Notice what you no longer write: fixed host ports, password plumbing, service
hostnames, and `depends_on` conditions. Burrow allocates a stable port per
project, generates a per-service secret, injects `{{db.url}}`, and gates the API
on the database actually being healthy.

## Decide how to run each service

For each service in your compose file, pick one of three options:

1. **Built-in package.** If it is PostgreSQL 16, NATS 2.15, or Caddy 2.11, use
   `use = "postgres@16"` (and so on). Burrow downloads a pinned, SHA-256-verified
   Windows binary and supervises it natively. See the
   [registry guide](registry.md).
2. **A command on your PATH.** For Node, Python, Go, .NET, or any Windows
   executable, use `run = "..."` with `cwd`, `env`, `health`, and `needs` as
   needed. You own installing the language runtime.
3. **Container fallback.** For Redis, MongoDB, MySQL, Elasticsearch, Kafka, or
   anything else with no verified native Windows build, use
   `use = "container:redis:7"`. Requires Docker Desktop or Podman. Container
   ports are loopback-only, and the filesystem is ephemeral — Burrow snapshots
   do not capture container data.

## Things that behave differently

- **Ports are allocated, not fixed.** Do not hardcode `localhost:5432` in your
  app. Use the generated `BURROW_DB_PORT` / `{{db.port}}` or the `{{db.url}}`
  template. Ports are remembered per project in `.burrow/ports.json`, so they
  stay stable across restarts.
- **One seed, once.** `seed = "db/init.sql"` runs after the first successful
  health check and is recorded by hash. Editing the script alone does not re-run
  it; use `burrow reset <service>`.
- **No image builds.** Burrow has no `build:` step. Build your app before
  `burrow up` (for example `npm install`), and let Burrow manage only its runtime.
- **No sandbox.** Services run with your user rights. Burrow is not a security
  boundary — see [Security & Trust](security.md).
- **Data lives in `.burrow/data/`.** Use `burrow snapshot` and `burrow restore`
  instead of named volumes for native services.

## Migrating step by step

1. Install Burrow (see [Getting Started](getting-started.md)).
2. Run `burrow init` in your project, or write `burrow.toml` by hand using the
   mapping above.
3. Run `burrow doctor` and fix anything it reports (it is read-only).
4. Run `burrow up`. On first start Burrow shows a trust summary and downloads
   any managed packages.
5. Open the `route` URL (for example `http://api.localhost`) or the direct URL
   from `burrow status`.
6. Run `burrow down` when finished.

If a service fails, `burrow logs <service>` and `burrow doctor` are the fastest
way to see why. The [FAQ](faq.md) covers common first-run problems, including
`*.localhost` resolution.
