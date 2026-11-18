# Community and contributing

Burrow is a small project. The fastest way to help is to try it on a real
Windows machine and report exactly what happened.

## Where to go

| Goal | Where |
| --- | --- |
| Report a bug or a confusing error | [GitHub Issues](https://github.com/vanshcodeworks/burrow/issues) |
| Ask a question or share a stack | [GitHub Discussions](https://github.com/vanshcodeworks/burrow/discussions) |
| Contribute code or docs | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| Find a scoped first task | [Roadmap](roadmap.md#good-first-issue-candidates) |
| See what is verified | [Compatibility matrix](compatibility.md) |
| Read the trust model | [Security and Trust](security.md) |

## Good first contributions

These are deliberately small. Each has a clear test strategy and none weakens
the trust model. Full list in the [roadmap](roadmap.md#good-first-issue-candidates).

- Add `burrow completions <shell>` for PowerShell tab completion
- Add `burrow env --format cmd`
- Add a long-path warning to `burrow doctor`
- Extend the Compose converter with one more common Compose construct
- Add a registry-entry walkthrough to `docs/registry.md`
- Harden `scripts/install.ps1` with an installed-binary smoke test

## Reporting a bug well

Include:

1. `burrow --version` and Windows version (`winver`)
2. The `burrow.toml` (redact secrets — Burrow generates them in `.burrow/secrets`)
3. The exact command and full output
4. `burrow doctor` output
5. Whether the terminal was elevated (it should not be)

`burrow doctor --report` produces a redacted diagnostic bundle. Check it before
sharing; redaction covers common secret patterns and temporary paths, but you
are responsible for what you post.

## Reporting an unverified platform

The compatibility matrix marks Windows 10, ARM64, true standard-user accounts,
OneDrive/UNC paths, FAT/exFAT volumes, corporate antivirus, `LongPathsEnabled=0`
and 8.3-disabled volumes as **unverified**. A clear "it works" or "it fails with
this message" report for any of these is more valuable than a new feature. Run
[`scripts/compatibility.ps1`](../scripts/compatibility.ps1) if you can, and
paste the JSON.

## Adding a package to the registry

Do not start with the code. Start with evidence:

1. Download the exact upstream Windows x86-64 artifact yourself
2. Record its byte size and SHA-256, and confirm the URL is the vendor's official
   HTTPS download
3. Check the archive layout so the `binaries` paths are right
4. Follow the [registry guide](registry.md)

An entry with an empty, copied, or unverified hash will be rejected. The four
placeholder packages removed before v0.1.0 are the example of what not to do.

## Communication norms

- Be honest about what was and was not tested. "I tried it on Windows 10 Home
  and it worked" is a useful data point; "works everywhere" is not.
- Prefer a failing test with a fixture to a prose description.
- Do not claim a channel is published until an actual install from it succeeds.
- Keep product docs and internal notes saying the same thing.
