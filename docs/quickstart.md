# 快速开始

## 1. 环境准备

- Rust stable
- 可选：Qdrant（如果使用持久化向量库）
- OpenAI-compatible embedding API

## 2. 启动服务（内存向量库）

```bash
export EMBED_API_URL=https://api.openai.com/v1/embeddings
export EMBED_API_KEY=sk-...
export MEMOS_LISTEN=0.0.0.0:8001
cargo run --bin mem-api
```

健康检查：

```bash
curl -sS http://localhost:8001/health
```

## 3. 写入一条记忆（同步）

```bash
curl -sS -X POST http://localhost:8001/product/add \
  -H 'Content-Type: application/json' \
  -d '{
    "user_id": "u1",
    "mem_cube_id": "u1",
    "memory_content": "I like strawberries",
    "async_mode": "sync"
  }'
```

## 4. 查询记忆

```bash
curl -sS -X POST http://localhost:8001/product/search \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "What do I like?",
    "user_id": "u1",
    "mem_cube_id": "u1",
    "top_k": 5
  }'
```

## 5. 异步写入流程

提交异步任务：

```bash
curl -sS -X POST http://localhost:8001/product/add \
  -H 'Content-Type: application/json' \
  -d '{
    "user_id": "u1",
    "memory_content": "Async memory",
    "async_mode": "async"
  }'
```

返回 `task_id` 后轮询：

```bash
curl -sS "http://localhost:8001/product/scheduler/status?user_id=u1&task_id=<task_id>"
```

## 6. 多租户命名空间约定

- 写入空间：`writable_cube_ids -> mem_cube_id -> [user_id]`
- 查询空间：`readable_cube_ids -> mem_cube_id -> [user_id]`

建议：同一业务租户统一使用同一个 `user_id + mem_cube_id` 组合。
