# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

### Added
- Open source governance and community files (`LICENSE`, `CONTRIBUTING`, `CODE_OF_CONDUCT`, `SECURITY`).
- GitHub templates (issues/PR) and CI workflows.
- Dependabot configuration for Cargo and GitHub Actions.

### Changed
- Improved README with complete API overview and contributor workflow.

## [0.1.0] - 2026-02-25

### Added
- Initial Rust workspace for MemOS-compatible memory kernel.
- `mem-api` Axum server endpoints for add/search/scheduler/update/delete/get/audit.
- In-memory graph/vector stores and optional Qdrant vector backend.
- OpenAI-compatible embedding client and mock embedder for tests.
- Integration tests for add/search/update/delete/get/isolation/async workflow.
