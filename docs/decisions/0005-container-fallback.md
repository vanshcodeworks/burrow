# 0005 - Container fallback for non-native services

- Status: Accepted
- Date: 2026-10-25

## Context

Some services have no trustworthy native Windows build (the registry's Redis
entry is absent for exactly that reason). The roadmap allows an optional
fallback: `use = "container:IMAGE"` delegates to Docker or Podman, clearly
labelled, without making Burrow a container runtime or bundling one.

## Decision

1. `use = "container:IMAGE"` is validated as a reference (non-empty image, no
   whitespace) and handled entirely outside the package registry: no download,
   no cache entry, no hash. The engine pulls the image.
2. Engine discovery prefers `$BURROW_CONTAINER_ENGINE`, then `docker`, then
   `podman`, resolved on `PATH`. `burrow up` and the daemon run `<engine>
   version` before starting anything, so "installed but not running" fails with
   a remedy instead of an engine error from the detached daemon. `burrow
   doctor` reports the same as the `container` check.
3. The supervised process is the engine client: `<engine> run --rm --name
   burrow-<project>-<service> -p 127.0.0.1:<host>:<container_port> [-e ...]
   [--memory N] IMAGE`. The port mapping is loopback-only. `container_port`
   (default: the service's own port) says which port the image listens on
   inside the container. Only user-configured `env` entries plus `PORT`, `HOST`,
   `BURROW_SERVICE`, `BURROW_PROJECT`, and `BURROW_SERVICE_PASSWORD` cross the
   boundary; host paths like `PATH` and data directories do not.
4. Graceful stop runs `<engine> stop --time <grace> <name>` before the job is
   terminated, and every start first runs `<engine> rm -f <name>` so a leftover
   from a crashed daemon cannot wedge `docker run` with a name collision.
5. `burrow status` labels the service with its `container:IMAGE` reference in
   the RUNTIME column and in `--json`, next to registry keys like
   `postgres@16` for native packages.

## Consequences

- **Container filesystems are ephemeral** (`--rm`): `burrow snapshot` and
  `restore` do not cover container services, and stopping the project removes
  whatever the container wrote. This is stated in the config reference; data
  that must survive belongs in a native package.
- **A daemon crash can leave the container running**: the Job Object guarantees
  the death of the engine's *client* process, not the engine's backend. The
  `rm -f` on the next start is the cleanup path; `docker ps` shows anything the
  fallback leaves behind.
- Resource limits map to `--memory` only; CPU caps apply to the client process
  and are not translated.
- The trust model is unchanged: the `use` value is part of the executable hash,
  so changing an image re-prompts, and the trust summary spells out that the
  engine, not Burrow, pulls the image.
