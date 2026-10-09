## What and why

## Linked issue

## Checks
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass
- [ ] Tests added or updated; external-service tests are `#[ignore]` and documented
- [ ] No secrets, personal data or machine-specific paths
- [ ] **Clean-room:** I did not use decompiled, disassembled or otherwise extracted code or assets from proprietary software (docs/clean-room.md)
- [ ] Any code copied or adapted from open source is recorded in NOTICE
- [ ] Performance-sensitive change: measurements included (docs/performance.md)
- [ ] Works on Windows, macOS and Linux (or platform code is cfg-gated)
