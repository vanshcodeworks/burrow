# Security and trust

Burrow runs native Windows processes under your user account. It is a process
manager, not a container runtime, sandbox, or privilege boundary. A
`burrow.toml` is executable configuration: treat it the way you treat a
`Makefile`, `package.json` scripts block, or a shell script in a cloned
repository.

This page describes what the trust prompt protects, what it does not, and where
local state lives.

## The trust prompt

The first `burrow up` in a project prints every command that will run and every
managed package that will be downloaded, then asks for confirmation:

```text
Burrow will run commands from C:\dev\shop\burrow.toml in C:\dev\shop

  api: node server.js
  db: download and run `postgres@16` from the Burrow registry

Treat this like a Makefile or package.json script: it can run arbitrary code.
Approve and start? [y/N]
```

Only `y` or `yes` (case-insensitive) approves. Anything else, including an empty
line or end-of-file, aborts without starting anything. When standard input is
not a terminal and `--yes` was not passed, Burrow refuses instead of assuming
approval.

Approval is stored in `.burrow/trust.json` as a hash. The next `burrow up`
re-prompts only when the hash changes. `burrow up --yes` records approval for
the current configuration without printing the summary; use it only after you
have reviewed the file in another way, such as in a reviewed pull request or a
throwaway CI checkout.

### What the hash covers

The hash is SHA-256 over canonical JSON containing:

- the project name;
- each service name;
- each service's `use`, `run`, and `cwd` values.

Adding, removing, or renaming a service changes the hash. Changing a command,
package reference, or working directory changes it too. `burrow.local.toml` is
merged before parsing, so a local override that changes one of those fields also
re-prompts. The prompt prints a note when a local override file is present.

### What the hash does not cover

Changing the following does **not** re-prompt:

- `env`, including variables such as `PATH`, `NODE_OPTIONS`, `PYTHONPATH`, or
  `DATABASE_URL`;
- the contents of scripts and binaries that `run` points at;
- the contents of a `seed` file;
- `port`, `needs`, `restart`, `memory`, `cpu`, `health`, `route`, `expose`, and
  `idle_stop`;
- service data under `.burrow/data/`.

This is deliberate: the prompt is a command-level check, not a content hash of
the repository. A commit that only edits `server.js` or `env` will run without a
new prompt after you update the checkout. Review pull requests and `git diff`
before running them, exactly as you would review a changed npm script.

Two consequences are worth stating explicitly:

- `env` can change what a command actually does. `PATH` controls which program
  is found; `NODE_OPTIONS=--require ./x.js` makes Node load code before the
  entry point; `PYTHONPATH` changes imports. Burrow does not treat these as
  executable configuration, even though they can be.
- The managed package digest protects the download, not your project. A
  registry entry is compiled into `burrow.exe`; a project cannot point `use` at
  an arbitrary URL.

### Trust state is not a security boundary

`.burrow/trust.json` is a plain JSON file. Any process that can write it can
approve a configuration. Its purpose is to avoid nagging after a deliberate
review, not to prove who approved what. Delete the file to force a prompt:

```powershell
Remove-Item .burrow\trust.json
```

`burrow doctor` reports whether the current hash is approved but does not
approve it.

`burrow exec SERVICE -- COMMAND` does not require the trust file, because the
command comes from your shell rather than from the project. It does resolve the
project's environment, templates, and package paths, and it may install the
service's managed package on first use.

### Container services

`use = "container:IMAGE"` is part of the executable hash, so changing an image
re-prompts, and the trust summary names the engine explicitly. Burrow never
downloads images itself: the engine does, under the engine's own account and
registry credentials. The engine client runs as an ordinary supervised child,
but a container's lifecycle is ultimately owned by the engine's backend —
Burrow stops containers best-effort (`<engine> stop`, then `rm -f` on the next
start) and cannot guarantee cleanup if the engine itself is killed. Container
filesystems are ephemeral and are not covered by snapshots. See
[ADR 0005](decisions/0005-container-fallback.md).

## Secrets

Each configured service gets a generated secret: 24 random bytes encoded as 48
lowercase hexadecimal characters. Burrow creates missing entries when a project
is resolved, which happens on the first `burrow up`, `burrow env`, or
`burrow exec`.

