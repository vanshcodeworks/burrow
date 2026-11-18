# Example stacks

Every example is a real directory under `examples/`. Copy one into your project
and edit the service names. Burrow downloads and verifies managed packages on
first start; you supply the language runtimes your services need.

| Example | Services | What it demonstrates |
| --- | --- | --- |
| [`examples/node-postgres`](../examples/node-postgres/) | Node.js API + PostgreSQL 16 | The full baseline: seed SQL, health-gated startup, `*.localhost` route, restart policy |
| [`examples/python-postgres`](../examples/python-postgres/) | FastAPI + PostgreSQL 16 | Python virtual environments and managed services inheriting the daemon environment |
| [`examples/node-nats-caddy`](../examples/node-nats-caddy/) | Node.js API + NATS 2 + Caddy 2 | Two built-in packages, generated `NATS_URL`, three routed services, no npm dependencies |
| [`examples/compose-migration`](../examples/compose-migration/) | Compose file only | `burrow init --from docker-compose.yml` conversion, including container fallback |

## Picking a starting point

- **Starting a new Node or Python app:** begin with `node-postgres` or
  `python-postgres`.
- **Adding a queue or a static server:** `node-nats-caddy` shows how built-in
  packages are referenced with `use = "..."`.
- **Already have Compose:** run the converter from `compose-migration`, then
  review the generated notes before `burrow up`.

## Common starting configs

### PostgreSQL only

```toml
[project]
name = "local-db"

[services.db]
use = "postgres@16"
```

### PostgreSQL + a queue + an app

```toml
[project]
name = "local-stack"

[services.db]
use = "postgres@16"
seed = "db/init.sql"

[services.queue]
use = "nats@2"

[services.api]
run = "npm start"
needs = ["db", "queue"]
env = { DATABASE_URL = "{{db.url}}", NATS_URL = "{{queue.url}}" }
health = { http = "/health" }
route = "api.localhost"
```

### Anything without a native Windows build

```toml
[services.cache]
use = "container:redis:7"
container_port = 6379
```

Requires Docker Desktop or Podman. Container filesystems are ephemeral and are
not covered by Burrow snapshots.

See the [configuration reference](config-reference.md) for every field and the
[registry guide](registry.md) for the built-in package versions.
