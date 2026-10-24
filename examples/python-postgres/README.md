# Python FastAPI + PostgreSQL

A small FastAPI app that reads from PostgreSQL and serves an HTML page and a
JSON endpoint.

## Run it

```powershell
cd examples\python-postgres
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
burrow up
```

Run `burrow up` from the activated shell: managed services inherit that
environment, so `python` resolves to the virtual environment's interpreter.

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
