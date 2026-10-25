# Launch kit (drafts)

**Status: drafts for a human to review and publish.** Nothing in this directory
posts, tags, signs, uploads, or submits a package. The repository still has no
published release or signed installer; the install path that works today is
from a checkout.

## Files

- [`announcement.md`](announcement.md) - short and long announcement copy, plus
  a Show HN draft.
- [`checklist.md`](checklist.md) - pre-release, release, package-manager, and
  post-launch steps, including the human-only actions.
- [`screenshots.md`](screenshots.md) - a shot list and capture commands. No
  screenshots are committed yet.
- [`../demo.md`](../demo.md) - the five-minute recording script.

## Rules for using these drafts

- Every product claim must be backed by [`README.md`](../../README.md), the
  user docs, or [`docs/benchmarks.md`](../benchmarks.md). If a claim is not
  measured or documented, remove it or mark it as unverified.
- Keep the boundary explicit: Burrow is not a Docker replacement, not a
  security sandbox, and not a production deployment tool.
- The only measured competitor performance numbers are the Docker Compose
  head-to-head in [`docs/benchmarks.md`](../benchmarks.md). The rest of the
  comparison is documentation-based. Do not add numbers for other tools.
- Do not claim a clean-machine pass, Windows 10 support, ARM64 support, or
  signed artifacts. Those remain unverified or human-only.
- Replace every `TODO(human)` placeholder only after the corresponding release
  or verification step actually happens.

## Human-only actions

Publishing, signing, tagging, package-manager submission, screenshot capture on a
clean machine, and social posting are human actions. See
[`docs/agent/NEEDS_HUMAN.md`](../agent/NEEDS_HUMAN.md) for the current list.
