# 配置说明

## 服务配置

- `MEMOS_LISTEN`：监听地址，默认 `0.0.0.0:8001`
- `RUST_LOG`：日志级别，默认 `info`

## Embedding 配置

- `EMBED_API_URL`：Embedding API 地址，默认 `https://api.openai.com/v1/embeddings`
- `EMBED_API_KEY`：API Key（可选，取决于上游）
- `EMBED_MODEL`：模型名（可选）

## 向量存储配置

不设置 `QDRANT_URL` 时使用内存向量库。

设置后启用 Qdrant：

- `QDRANT_URL`：例如 `http://localhost:6334`
- `QDRANT_COLLECTION`：集合名（可选）

## 审计日志配置

- `AUDIT_LOG_PATH`：设置后使用 JSONL 文件持久化审计日志
- 未设置时使用进程内存审计日志（重启后丢失）

## 生产建议

- 使用 Qdrant + `AUDIT_LOG_PATH`
- 配置进程守护（systemd / supervisor / k8s）
- 在网关层做鉴权与限流
