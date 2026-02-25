# Contributing to mem-kernel-rs

Thanks for contributing. This project aims to provide a MemOS-compatible memory kernel in Rust.

## Ground rules

- Be respectful and collaborative.
- Keep PRs focused and small.
- Add or update tests for behavioral changes.
- Update docs when API behavior changes.

## Development setup

Prerequisites:

- Rust stable toolchain (`rustup` + `cargo`)

Run locally:

```bash
cargo run --bin mem-api
```

Run checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

## Commit and PR guidelines

- Use clear commit messages (imperative, present tense).
- In each PR description include:
  - Problem statement
  - What changed
  - How it was tested
  - Backward compatibility impact
- Link related issues if applicable.

## Testing expectations

- New features should include tests (unit/integration as appropriate).
- Bug fixes should include a regression test when feasible.

## API compatibility policy

`mem-api` targets MemOS-compatible request/response shapes for current endpoints.
If a change affects compatibility, call it out explicitly in PR and changelog.

## License

By contributing, you agree that your contributions will be licensed under Apache-2.0.
