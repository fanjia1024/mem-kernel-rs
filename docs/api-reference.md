# API 参考

Base URL: `http://<host>:8001`

## 通用响应结构

```json
{
  "code": 200,
  "message": "Success",
  "data": {}
}
```

## `POST /product/add`

写入记忆。

关键字段：

- `user_id` string 必填
- `async_mode` string，`sync` 或 `async`，默认 `sync`
- `messages` array，可选
- `memory_content` string，可选（当 `messages` 缺失时使用）
- `mem_cube_id` string，可选
- `writable_cube_ids` array，可选

说明：如果 `async_mode=async`，返回 `task_id`，随后通过调度接口查询状态。

## `GET /product/scheduler/status`

查询异步写入任务。

Query 参数：

- `user_id` string 必填
- `task_id` string 必填

返回：

- 200：任务存在且归属该 `user_id`
- 404：任务不存在或非该 `user_id` 所有

## `POST /product/search`

检索记忆。

关键字段：

- `query` string 必填
- `user_id` string 必填
- `top_k` number，可选，默认 10
- `mem_cube_id` string，可选
- `readable_cube_ids` array，可选
- `filter` object，可选（会与服务端租户过滤合并）
- `relativity` number，可选（相似度阈值，`> 0` 时生效）

注意：服务端会强制注入 `mem_cube_id` 过滤，不能通过 `filter` 读取其他租户数据。

## `POST /product/update_memory`

更新已有记忆。

关键字段：

- `memory_id` string 必填
- `user_id` string 必填
- `memory` string 可选（更新文本并重建向量）
- `metadata` object 可选

## `POST /product/delete_memory`

删除记忆。

关键字段：

- `memory_id` string 必填
- `user_id` string 必填
- `soft` bool，可选，默认 `false`

行为：

- `soft=true`：标记 tombstone，并从向量库移除
- `soft=false`：图节点和向量都硬删除

## `POST /product/get_memory`

按 id 获取单条记忆。

关键字段：

- `memory_id` string 必填
- `user_id` string 必填
- `include_deleted` bool 可选，默认 `false`

## `GET /product/audit/list`

查询审计日志。

Query 参数（全可选）：

- `user_id`
- `cube_id`
- `since`（ISO8601）
- `limit`
- `offset`

## `GET /health`

返回 `ok`。