Secrets are stored in `.burrow/secrets` as plaintext JSON:

```json
{
  "version": 0,
  "secrets": {
    "db": "0123…",
    "api": "89ab…"
  }
}
```

Burrow does not set a custom ACL on this file or on `.burrow/`; they inherit
the permissions of the project directory. On a normal single-user workstation
that means your account can read them, and so can any process running as you.
The repository's own `.gitignore` ignores `.burrow/`, but `burrow init` does not
edit your project's `.gitignore`. Add the directory yourself:

```gitignore
.burrow/
burrow.local.toml
```

A service receives its own secret as `BURROW_SERVICE_PASSWORD`. A configuration
can also interpolate any known service's secret with `{{service.password}}`,
which is how a database password is passed to an application if you choose to
use one. PostgreSQL's built-in entry initializes its cluster with `-A trust`, so
the generated password is not used for authentication in that example; it is
still delivered to the process environment.

`burrow env SERVICE` is the only command that prints a complete service
environment, including `BURROW_SERVICE_PASSWORD`. Treat its output as sensitive.
Other commands avoid printing secrets, but Burrow does not redact service logs:
if your application logs its environment or connection string, the secret ends
up in the log ring buffer and in `burrow logs` output.

To rotate every generated secret, stop the project, delete `.burrow/secrets`,
and start again:

```powershell
burrow down
Remove-Item .burrow\secrets
burrow up
```

Rotation can break a service whose data was initialized with the old
credential. Snapshot the data first if that matters.

## Local state and the control token

While a project runs, `.burrow/state.json` contains the daemon PID, ports, the
per-service snapshot, and the control bearer token. The token is 32 random
bytes encoded as 64 hex characters. The control listener binds only to
`127.0.0.1`.

The token is the only authentication on that listener. It is written to a
plaintext file readable by your user, so any process running as you can read it
and ask the daemon to stop, restart, or return logs. This is a local
convenience, not a defense against a compromised account.

Public output such as `burrow status --json` omits the token. Service logs and
`.burrow/daemon.log` can still contain anything the services print.

## Network exposure

By default the reverse proxy binds to `127.0.0.1`. If any routed service sets
`expose = true`, the proxy binds to `0.0.0.0`; remote peers can then reach only
the routes whose service has `expose = true`, while loopback clients can reach
every route.

`expose` changes the proxy's bind address. It does **not** change how your
service binds. If your command listens on `0.0.0.0` or `::`, its service port is
reachable directly, regardless of `expose`. Burrow only controls its own
listeners. Commands are responsible for binding to loopback when that is what
you want; PostgreSQL's registry entry passes `listen_addresses=127.0.0.1`.

`burrow doctor` inspects the actual TCP sockets owned by supervised process
trees and warns when a service listens on a non-loopback address without
`expose = true`. The check is named `listeners`, runs only while the project is
running, and names the service, PID, address, and port. It is a warning rather
than a failure because a deliberate interface bind is a valid configuration.
Inspection is read-only: it uses the Windows IP Helper owner-PID tables and a
Toolhelp process snapshot, opens no sockets, and never changes firewall state.
It covers TCP only, not UDP, and a process outside the supervised tree is not
attributed. If the operating system denies the query, `doctor` reports that it
could not inspect rather than reporting the project as clean. See
[ADR 0006](decisions/0006-listener-inspection.md).

The proxy speaks plain HTTP and routes by the `Host` header. It does not
terminate TLS, authenticate users, or add authorization. Do not expose a route
to an untrusted network.

Managed services inherit the environment of the process that started the
daemon, plus their own overrides. Start Burrow from a shell that does not hold
unrelated credentials (cloud tokens, registry passwords, SSH agent variables)
if the project's commands do not need them.

## Package downloads

The registry is compiled into `burrow.exe`. Each `windows-x86_64` artifact pins
an HTTPS URL, an exact byte size, and a SHA-256. Burrow refuses to extract an
archive until both the size and digest match, and it re-hashes a cached archive
before reusing it.

Extraction is hardened independently:

