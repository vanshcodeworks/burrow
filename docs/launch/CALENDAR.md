# Two-week launch sequence (plan, not automation)

Nothing here posts anything. This is a schedule for a human to execute. Do not
publish a channel until the demo asset and the numbers graphic are final and
every claim in the copy has been checked against [`docs/benchmarks.md`](../benchmarks.md).

## Before day 1

- [ ] Full local gate green on the release commit: fmt, clippy, tests, audit, deny
- [ ] GitHub Release `v0.2.0` exists with zip, SBOM and `SHA256SUMS.txt`
- [ ] One-line installer verified from a clean user profile
- [ ] `burrow init --from docker-compose.yml` verified on a real compose file
- [ ] 10-second demo clip recorded and watched once at full size
- [ ] Numbers graphic rendered and checked against the source data

## Week 1 — the technical audience

**Day 1 — Show HN.** Post the copy in [`CHANNELS.md`](CHANNELS.md#show-hn) with
the demo clip. Stay in the comments for the first two hours. Answer the prepared
responses honestly; link the [comparison page](../comparisons/README.md) and
the [security model](../security.md) rather than arguing from memory.

**Day 2 — r/programming.** Use the engineering-led post. Lead with Job Objects,
verified extraction and the dependency-free Compose converter, not the marketing
tagline.

**Day 3 — Hacker News follow-up / dev.to.** Publish the technical "how I built
it" article from the outline. This is the long-lived searchable asset.

**Day 4 — r/dotnet and r/webdev.** One subreddit per day, channel-specific hook,
no cross-posting the same text.

**Day 5 — rest and triage.** Answer issues, fix documentation gaps revealed by
questions, and note every claim someone challenged.

## Week 2 — the broader audience

**Day 6 — X thread.** Seven posts from [`CHANNELS.md`](CHANNELS.md#x-twitter-thread-7-posts).
Clip first, link second.

**Day 7 — YouTube Short / TikTok.** The 20–25 s script. Captions carry the story.

**Day 8 — example gallery announcement.** Link the four
[example stacks](../examples.md), especially the Compose migration example.

**Day 9 — winget / Scoop PRs.** Open the upstream PRs once the manifests match
the published hashes. This is slow but permanent discovery.

**Day 10 — update post.** "What the first week taught us": real bugs fixed,
platform reports received, and anything that turned out to be unverified.

**Days 11–14 — long tail.** Answer comments, improve the docs that people
actually opened, and re-post the demo clip with a different hook only if there
is genuinely new information.

## What success looks like

Not virality — that cannot be guaranteed. Useful signals:

- someone reports running it on Windows 10 or ARM64 (an unverified row becomes
  verified)
- an issue arrives with a real `burrow.toml` and a reproducible failure
- a contributor picks up a good first issue
- the migration guide gets used without you having to explain the mapping

If those happen, the project is being adopted even if the post did not trend.
