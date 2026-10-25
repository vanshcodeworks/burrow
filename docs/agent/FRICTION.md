# Clean-machine friction log

This file records every place the documented first-run path failed or misled a
new user. A clean-machine pass is not complete until a full run has zero
friction. A VM, Windows Sandbox, or a CI runner is the preferred environment;
when one is unavailable, the entry says so.

## 2026-10-26 - cycle 6 (README-only pass, not a clean machine)

**Environment**: the development machine (Windows 11 10.0.26200, normal user
token). The release binary `target\release\burrow.exe` was used instead of a
fresh `cargo install`, and the shared package cache was warm. This is therefore
a documentation and first-run-flow pass, not a clean-machine verification. It
does not replace the Phase 1 VM/Sandbox run.

**Path followed**: README Quick start, Node.js + PostgreSQL example. Removed
`examples\node-postgres\.burrow`, ran `npm install`, then `burrow up --detach
--yes`, opened the routed URL, ran `burrow doctor --json`, and stopped with
`burrow down`.

### Friction found and fixed

1. **README limitations contradicted the implemented product.** The README said
   there was no file watching/restart-on-change and no container fallback, and
   that the registry contained PostgreSQL only. All three were false after
   Phase 2. Fixed: the limitations now name all three registry packages and
   describe opt-in `watch` and the container fallback accurately.
2. **FAQ said the registry contained PostgreSQL only, then later listed NATS and
   Caddy.** Fixed: the "install runtimes" answer now names PostgreSQL 16.15,
   NATS 2.15.1, and Caddy 2.11.7.
3. **FAQ said Burrow does not watch files.** Fixed: it now documents opt-in
   `watch = [...]` and links the configuration reference.
4. **README implied every later start takes about two seconds.** A start with a
   warm package cache but a fresh data directory re-runs `initdb` and seeding
   and took 10.28 s here. Fixed: the sentence now says "reuse both the shared
   cache and the initialized cluster".

### Friction observed but not fixed

- A truly cold first start (empty package cache) still requires a 373 MB
  download. This is inherent to the managed PostgreSQL package and is disclosed
  in the README. Time-to-first-success on a cold cache remains a Phase 1
  measurement.
- `burrow up --detach` does not print the proxy port unless `status` is run.
  The routed URL is printed, so this did not block the pass.

### Verified during the pass

- `burrow up --detach --yes` on a fresh `.burrow`: both services healthy in
  10.28 s (warm cache, fresh cluster).
- `http://api.localhost/api/todos` returned `200` and the seeded row.
- `burrow status --json` and `burrow doctor --json` were valid JSON; doctor
  reported 11 ok / 0 warnings / 0 failures, including the new `listeners`
  check ("2 supervised listeners; none accept non-loopback traffic without
  expose").
- `BURROW_NO_BROWSER=1 burrow open` printed `http://api.localhost`.
- `burrow down` took 0.61 s, both service PIDs were gone, and
  `.burrow\state.json` was removed.
