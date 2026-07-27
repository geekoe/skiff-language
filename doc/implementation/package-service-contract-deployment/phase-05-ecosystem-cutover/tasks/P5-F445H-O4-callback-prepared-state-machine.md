# P5-F445H-O4 Callback prepared state machine

状态：Ready。actual-Pending correction DAG 的 callback owner；与 O1–O3、O5并行。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

production prerequisite 为 Skiff integration `d39ad5b0`。

## 生产目标

把 in-process callback从跨 await借 caller context/borrowed owner heap guard改为专用状态机：

1. callback adapter提供 clone的 owner heap `Arc<tokio::sync::Mutex<RequestHeap>>`或等价
   `OwnedMutexGuard`入口；不能暴露任意公共 heap mutation API；
2. prepare校验 carrier/operation/generation，并把 caller args物化到 owner heap；
3. owned wait持有 owner heap guard、owned program context、owner args和调用信息，递归执行
   owner executable，不借 caller heap/env/Actor frame；
4. wait结束后释放或显式交回 owner guard/outcome；finalize才把 owner result导入 caller heap；
5. parameter materialization失败、method error/cancel/drop保留既有 owner-heap checkpoint和
   可见性语义，guard exactly once释放；
6. `OwnedProgramExecutionContext::capture`不得新增 caller Actor frame捕获。

现有 callback async入口可薄组合新阶段以维持编译；E4R负责最终 actual-Pending接线。

## Test-first 与验收

先写 RED；至少覆盖：

- wait存活时 caller heap/env可独立访问；
- Ready/Pending callback只执行一次；
- owner heap lock不会跨错误路径泄漏或重入死锁；
-参数 prepare失败checkpoint恢复；normal/error/cancel/drop finalize/cleanup exactly once；
- generation/operation/method ABI/fixed error语义不回归；
- callback递归 evaluator不持 caller Actor frame。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo test -p skiff-runtime-eval callback_native -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo test -p skiff-runtime-native callback_adapter -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo check -p skiff-runtime-eval -p skiff-runtime-native --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数。

## 写集与停止规则

只允许：

- `runtime/eval/src/assembly_execution/callback_native.rs`
- `runtime/eval/src/assembly_execution/callback_native/**`
- `runtime/native/src/callback_adapter.rs`
- `runtime/native/src/callback_adapter/**`
- 本 result

不得修改 `assembly_execution/mod.rs`、`eval_context.rs`、Actor、其它 native dispatch、host或
manifest。若必须改变 callback owner-heap可见语义、无法形成 owned guard、或需捕获 caller Actor
frame，立即 `TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o4-callback
branch   codex/p5-f445h-o4-callback
```

先提交 implementation，再提交
`P5-F445H-O4-callback-prepared-state-machine-result.md`；最终 clean，不 merge/rebase/push，
不运行 stable/live/network，不派子 Agent。
