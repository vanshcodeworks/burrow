# Benchmark history (machine specs mandatory; never publish estimates)

Format: one block per measurement run. Reference machine described in
`docs/benchmarks.md`.

## 2026-10-25 — phase-2 spot checks (not a benchmark suite)

Machine: the development laptop used for phase 2; same machine class as
`docs/benchmarks.md`. These are single-shot observations, not the
recorded benchmark methodology; treat as indicative only.

- Warm `burrow up` of the nats+caddy verification project (3 services):
  healthy within ~3 s of `up --detach` (observed once; not instrumented).
- Container fallback round trip (`traefik/whoami`, image pull excluded —
  image was already local): healthy in ~13 s wall clock for the whole
  test including engine checks (single observation).
- No memory measurements taken this cycle; idle-stop savings measurement
  is queued as QUEUE B1.1.