- absolute paths, drive letters, `..`, backslashes, trailing dots or spaces, and
  reserved Windows device names are rejected;
- symlinks, hard links, and special entries are rejected;
- total uncompressed size is capped at 8 GiB, a single file at 4 GiB, and the
  archive at 250,000 entries and 64 path components;
- files are created with `create_new`, so an entry cannot follow a link planted
  by an earlier entry;
- extraction happens in a temporary directory and is renamed into the
  content-addressed cache only after it succeeds.

There are limits to this protection:

- The digest proves the download matches the registry, not that the upstream
  vendor is trustworthy. Burrow does not audit or patch third-party binaries.
- After a package is extracted, `burrow up` trusts its marker file and does not
  hash every extracted file. A process that can write to
  `%LOCALAPPDATA%\burrow\cache` can modify an installed package and have it run
  later. `burrow pkg verify` re-hashes archives and validates markers; it is an
  integrity check, not a full-file audit.
- A registry entry can run arbitrary commands from the extracted package. You
  are trusting the package vendor and the reviewed registry entry.
- Burrow downloads packages from the vendor. It does not redistribute them and
  does not override the vendor's license.

## Process containment

Each managed service runs in a Windows Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When the daemon drops the last job handle,
Windows terminates any process still in the job, including descendants that did
not break away. The job also applies the configured `memory` limit to the
process and the job, and the configured `cpu` percentage as a hard CPU rate cap.
This is cleanup and resource governance.

It is **not** isolation:

- services run at your integrity level and can read or write anything your
  account can;
- services share your user profile, network identity, and access tokens;
- there is no filesystem, registry, device, or network namespace;
- a service can start processes outside the job if it has the rights to break
  away;
- there is no AppContainer, restricted token, or low-integrity boundary.

Do not run untrusted code with Burrow just because the process is in a job
object.

## What Burrow does not protect against

Be explicit about the boundary:

- **A malicious configuration you approved.** The prompt is informational. Once
  approved, the commands have your full privileges and can exfiltrate files,
  install persistence, or contact any network.
- **A repository that changes after approval.** Changes to `env`, scripts, seed
  data, or files a command reads do not re-prompt.
- **Other processes running as you.** They can read `.burrow/secrets`,
  `.burrow/state.json`, and the package cache, or write `.burrow/trust.json`.
- **Another local user on a weakly permissioned project directory.** Burrow
  relies on the filesystem ACL to keep other accounts out. It does not add one.
- **Network attackers.** The proxy has no TLS or authentication, and `expose`
  widens its bind address. Service ports may already be exposed independently.
- **Vulnerabilities in upstream binaries or your own dependencies.** There is no
  sandbox, no image scanning, and no CVE tracking.
- **Data safety.** Snapshots are plain directory copies, not database-consistent
  backups. Stop services before copying. See the
  [CLI data-command notes](cli.md#data-commands).
- **Denial of service.** CPU and memory caps help, but a service can still
  exhaust disk, ports, or other shared resources within your account's limits.
- **Secrets in logs.** Burrow does not scan or redact service output.

## Hardening checklist

- Review `burrow.toml` and `burrow.local.toml` before the first start, and review
  diffs before pulling.
- Add `.burrow/` and `burrow.local.toml` to the project's `.gitignore`.
- Use `--yes` only in CI or disposable environments where the file was reviewed
  elsewhere.
- Keep `expose = false` unless a route must be reachable from another machine.
- Make services bind to `127.0.0.1`, not `0.0.0.0`, unless you intend otherwise.
- Start Burrow from a shell without unrelated credentials.
- Delete `.burrow/trust.json` after changing a project's commands outside your
  normal review flow.
- Run `burrow doctor` after machine or configuration changes.
- Keep `burrow.exe` and your toolchains up to date.

## Reporting a vulnerability

This repository does not currently publish a dedicated security contact. If you
are reading a published copy, use that host's private security-advisory flow
rather than a public issue. Do not paste secrets, tokens, or full
`.burrow/state.json` contents into a report.

See also:

- [Configuration reference](config-reference.md)
- [CLI reference](cli.md)
- [Registry authoring guide](registry.md)
- [Troubleshooting](troubleshooting.md)
