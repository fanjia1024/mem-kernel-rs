# 1.0 路线图（Draft）

目标版本：`v1.0.0`

## 里程碑

## M1: 安全与接口稳定（当前）

- [x] 多租户隔离（`user_id + mem_cube_id`）
- [x] scheduler 状态按 owner 隔离
- [x] 检索 filter 合并并强制租户过滤
- [x] 可选 API 鉴权（`MEMOS_AUTH_TOKEN`，Bearer）
- [ ] API 字段兼容性清单冻结（向后兼容策略）

## M2: 可靠性与可观测性

- [ ] `/ready` 就绪探针（依赖项检查）
- [ ] 结构化审计持久化轮转策略
- [ ] 关键路径指标（QPS/Latency/Error）
- [ ] 统一错误码枚举与文档

## M3: 持久化与恢复

- [ ] graph 持久化后端（替代纯内存）
- [ ] 数据导入导出工具
- [ ] 版本化迁移脚本

## M4: 文档与发布工程

- [x] 用户文档体系（quickstart/api/config/architecture）
- [x] Docker Compose 快速验收
- [ ] OpenAPI 文档导出
- [ ] 升级指南（0.x -> 1.0）

## M5: 1.0 发布门禁

- [ ] CI 必过：fmt/clippy/test/integration
- [ ] 安全扫描和依赖审计
- [ ] Release Candidate（`v1.0.0-rc.1`）
- [ ] 7 天稳定窗口后发布 `v1.0.0`

## 版本策略建议

- `0.x` 阶段允许破坏性改动，但要在 `CHANGELOG` 明确标注。
- 从 `1.0.0` 开始遵守 SemVer：
  - `MAJOR`：破坏性改动
  - `MINOR`：向后兼容功能增强
  - `PATCH`：向后兼容修复
