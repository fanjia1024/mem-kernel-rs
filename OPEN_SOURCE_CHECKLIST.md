# Open Source Readiness Checklist

This checklist tracks what is needed before public launch.

## Governance and legal

- [x] License file (`LICENSE`)
- [x] Contribution guide (`CONTRIBUTING.md`)
- [x] Code of Conduct (`CODE_OF_CONDUCT.md`)
- [x] Security policy (`SECURITY.md`)
- [x] Governance document (`GOVERNANCE.md`)
- [x] Changelog initialized (`CHANGELOG.md`)

## Repository hygiene

- [x] README updated with architecture, API, quick start, and dev flow
- [x] Issue templates configured
- [x] Pull request template configured
- [x] Editor defaults (`.editorconfig`)
- [x] Crate license metadata added

## Automation

- [x] CI workflow (`fmt` + `clippy` + `test`)
- [x] Dependabot for Cargo and GitHub Actions

## Validation

- [x] `cargo test --workspace` passes locally
- [x] `cargo fmt --all --check` passes locally
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes locally

## Before making repository public

- [ ] Set repository description/topics/homepage in GitHub settings
- [ ] Enable branch protection for `main`
- [ ] Require PR + CI checks before merge
- [ ] Enable GitHub Security Advisories
- [ ] Add at least one maintainer backup/admin
- [ ] Cut first tagged release (`v0.1.0`)
