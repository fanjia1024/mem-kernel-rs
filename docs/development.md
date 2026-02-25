# 开发与测试

## 常用命令

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

## 本地运行 API

```bash
cargo run --bin mem-api
```

## 代码结构建议

- DTO 和 trait 修改放在 `mem-types`
- 行为编排放在 `mem-cube`
- 接口协议与状态码映射放在 `mem-api`
- 新后端实现优先复用已有 `GraphStore`/`VecStore` trait

## 测试策略

- 集成测试优先覆盖 API 行为与跨模块链路
- 回归修复必须附带测试
- 多租户隔离和权限边界要有明确测试

## 提交前检查

- 保持 `cargo fmt/clippy/test` 全通过
- 如果改动接口行为，同步更新 `docs/api-reference.md`
- 如果改动配置项，同步更新 `docs/configuration.md`
