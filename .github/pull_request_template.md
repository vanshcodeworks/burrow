## Summary

<!-- What changed and why? Link the issue if one exists. -->

## Changes

- <!-- one logical change per bullet -->

## Testing

<!-- Paste the commands you ran and their results. -->

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets -- --test-threads=1
```

## Checklist

- [ ] The change is focused and uses Conventional Commits.
- [ ] I added or updated tests for behavior changes.
- [ ] I updated documentation for user-visible behavior.
- [ ] New claims are measured or cite a verifiable source.
- [ ] No secrets, generated passwords, or private paths are included.
- [ ] Any new `unsafe` code is confined to a crate's `win` module and has a `// SAFETY:` comment.
- [ ] Registry changes pin a verified upstream URL, size, and SHA-256.
- [ ] I have read `CONTRIBUTING.md` and the security and trust model.
