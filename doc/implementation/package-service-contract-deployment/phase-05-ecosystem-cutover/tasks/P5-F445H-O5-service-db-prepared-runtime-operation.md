# P5-F445H-O5 Service-DB prepared runtime operation

状态：Ready。actual-Pending correction DAG 的 service-db前置；完成后才可启动 O6 eval DB状态机。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`

production prerequisite 为 Skiff integration `d39ad5b0`。本节点只分离 service-db
raw/recoverable codec与外部 wait，不修改 evaluator transaction/lease控制流。

## 生产目标

对 eval实际消费的 `*_runtime` DB入口建立 prepared协议：

- prepare同步读取 caller `RuntimeValue`/heap，完成 mapping、wire/recoverable input编码和所有
  owned command构造；
- owned wait只持 store/client/session、owned BSON/document/command和 recoverable retention
  状态，不借 caller `RequestHeap`；
- wait返回 raw/owned outcome；
- finalize同步接收 caller heap，完成 recoverable decode、runtime value materialization与必要的
  retention receipt；
-外部 DB副作用只执行一次，不能通过重建 future重放 command；
- existing public async runtime入口可薄组合 prepare→wait→finalize维持调用方编译，但核心 wait
  不再持 heap。

至少覆盖 eval可达的 find-one/find-many/update/replace/create及 lease-read所需路径；raw
`DbDocument` API不做无关重写。transaction begin/commit/abort和 claim/renew/release raw waits可
保持现有 store API，O6负责 evaluator阶段状态机。

`lib.rs` 已超过 1900行。新增 prepared types/codec必须放窄 child module，root只保留声明与薄
转发。

## Test-first 与验收

先写 RED；至少覆盖：

- prepared wait存活时 caller heap可独立 mutation；
- finalize前 caller heap无新增/原地修改；
- ordinary与 recoverable read/write结果和现有 wire identity完全一致；
- mapping/encryption/retention root行为不回归；
- wait error、decode/resource failure、drop不重复DB副作用且不留下部分 caller materialization；
- existing raw API与 transaction/lease API不回归。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5-service-db/build/cargo-target \
  cargo check -p skiff-runtime-service-db --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5-service-db/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数。

## 写集与停止规则

只允许：

- `runtime/service-db/src/lib.rs`
- `runtime/service-db/src/store.rs`
- `runtime/service-db/src/mapping.rs`
- `runtime/service-db/src/prepared_runtime.rs`
- `runtime/service-db/src/prepared_runtime/**`
- `runtime/service-db/src/tests.rs`
- `runtime/service-db/src/tests/**`
- 本 result

不得修改 eval、Actor、host/native、DB语言语义、Cargo manifest或 lockfile。若 recoverable或
encryption owner要求上述写集外 production、wait仍必须持 caller heap、或分离会改变 storage
identity/副作用顺序，立即 `TASK_SCOPE_EXPANDED`，不得吞并 O6。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5-service-db
branch   codex/p5-f445h-o5-service-db
```

先提交 implementation，再提交
`P5-F445H-O5-service-db-prepared-runtime-operation-result.md`；最终 clean，不
merge/rebase/push，不运行 stable/live/network，不派子 Agent。
