# mem-kernel-rs v0.1.0 Release Notes

Release date: 2026-02-25

## Highlights

- Initial MemOS-compatible Rust memory kernel release.
- Axum API server with endpoints for add/search/update/delete/get and async scheduler status.
- In-memory graph/vector backends and optional Qdrant vector integration.
- OpenAI-compatible embedding client and mock embedder for test/dev.
- Integration tests covering sync/async add flow, isolation, update, and delete lifecycle behavior.

## Open Source Readiness

This release includes complete open source baseline:

- Apache-2.0 license
- Contribution guide
- Code of Conduct
- Security policy
- Governance document
- Changelog
- Issue and PR templates
- CI workflow and Dependabot config

## Verification

Validated locally with:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`

## Notes

- This is a pre-1.0 release. Breaking changes may occur in future minor releases.
