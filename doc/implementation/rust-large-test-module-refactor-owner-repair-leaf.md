# Rust 大型测试模块重构：Service DB owner 修复叶子

日期：2026-08-01

状态：complete；动态验证因 P1 磁盘硬阻断移交 combined probe

直接父节点：
[`rust-large-test-module-refactor-repair-batch.md`](./rust-large-test-module-refactor-repair-batch.md)；该父节点继续引用
F/G result、阶段 DAG 与唯一权威设计。本文只记录 R1 的机械 owner 修复，不改变设计语义、完成标准或其它
repair 节点。

## 入口身份与预检

- baseline commit：`2c4356c8827930967886c270f74d89ddacdd678a`。
- baseline tree：`751f523cfd3c4241960862702475db082520b770`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-owner-repair` /
  `codex/rust-test-owner-repair`。
- 创建 worktree 前以 `git show` / `git grep` 做了零-worktree 只读预检，未 build、test 或写缓存；目标路径与
  branch 均不存在。
- 十个 Service DB 领域模块共有 102 个测试函数；既有 `prepared_runtime/**` 共有 11 个测试函数。
- owner 范围内唯一 ignore 仍是
  `service_db_runtime_create_and_find_runtime_roundtrips_local_interface`，原因是
  `requires a local MongoDB replica set and real network resources`。

三个待移动函数在 baseline 上均唯一，旧 owner 与父节点 finding 一致：

| 函数 | 旧 owner | 权威 owner |
| --- | --- | --- |
| `service_db_capability_context_does_not_require_request_frame` | `error_contract.rs` | `provider.rs` |
| `object_metadata_accepts_retention_field` | `runtime_config.rs` | `metadata.rs` |
| `skiff_file_record_document_preserves_capability_record_fields` | `metadata.rs` | `mapping.rs` |

## 写集与实现约束

唯一代码目标是移动以上三个完整测试。函数名、测试属性与函数体逐字保持；只允许为目标/源模块闭合最小
import。不得修改 support、生产源码、测试逻辑、其它测试、line gate、原 A/B/D/F/G 文档、repair batch 或
non-blocking 文档问题。

叶子证据文件为本文与对应
`rust-large-test-module-refactor-owner-repair-result.md`。验证使用独立 target
`/Users/geek/workspace/skiff-rust-test-owner-repair/build/cargo-target`，不运行 ignored live Mongo 或 full verify。
