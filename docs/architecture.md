# 架构设计

## 核心组件

- `mem-api`：HTTP 接口层，负责请求解析、错误映射、审计记录
- `mem-cube`：编排层，统一 add/search/update/delete/get
- `mem-graph`：图存储抽象与内存实现，存文本与元数据
- `mem-vec`：向量存储抽象，内存实现 + Qdrant 实现
- `mem-embed`：Embedding 适配层
- `mem-scheduler`：异步写入队列与任务状态
- `mem-types`：DTO、traits、错误定义

## 写入链路

1. API 接收 `/product/add`
2. `mem-cube` 将文本转 embedding
3. 写入 graph 节点
4. 写入 vector item
5. 返回 memory id
6. 记录审计事件（同步成功或异步完成）

## 查询链路

1. API 接收 `/product/search`
2. query 转 embedding
3. 向量检索（含租户过滤 + 自定义 filter）
4. 回查 graph 节点补全 memory/metadata
5. 根据 `relativity` 过滤低分结果
6. 返回 `text_mem` bucket

## 多租户隔离

- 逻辑隔离键：`user_id` + `mem_cube_id`
- 向量检索层强制租户过滤
- 调度状态查询按 `user_id + task_id` 校验归属

## 删除语义

- 软删除：标记 `state=tombstone`，并移除向量
- 硬删除：图和向量都删除
