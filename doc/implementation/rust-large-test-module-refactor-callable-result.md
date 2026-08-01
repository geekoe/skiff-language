# Callable effects 大型测试模块重构：开发结果

日期：2026-08-01

状态：PASS，等待 C / integration checkpoint

执行合同：[`rust-large-test-module-refactor-callable-leaf.md`](./rust-large-test-module-refactor-callable-leaf.md)。
调度信封误写的 `rust-large-test-module-refactor-tasks/callable-effects*.md` 已由协调 owner 裁决作废；实现严格使用
冻结 baseline 父节点规定的本 leaf/result 路径，没有创建第二套文档。

## 代码状态与提交

- baseline：commit `805426f2249ca24d7c3b46439ac5a60be2ca3ae2`，tree
  `5db1c89a0c47e3ccf84cb564610b17f68a916c0e`。
- 结构提交：`526a7da0b1af62b899b0d15e4f93e774cfb35643`，tree
  `85c3f04b0c5339fea7cab2a855df9f54d33452b9`。
- harness 提交：`6258a7a413f54de84fdd2c896c47e54fbf9bee2f`，tree
  `300a35e6990841037e7fde70cd4517f58d8b1d82`。
- branch / worktree：`codex/rust-test-callable` / `/Users/geek/workspace/skiff-rust-test-callable`。
- 本文件的 result commit/tree 由最终交接报告给集成 owner；提交不能自引用自身 identity。

## 实际终态

根 `compiler/source/src/callable_effects/tests.rs` 为 7 行，只声明六个领域模块和一个 support 模块。
测试映射如下：

| owner | 测试数 | 实际职责 |
| --- | ---: | --- |
| `analysis_resolution.rs` | 18 | pending/default、local/cross-file graph、target/interface/actor resolution 与 mixed target fail-closed |
| `heap_provenance.rs` | 19 | fresh/alias、projection/store、cycle/SCC、formal/actual alias 与 identity transfer |
| `escape_boundaries.rs` | 8 | throw/rethrow、stream/spawn/callback、DB write/transaction escape lane |
| `native_functions.rs` | 18 | context-free、HTTP/file/config、std/package native summary |
| `receiver_builtins.rs` | 15 | Date/String/Bytes/JsonObject/Map/Array receiver contextual transfer |
| `dependencies_contracts.rs` | 8 | dependency artifact/signature/field callable 与 contract descriptor/fail-closed |
| `support.rs` | 0 | 唯一 compile harness、dependency fixtures 和窄断言 |

`AnalysisFixture` 现在统一单源、多源、module/package、dependency analysis、package alias/dependency/artifact、
platform/prelude 初始化和 success/error compile 路径。旧 `analyze*` 多入口及
`#[allow(clippy::too_many_arguments)]` 已删除。所有 Skiff 行为源码仍位于各自测试；没有源码模板或参数矩阵。
最终最大领域文件为 `analysis_resolution.rs` 994 行，`support.rs` 为 434 行。

## 测试身份与 fixture 审计

- baseline detached worktree 上实际运行 `cargo test --manifest-path compiler/source/Cargo.toml
  callable_effects::tests -- --list`：86 tests，0 benchmarks；旧路径均为
  `callable_effects::tests::<function>`。该辅助 worktree 与独立 target 已在取证后清理。
- 最终代码上相同 list 命令：86 tests，0 benchmarks；每条路径只插入一个设计规定的领域段。
- 基于 baseline Git object 与最终六个领域文件的静态函数名集合比较：86 / 86，排序集合完全相同。
- `#[ignore]` 审计：baseline 0，最终 0；所有测试继续使用 `#[test]`。
- 原文件 89 个 raw Skiff fixture 与最终六个领域文件的 SHA-256 multiset 比较：89 / 89，完全相同。
- 领域计数合计 `18 + 19 + 8 + 18 + 15 + 8 = 86`；support 中没有 `#[test]`。

## 自验收矩阵

所有 Cargo 命令均使用独立
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-rust-test-callable/build/cargo-target`。

| 层级 | 命令 / 探针 | 代码状态 | 结果 | 覆盖 |
| --- | --- | --- | --- | --- |
| baseline list | `cargo test --manifest-path compiler/source/Cargo.toml callable_effects::tests -- --list` | `805426f` | PASS，86 | 真实旧测试全名 |
| final list | 同上 | `6258a7a4` tree | PASS，86 | 六域新全名 |
| focused | `cargo test --manifest-path compiler/source/Cargo.toml callable_effects::tests -- --test-threads=1` | `6258a7a4` tree | PASS，86 passed / 0 failed / 0 ignored | callable-effects 行为 |
| fixture identity | Git object/current 文件 raw-string SHA-256 multiset 比较 | `805426f` → `6258a7a4` | PASS，89 / 89 identical | 完整显式 Skiff raw fixture |
| function identity | Git object/current 静态测试函数名集合比较 | `805426f` → `6258a7a4` | PASS，86 / 86 identical | 名称双射，无复制/删除/重命名 |
| ignore | `rg '#\[ignore'` + test list | `805426f` → `6258a7a4` | PASS，0 → 0 | 属性边界 |
| format | `cargo fmt --manifest-path compiler/source/Cargo.toml -- --check` | `6258a7a4` tree | PASS | 受影响 crate rustfmt |
| Clippy | `cargo clippy --manifest-path compiler/source/Cargo.toml --tests` | `6258a7a4` tree | PASS（exit 0；workspace 既有 advisory warnings，改动文件无 warning） | crate test 编译与 lint |
| static harness | `rg` 检查旧 helper、`too_many_arguments` 和测试 owner | `6258a7a4` tree | PASS | 单一 builder / support 无测试 |
| diff | `git diff --check` 与 `git diff --name-only 805426f...HEAD` | 最终提交前 | PASS | 空白与写集 |

本开发节点未运行 full verify、compiler selector 或 line gate；它们分别由后续 integration、stable gate 和
line-gate owner 独占。未启动 runtime/router/Mongo，未修改生产代码/API、Cargo 配置/依赖、service-db 或
`scripts/check-rust-file-lines.mjs`。

## 实际写集

- `compiler/source/src/callable_effects/tests.rs`
- `compiler/source/src/callable_effects/tests/analysis_resolution.rs`
- `compiler/source/src/callable_effects/tests/heap_provenance.rs`
- `compiler/source/src/callable_effects/tests/escape_boundaries.rs`
- `compiler/source/src/callable_effects/tests/native_functions.rs`
- `compiler/source/src/callable_effects/tests/receiver_builtins.rs`
- `compiler/source/src/callable_effects/tests/dependencies_contracts.rs`
- `compiler/source/src/callable_effects/tests/support.rs`
- `doc/implementation/rust-large-test-module-refactor-callable-leaf.md`
- `doc/implementation/rust-large-test-module-refactor-callable-result.md`

开发节点完成；证据在上述源码、manifest/依赖、工具链或测试环境变化后失效，后续只能由唯一集成 owner
cherry-pick，不在本分支自行合并或 push。
