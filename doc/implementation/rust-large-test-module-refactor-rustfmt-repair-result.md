# Rust 大型测试模块重构：linker rustfmt 修复结果

日期：2026-08-01

状态：PASS；等待集成 owner 接收

任务合同：
[`rust-large-test-module-refactor-rustfmt-repair-leaf.md`](./rust-large-test-module-refactor-rustfmt-repair-leaf.md)；
直接父节点与唯一权威设计由该合同继续追溯。

## 代码身份与交付

- baseline：commit `2d040ac9b7324741f4083823178a2f0fd838c16e`，tree
  `8920797830eb9b9a6217514816cc7a6f87b337cb`。
- implementation：commit `d20b50b3b082c789b635eab209b6feb72454d455`，tree
  `cf480432b730989945ffaf282be6126b980f0029`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-rustfmt-repair` /
  `codex/rust-test-rustfmt-repair`。
- 唯一集成 owner：`/root/rust_test_integrator`；本分支未 merge、push 或清理。
- 最终 result 提交/tree 与 clean 状态由 handoff 记录，避免在同一提交中写入自引用 identity。

## 机械格式化结果

唯一源码改动是
`runtime/linker/src/assembly/tests/cross_package_actor.rs` 顶部 `skiff_artifact_model` import 列表的 rustfmt
换行；没有重排 import 项、修改函数/属性/测试体或触碰 linker 行为。

- baseline 目标 blob：`8bbe074b19c47f6178298a5a59950e876baeea86`。
- rustfmt 后目标 blob：`c4b471f0dfd284bfc0f6d627dae668b8c177e62f`。
- formatter：`rustfmt 1.8.0-stable (6b00bc3880 2025-06-23)`，edition 2021。
- 将 baseline blob 通过 `rustfmt --edition 2021 --emit stdout` 后得到 blob
  `c4b471f0dfd284bfc0f6d627dae668b8c177e62f`，与交付文件逐字节相同；因此全部源码差异均是同一 formatter 对
  baseline 的确定输出。
- 前后均有 16 个函数声明、2 个 `#[test]`、0 个 ignore；完整属性清单 SHA-256 前后均为
  `28bac90d8f4ace3406a8845a2f1050cdcb239e4390a6d33bb2f85e90ec371126`，完整函数声明清单 SHA-256 前后均为
  `5c610834e2589cc808611e9c80f805204dfdfa8b82d412fd85072e10ac3e570a`。

两个测试名保持为：

- `cross_package_actor_registry_get_and_method_call_link_through_provider_artifact`
- `cross_package_actor_reference_fails_closed_without_provider_actor_declaration`

## 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 仅格式化授权的 linker test-only 文件 | 目标 blob 是 baseline 经同版本 rustfmt 的逐字节结果；diff 仅为 import 换行 | implementation commit 的源码写集只有目标路径 | 单文件 `rustfmt --check` PASS |
| 函数、属性与测试逻辑不变 | 16 个函数声明和 2 个测试属性清单 hash 前后相同；整文件 formatter 等价 | 0 个 ignore；两个测试名各保留一次 | formatter 等价审计 PASS |
| 解除 workspace rustfmt blocker | implementation tree 上没有额外 Rust diff | `git diff --check` 与精确写集审计 PASS | `cargo fmt --all -- --check` PASS |
| 不扩大生产/配置/依赖范围 | 无生产、manifest、lockfile、配置或其它 test 源码改动 | `git diff-tree --name-status` 仅 leaf 与目标源码 | 未运行完整 selector 或 live 测试 |

## 命令证据

| 层级 | 命令 | owner | 代码状态 | 结果 | 覆盖范围 |
| --- | --- | --- | --- | --- | --- |
| formatter identity | `rustfmt --version` | R2 developer | implementation worktree | exit 0 | rustfmt 1.8.0-stable |
| formatter equivalence | `git show <baseline>:<target> \| rustfmt --edition 2021 --emit stdout \| git hash-object --stdin` 与工作文件 blob 比较 | R2 developer | pre-implementation commit | exit 0，blob 相同 | 整个目标文件 |
| 属性/函数身份 | baseline/结果的 attribute 与 function declaration 清单 `diff -u` 和 SHA-256 | R2 developer | implementation tree | exit 0，清单相同 | 16 个函数、2 个测试属性 |
| 单文件格式 | `rustfmt --edition 2021 --check runtime/linker/src/assembly/tests/cross_package_actor.rs` | R2 developer | implementation tree | exit 0 | 唯一源码写集 |
| workspace 格式 | `cargo fmt --all -- --check` | R2 developer | implementation tree | exit 0 | authority 第 4 节 rustfmt gate |
| whitespace | `git diff --check` | R2 developer | implementation worktree | exit 0 | 全部交付 diff |
| 写集 | `git diff-tree --no-commit-id --name-status -r d20b50b3b082c789b635eab209b6feb72454d455` | R2 developer | implementation commit | exit 0 | leaf + 单一目标源码 |

未运行 compiler/runtime/rust-quality/checks 完整 selector、ignored/live 测试、runtime/router/Mongo，也未写入或清理
任何 Cargo target、其它 worktree 或外部状态。最终合流状态仍须由集成 owner 运行 repair batch 规定的 combined probe。

## 写集

实际写集严格为：

- `runtime/linker/src/assembly/tests/cross_package_actor.rs`
- `doc/implementation/rust-large-test-module-refactor-rustfmt-repair-leaf.md`
- 本 result 文档

未修改其它源码、测试、证据记录、line gate、manifest/lockfile、依赖或配置。
