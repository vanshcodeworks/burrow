# Channel-specific launch copy (drafts)

Nothing here posts anything. These are drafts for a human to review, adapt and
publish. Every number below comes from [`docs/benchmarks.md`](../benchmarks.md)
and was measured on one Windows 11 machine with small samples. Do not add
unmeasured claims.

Shared assets:
- 10-second demo clip (record with [`docs/demo.md`](../demo.md))
- [Numbers graphic](../assets/burrow-numbers.svg)
- [Comparison page](../comparisons/README.md) with official sources
- [Migration guide](../migration-from-docker-compose.md)

---

## Show HN

**Title**

> Show HN: Burrow – run a local dev stack natively on Windows, no Docker VM

**Body**

> I got tired of starting Docker Desktop just to run PostgreSQL, so I built
> Burrow: a single Windows executable that reads one `burrow.toml`, downloads
> SHA-256-verified native binaries, allocates stable ports, waits for health
> checks, and supervises everything in Windows Job Objects.
>
> The part I did not expect to be useful: `burrow init --from docker-compose.yml`.
> It converts an existing compose file into a `burrow.toml` — built-in images
> become managed packages, everything else becomes a container fallback, and it
> writes comments for the Docker-only fields it could not translate.
>
> On my machine, warm start to HTTP 200 was 1.84–1.88 s for Burrow versus
> 1.92–3.20 s for Compose; the Burrow daemon used 10.9–11.0 MB versus 37.2–62.1 MB
> for the containers (excluding the Docker VM). Cold start favored Compose
> (4.11 s vs 10.7 s) because initializing empty app data is slower here. These
> are small samples on one machine, not a benchmark suite.
>
> It is Windows 11 x86-64 only, not a sandbox, and not for production. Built-in
> packages are PostgreSQL 16, NATS 2.15 and Caddy 2.11.
>
> Source and the full honest comparison: github.com/vanshcodeworks/burrow

---

## Reddit

Pick one subreddit per post; do not cross-post the same text on the same day.

### r/programming — "I built a native Windows alternative to Docker Compose"

Lead with the engineering, not the marketing:

- Windows Job Objects for supervision; `CTRL_BREAK` then `TerminateJobObject`
- Pinned URL + byte size + SHA-256, resumable downloads, hardened extraction
- Per-project stable ports in `.burrow/ports.json`, `*.localhost` proxy
- Snapshots are full-directory copies with journaled restore, not DB dumps
- What it is not: no sandbox, no production support, Windows-only

End with the compose converter and the measured numbers, plus the caveat that
cold starts were slower.

### r/dotnet / r/csharp — "Run your local Postgres + NATS stack without Docker on Windows"

Lead with the pain: Docker Desktop licensing, VM memory, slow startup. Show:

```powershell
iwr https://raw.githubusercontent.com/vanshcodeworks/burrow/main/scripts/install.ps1 | iex
burrow init --from .\docker-compose.yml
burrow up
```

Mention that app runtimes (including .NET) are started from `PATH` with generated
environment variables, and that Burrow does not install language runtimes.

### r/webdev — "Point it at your docker-compose.yml and never start Docker again"

Lead with the one-line converter and the demo clip. Keep the numbers to two
lines. Link the migration guide for the full mapping.

---

## X / Twitter thread (7 posts)

1. Hook + demo clip: "I replaced Docker Compose with a single 11 MB Windows
   binary. Here's `burrow up` starting a full stack natively — no VM, no WSL."
2. The one-liner install (show the command, mention SHA-256 verification from
   the same GitHub Release).
3. The headline feature: `burrow init --from .\docker-compose.yml` — show a
   before/after screenshot of compose YAML → `burrow.toml`.
4. The numbers graphic. Caption: "Warm 1.84–1.88s vs 1.92–3.20s; daemon 11 MB vs
   37–62 MB containers (no VM). One machine, small samples, source linked."
5. What it actually does: verified native binaries, stable ports, health-gated
   startup, Job Objects, logs, snapshots, `burrow top`.
6. Honest limits: Windows 11 x64 only; built-ins are Postgres, NATS, Caddy;
   Redis/MySQL/MongoDB go through the optional container fallback; not a sandbox.
7. CTA: "Source, benchmarks and the comparison page:
   github.com/vanshcodeworks/burrow · MIT OR Apache-2.0"

---

## dev.to / Hashnode article outline

**Title:** "How I built a Windows-native Docker Compose alternative in Rust"

1. The problem: Docker Desktop on Windows means a VM, memory, licensing, and
   dynamic ports.
2. The model: one `burrow.toml` → daemon → per-service Job Objects; verified
   downloads into a content-addressed cache.
3. The interesting bits (with code links):
   - hardened zip/tar extraction (zip-slip, symlinks, size caps, `create_new`)
   - stable port allocation remembered per project
   - health-gated dependency graph and crash-loop breaker
   - converting compose YAML with a small dependency-free YAML reader
4. Measured results, with the method and caveats. Include the cold-start loss.
5. What I would not use it for: production, sandboxing, macOS/Linux, large
   service catalogs.
6. Try it: one-line install, converter, `burrow doctor`, `burrow up`.

---

## YouTube Short / TikTok (20–25 s)

- 0–3 s: Docker Desktop spinning up; caption "starting Docker just for Postgres?"
- 3–10 s: `burrow up` — trust prompt, download, services turn healthy, route opens
- 10–15 s: `burrow top` with logs and CPU
- 15–20 s: `burrow init --from .\docker-compose.yml` → generated `burrow.toml`
- 20–25 s: numbers card + "github.com/vanshcodeworks/burrow"

No voiceover needed; captions carry the story.

---

## Hacker News / Reddit comment responses (prepared facts)

- "Why not just use Docker?" → link the sourced comparison page; state the
  cold-start caveat upfront.
- "Is it a sandbox?" → no. Services run with your user rights; see security docs.
- "Where are Redis/MySQL/MongoDB?" → no verified native Windows build was
  available, so they use the optional container fallback rather than a fake hash.
- "Windows only?" → yes, x86-64; Windows 10, ARM64 and clean-machine runs are
  unverified.
- "Is it signed?" → no. Binaries are unsigned; no EV certificate yet.
