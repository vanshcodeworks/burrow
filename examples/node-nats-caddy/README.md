# Node + NATS + Caddy

A three-service stack using two built-in packages and one local `run` command.
No Docker, no WSL, no npm dependencies.

## Requirements

- Node.js on `PATH` (Burrow does not install language runtimes)
- Burrow installed (see the [quick install](../../README.md#quick-install))

## Run

```powershell
cd examples\node-nats-caddy
burrow up
```

On first start Burrow downloads and verifies NATS and Caddy, allocates stable
ports, starts the queue, waits for it to be healthy, then starts the API. Open
`http://api.localhost` (or the direct loopback URL from `burrow status`). The
`/health` endpoint reports the generated `NATS_URL`.

`web.localhost` reaches the built-in Caddy `file-server`, which serves its own
service data directory. Replace the `web` service with
`run = "caddy run --config Caddyfile"` if you need a reverse proxy configuration.

Stop with `burrow down` or `Ctrl+C`.
