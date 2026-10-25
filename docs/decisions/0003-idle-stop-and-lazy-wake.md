# 0003 - Idle stop with lazy wake-up

- Status: Accepted
- Date: 2026-10-25

## Context

`idle_stop = "10m"` asks Burrow to release a service's resources while nobody
uses it, and to bring the service back transparently on the next connection.
Two questions dominate the design: how "idle" is measured without instrumenting
every service, and how a connection meant for a *stopped* service can both
trigger the restart and still reach it.

## Decision

1. **Idleness is the absence of established TCP connections to the service's
   port.** The daemon samples `GetExtendedTcpTable` every two seconds and counts
   rows in the `ESTAB` state whose local port matches. Any connection — from the
   proxy, a health check that is still running, or a user's client — counts as
   activity. Sampling only starts once the project is fully up and the service
   is healthy, and only counts to zero after the whole configured duration has
   passed with zero connections.
2. **Idle-stop is a graceful stop with the port held by a wake listener.** The
   service is stopped exactly like `burrow down` stops it (registry stop
   commands first, then CTRL_BREAK, then terminate), after which the daemon
   binds `127.0.0.1:<port>` itself. Holding the port means no other process can
   steal it while the service sleeps, and any client that connects becomes a
   wake-up trigger.
3. **Routed services wake through the proxy first.** The proxy consults a
   shared idle set before connecting upstream. For an idle port it sends a wake
   request to the daemon over an unbounded channel and holds the client's
   request until the daemon replies (healthy) or fails, so the HTTP client sees
   either a real response or a specific error, never a refused connection.
4. **Direct connections wake through the listener.** The listener accepts one
   client, drops the listen socket (releasing the port for the real service),
   and hands the client to the daemon. The daemon parks the client until the
   service is healthy, then splices it to the service with
   `copy_bidirectional`. Clients that arrive in the gap between listener close
   and service bind are refused and must retry, which matches normal TCP
   behaviour during any restart.
5. **Wake-ups are deliberate, not crashes.** A wake resets the restart backoff
   and counts in the `restarts` column, but cannot trip the crash-loop breaker
   by itself. Parked proxy requests and clients parked during a failed wake get
   a definitive error once the service gives up.

## Consequences

- Idle services show as `idle` in `burrow status` (no PID, no resource use),
  and `burrow restart <service>` works on them like on any stopped service.
- The connection table is a whole-system view, so only loopback-bound services
  (Burrow's default posture) are measured meaningfully.
- A routed service also keeps the wake listener armed; direct connections to
  its port wake it without going through the proxy.
- WebSocket and HMR connections hold the service awake for exactly as long as
  they are established, which is the behaviour developers expect.
