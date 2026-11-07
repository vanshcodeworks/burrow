# Security policy

Burrow runs native Windows processes under your user account. It is a process
manager, not a sandbox or a privilege boundary. The full trust model, including
what the approval prompt does and does not cover, is in
[`docs/security.md`](docs/security.md).

## Supported versions

Security fixes are applied to the latest `0.1.x` release. There is no long-term
support branch for pre-1.0 versions.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private
vulnerability reporting for the repository. If private reporting is unavailable,
open a minimal issue asking for a private channel without including exploit
details.

Include:

- The Burrow version (`burrow --version`) and Windows version.
- A minimal reproduction, including the relevant `burrow.toml` with secrets
  removed.
- The impact you believe is possible, such as command execution, path escape,
  credential disclosure, or proxy exposure.
- Any relevant logs with generated passwords and private paths redacted.

Do not include `.burrow/secrets`, `BURROW_SERVICE_PASSWORD`, or a connection URL
containing a password.

You should receive an acknowledgement within a few business days. Please allow
time for a fix and coordinated disclosure before publishing details.
