# 常见问题

## 1. 启动后搜索报 embedding 错误

检查：

- `EMBED_API_URL` 是否可访问
- `EMBED_API_KEY` 是否有效
- 上游服务是否兼容 OpenAI Embeddings 请求

## 2. 搜索不到刚写入的数据

排查顺序：

1. 确认 add 返回 `code=200`
2. 确认查询使用相同的 `user_id/mem_cube_id`
3. 检查是否被软删除（`state=tombstone`）
4. 若设置了 `relativity`，尝试降低阈值

## 3. 异步任务状态 404

可能原因：

- `task_id` 错误
- 使用了不同 `user_id` 查询（状态对非 owner 不可见）

## 4. Qdrant 模式下写入失败

检查：

- `QDRANT_URL` 是否正确
- Qdrant 服务是否可用
- 集合权限与向量维度是否符合写入数据

## 5. 审计日志重启后丢失

- 未设置 `AUDIT_LOG_PATH` 时，审计日志仅在内存中。
