# 0006 - Listener inspection for `expose` enforcement

- Status: Accepted
- Date: 2026-10-26

## Context

`expose = true` controls only Burrow's reverse proxy bind address. It cannot
change how a service process binds its own socket. A service started with
`listen_addresses=0.0.0.0`, `HOST=0.0.0.0`, or a framework default that listens
on every interface is reachable directly from the network even though the
project never opted in. Configuration review alone cannot catch this: the bind
address may come from a framework default, an environment variable, or a child
process.

Burrow needs a read-only way to see what its supervised process trees actually
listen on and to warn before a user assumes `expose = false` means "loopback
only".

## Decision

1. `burrow_supervisor::listening_sockets(roots)` returns TCP listeners owned by
   the given root PIDs or any descendant. It reads the Windows IP Helper tables
   (`GetExtendedTcpTable`, `TCP_TABLE_OWNER_PID_ALL`) for IPv4 and IPv6 and
   keeps rows in `MIB_TCP_STATE_LISTEN`.
2. Descendants are discovered with a Toolhelp process snapshot
   (`CreateToolhelp32Snapshot`/`Process32FirstW`/`Process32NextW`) and a walk
   from each root. A listener in a child process is attributed to the root PID
   whose tree contains it, so the daemon can map it back to a service even when
   the listening process is a wrapper, a shell, or a forked worker.
3. `Listener` reports the owning PID, the root PID, port, address, and whether
   the address is the IPv4/IPv6 wildcard. A listener is considered loopback
   only when its address parses as a loopback IP; everything else is
   non-loopback.
4. `burrow doctor` adds a `listeners` check. When the project is stopped it
   reports "not inspected". When no supervised PID is running it reports "no
   supervised process is running". Otherwise it warns for every non-loopback
   listener whose service lacks `expose = true`, naming the service, PID,
   address, and port, with a remedy to bind loopback or opt in. A wildcard bind
   on a service with `expose = true` is accepted. A loopback-only project is
   accepted.
5. The check is a warning, not a failure. It describes a real exposure but does
   not prevent startup: the user may have intentionally bound an interface
   address for a reason Burrow cannot see, and `doctor` must not make a working
   project unusable. Startup does not block on socket inspection.
6. Inspection is read-only. It opens no sockets, changes no firewall state, and
   does not terminate anything. If the IP Helper or Toolhelp call fails, the
   check reports a warning with the API error and a remedy; it never silently
   passes.

## Consequences

- **TCP only.** UDP listeners are not inspected. A UDP service bound to a
  non-loopback address is not flagged.
- **Point-in-time.** A process that exits between the process snapshot and the
  socket read contributes no row. A PID reused inside that window could, in
  theory, attribute an unrelated listener to a service. The window is small and
  the result is a warning, not an enforcement action.
- **No privilege escalation.** The owner-PID tables are readable without
  administrator rights on supported Windows versions. If a locked-down machine
  denies the query, `doctor` shows the warning rather than claiming the project
  is clean.
- **IPv6 may be disabled.** The IPv6 query is independent; a machine without
  IPv6 simply has no IPv6 rows.
- **A listener started outside the supervised tree is invisible.** Processes
  that break away from the Job Object, or that a service asks another service
  manager to start, are outside Burrow's attribution. The same limitation
  applies to the process-cleanup guarantee.
- The check reflects current sockets, not configuration. Fixing
  `burrow.toml` requires a restart before the warning clears.
