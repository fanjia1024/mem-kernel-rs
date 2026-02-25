# mem-kernel-rs

Rust implementation of a MemOS-compatible memory kernel: unified add/search/update/delete/get over graph + vector memory backends.

## Status

- Stage: pre-1.0 (`0.1.x`)
- API style: MemOS-compatible request/response envelopes
- Runtime: Axum + Tokio

## Project layout

- `crates/mem-types`: DTOs, traits, lifecycle/audit types
- `crates/mem-graph`: graph store trait + in-memory graph implementation
- `crates/mem-vec`: vector store trait + in-memory + optional Qdrant backend
- `crates/mem-embed`: OpenAI-compatible embedding client + mock embedder
- `crates/mem-cube`: `NaiveMemCube` orchestration (graph + vec + embedder)
- `crates/mem-scheduler`: in-memory async add scheduler
- `crates/mem-api`: Axum REST API server

## API endpoints

- `POST /product/add`
- `POST /product/search`
- `GET /product/scheduler/status?user_id=...&task_id=...`
- `POST /product/update_memory`
- `POST /product/delete_memory`
- `POST /product/get_memory`
- `GET /product/audit/list`
- `GET /health`

## Quick start

### 1) Run with in-memory vector store

```bash
export EMBED_API_URL=https://api.openai.com/v1/embeddings
export EMBED_API_KEY=sk-...
export MEMOS_LISTEN=0.0.0.0:8001

cargo run --bin mem-api
```

### 2) Optional: use Qdrant vector store

```bash
export QDRANT_URL=http://localhost:6334
export QDRANT_COLLECTION=memos
cargo run --bin mem-api
```

### 3) Optional: persistent audit log

```bash
export AUDIT_LOG_PATH=./audit.jsonl
cargo run --bin mem-api
```

## Example requests

### Add memory (sync)

```bash
curl -sS -X POST http://localhost:8001/product/add \
  -H 'Content-Type: application/json' \
  -d '{
    "user_id": "u1",
    "mem_cube_id": "u1",
    "memory_content": "I like strawberry",
    "async_mode": "sync"
  }'
```

### Search memory

```bash
curl -sS -X POST http://localhost:8001/product/search \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "What do I like?",
    "user_id": "u1",
    "mem_cube_id": "u1",
    "top_k": 10
  }'
```

### Update memory

```bash
curl -sS -X POST http://localhost:8001/product/update_memory \
  -H 'Content-Type: application/json' \
  -d '{
    "memory_id": "<memory-id>",
    "user_id": "u1",
    "memory": "I like strawberry and peach"
  }'
```

### Delete memory (soft delete)

```bash
curl -sS -X POST http://localhost:8001/product/delete_memory \
  -H 'Content-Type: application/json' \
  -d '{
    "memory_id": "<memory-id>",
    "user_id": "u1",
    "soft": true
  }'
```

## Multi-tenant behavior

Storage is isolated by `user_id` and cube namespace.

- Write scope resolution: `writable_cube_ids` -> `mem_cube_id` -> `[user_id]`
- Read scope resolution: `readable_cube_ids` -> `mem_cube_id` -> `[user_id]`

Use consistent user/cube identifiers across write and read requests.

## Development

### Local checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

### CI

GitHub Actions runs format, clippy, and tests on pushes/PRs to `main`.

## Open source docs

- Contribution guide: `CONTRIBUTING.md`
- Code of Conduct: `CODE_OF_CONDUCT.md`
- Security policy: `SECURITY.md`
- Governance: `GOVERNANCE.md`
- Changelog: `CHANGELOG.md`

## License

Apache-2.0. See `LICENSE`.
