# P5-F440V1 WebSocket RPC eval test-module split result

状态：`PASS / TEST_MODULE_SPLIT_ONLY`。

`runtime_websocket_jsonrpc.rs` 的单一 inline `#[cfg(test)] mod tests` 已机械移动到同模块
`runtime_websocket_jsonrpc/tests.rs`。production、public API、private visibility、test name、
fixture/helper 与断言均未改变。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 integration baseline | `b3ca0f0e225ff7bcd7c90c8a16b4335fa3224e58` | `c1c439d836c9981ef7fd887eb83433ed178f74ab` |
| worktree 实际起点 | `582bc04752ae42a9c5db464dcf44265616524285` | `ca1abb3d11cdad4e516a2f5e66b5fd51113e41a5` |
| implementation | `201ec5dbc4f069c000f79b1b8acb260e20ba34b6` | `ac4aec2ffe1005f495b41b245e7d5486248be7ea` |

`b3ca0f0e..582bc047` 只新增 F440V1 与 F440W 两个 task 文件，没有 production/test
差异。Implementation 与本文 result 分离提交；result commit/tree 由最终交付消息记录。

## 2. 结构性 RED

移动前先验证目标布局尚未成立：

- `runtime/eval/src/runtime_websocket_jsonrpc/tests.rs` 不存在；
- 原文件第 461 行为 `#[cfg(test)]`，第 462 行仍为 inline `mod tests {`；
- inline 模块含 10 个 `#[tokio::test]`，与 F440V result 的 10 项计数一致。

因此本 leaf 命中了真实的结构缺口，而不是零测试或已满足目标。

## 3. 机械移动与等价审计

终态父文件只保留：

```rust
#[cfg(test)]
mod tests;
```

新子文件继续以 `use super::*;` 使用父模块 private symbol；未扩大任何 visibility。逐字节
`cmp` 证明原文件前 460 行 production 与 implementation baseline 完全相同，production-side
唯一 diff 是把 inline test module declaration/body 替换为上述 external module declaration。

移动前后测试名列表完全相同，数量均为 10。测试体从 baseline 精确抽取，仅由 `rustfmt` 去除
外层 module 对 Rust 代码造成的结构缩进；没有手工编辑 fixture/helper/断言。

一次未提交的全行去缩进试探曾同时改变 raw YAML fixture 的内容缩进，focused test 在 fixture
parse 阶段将其拒绝。该试探已完全丢弃，测试体随后从 `HEAD` 原文重新抽取；最终 implementation
的 10 项 focused test 全部通过，证明该 raw-string 漂移未进入提交。

## 4. 规定验证

所有 Cargo 命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc` | PASS：10 passed / 209 filtered；两个 integration binaries 均 0 executed（分别 4 / 6 filtered） |
| `cargo check -p skiff-runtime-eval` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

Cargo 只显示 repository 既有 unused/dead-code/unreachable-pattern warnings，没有本 leaf
新增 warning 或 test/check failure。

## 5. Scope audit

Implementation 精确修改两个任务允许文件：

- `runtime/eval/src/runtime_websocket_jsonrpc.rs`
- `runtime/eval/src/runtime_websocket_jsonrpc/tests.rs`

本文是唯一额外写入的 result。没有修改 production implementation、API、assertion、fixture、
helper、其它 test/module、Cargo 配置或文档；没有启动 instance/live/server/network；未派子
Agent，未 merge、rebase 或 push。
