# Rust 大型测试模块重构：Service DB owner 修复结果

日期：2026-08-01

状态：PASS（机械 owner 修复与静态验收）；Cargo/rustfmt 动态验收因 P1 磁盘硬阻断 deferred

任务合同：
[`rust-large-test-module-refactor-owner-repair-leaf.md`](./rust-large-test-module-refactor-owner-repair-leaf.md)；
直接父节点与唯一权威设计由该合同继续追溯。

## 代码身份与交付

- baseline：commit `2c4356c8827930967886c270f74d89ddacdd678a`，tree
  `751f523cfd3c4241960862702475db082520b770`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-owner-repair` /
  `codex/rust-test-owner-repair`。
- 唯一集成 owner：`/root/rust_test_integrator`；本分支不 merge、push 或清理。
- 最终交付 commit/tree 与 clean 状态由 handoff 记录，避免在同一提交内写入自引用 identity。

## 机械移动结果

三个测试都已移动到权威 owner：

| 函数 | baseline owner | 终态 owner |
| --- | --- | --- |
| `service_db_capability_context_does_not_require_request_frame` | `error_contract.rs` | `provider.rs` |
| `object_metadata_accepts_retention_field` | `runtime_config.rs` | `metadata.rs` |
| `skiff_file_record_document_preserves_capability_record_fields` | `metadata.rs` | `mapping.rs` |

五个源/目标模块原本都只使用同一条测试模块 import：
`use super::{super::*, support::*};`。移动后的符号仍由该既有 import 闭合，因此没有新增、扩大或重排 import。
没有修改函数名、属性、函数体、support、其它测试或生产源码。

三个完整 `#[test]` + 函数的 baseline/终态 SHA-256 分别为：

- capability context：`83c85ef05de54c641ece6a061e12a6e72ea879c24d7ca160179c205e783c101f`；
- retention metadata：`00ed4d08972b110d2ea4fa5180e3f319b9e7a02f70e2f35955cb720a55ba7c78`；
- file record mapping：`c7453707c962b13663cda8a05cfa484bc23f2e81ff9cc048636f08b833a58135`。

每项移动前后 hash 完全相同。

## 测试身份与领域分布

静态解析 baseline 与终态的函数名及 `#[test]` / `#[tokio::test]` / `#[ignore]` 属性：

- 十个领域模块均为 102 个测试、102 个唯一函数名；排序后的名称+属性清单 SHA-256 均为
  `1c3a0e3aeda4e7075d0d63c4313759c51c49f5a9b25b7d536095f3f227209aa7`。
- `prepared_runtime.rs` 与 `prepared_runtime/**` 均为 11 个测试、11 个唯一函数名；排序后的名称+属性清单
  SHA-256 均为 `d24c6f55a0c71a285b4a0f7e8a9868dd9342d3422944681c995499a72afa16d0`。
- owner 范围内仍只有一个 ignore：
  `tests::live_mongo::service_db_runtime_create_and_find_runtime_roundtrips_local_interface`；保留
  `#[tokio::test]` 与原因 `requires a local MongoDB replica set and real network resources`。

修复后的十域分布为：

| 模块 | 测试数 |
| --- | ---: |
| `error_contract.rs` | 10 |
| `provider.rs` | 4 |
| `runtime_config.rs` | 16 |
| `metadata.rs` | 16 |
| `mapping.rs` | 19 |
| `lease.rs` | 4 |
| `recoverable.rs` | 16 |
| `mongo.rs` | 7 |
| `encrypted_mapping.rs` | 9 |
| `live_mongo.rs` | 1 |

合计仍为 102。

## 自验收矩阵

| 层级 | 结果 | 证据 |
| --- | --- | --- |
| baseline identity | PASS | 零-worktree `git show` 确认精确 commit/tree；三个旧 owner 各唯一一次 |
| 三测试身份 | PASS | 完整属性+函数逐项 SHA-256 与 baseline 相同 |
| owner 唯一性 | PASS | 三个函数终态只在对应权威 owner 各出现一次 |
| 102 测试双射 | PASS | baseline/终态名称+属性集合均 102 unique，清单 hash 相同 |
| 11 prepared 双射 | PASS | baseline/终态名称+属性集合均 11 unique，清单 hash 相同 |
| live ignore | PASS | 唯一 owner、函数名、tokio 属性与 ignore 原因未变 |
| whitespace | PASS | `git diff --check` |
| rustfmt | DEFERRED | P1 报告剩余空间约 4.0 GiB 并禁止继续任何 Cargo/rustfmt 命令 |
| crate test/list/check/Clippy | DEFERRED | 同一 P1 磁盘硬阻断；本 owner 未启动 Cargo、未创建或增长独立 target |

动态验证移交合流后同一代码状态上的 combined probe。未运行 ignored live Mongo、full verify、runtime/router/Mongo，
也未删除任何其它 owner 的 target/cache 数据。

## 写集

实际写集严格为：

- `runtime/service-db/src/tests/error_contract.rs`
- `runtime/service-db/src/tests/provider.rs`
- `runtime/service-db/src/tests/runtime_config.rs`
- `runtime/service-db/src/tests/metadata.rs`
- `runtime/service-db/src/tests/mapping.rs`
- 本 leaf/result 文档。

未修改 support、生产源码、其它测试、callable-effects、line gate、原 A/B/D/F/G 文档、repair batch、无关 linker
rustfmt 文件或 non-blocking 文档问题。
