# Node.js API + PostgreSQL

A small API that serves an HTML page and a JSON endpoint from PostgreSQL. It
uses Node's built-in HTTP server, so `pg` is the only dependency.

## Run it

```powershell
cd examples\node-postgres
npm install
burrow up
```

The first start downloads PostgreSQL 16 into Burrow's shared cache and verifies
its SHA-256 (about 356 MB). Later projects reuse the same extracted copy.

Open <http://api.localhost> or <http://127.0.0.1:PORT> after the dashboard shows
the API as healthy. `burrow status` prints the allocated port.

## What to look at

- `burrow.toml` wires `api` to `db` with `needs`, injects `DATABASE_URL` from
  the generated PostgreSQL URL, and routes `api.localhost` through the proxy.
- `db/seed.sql` runs exactly once, after the database passes its TCP health
  check. `burrow reset db` removes the data directory so the next start
  initializes and seeds a fresh database.
- `burrow snapshot before-change` / `burrow restore before-change` work only
  while the project is stopped. Run `burrow down` first.
