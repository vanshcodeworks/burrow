# Compose migration example

This directory is a starting point for converting an existing Docker Compose
project. It contains only a `docker-compose.yml` so you can see the converter's
output without any application code.

```powershell
cd examples\compose-migration
burrow init --from .\docker-compose.yml
```

Burrow writes a commented `burrow.toml`:

- `postgres:16` → `use = "postgres@16"` with the seed mount mapped to
  `seed = "./db/init.sql"`
- `redis:7` → `use = "container:redis:7"` with `container_port = 6379`
- `build: .` + `command: npm start` → `run = 'npm start'`
- `depends_on` → `needs`, health check → `health = { tcp = true, interval = "2s" }`
- fixed `ports` are commented out because Burrow allocates stable ports
  automatically

The generated file ends with a list of everything the converter could not carry
over. Review it, create the seed file if you want seeding, then run:

```powershell
burrow doctor
burrow up
```

A container service requires Docker Desktop or Podman and has an ephemeral
filesystem. See the full [migration guide](../../docs/migration-from-docker-compose.md).
