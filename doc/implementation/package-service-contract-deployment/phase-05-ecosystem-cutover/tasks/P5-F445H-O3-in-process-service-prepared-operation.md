# P5-F445H-O3 In-process service prepared operation

状态：Ready。actual-Pending correction DAG 的 canonical activation-relative service owner；
与 O1、O2、O4、O5并行。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

production prerequisite 为 Skiff integration `d39ad5b0`。

## 生产目标

只重构 canonical in-process service boundary，不复活 legacy outbound relay：

- prepare在 caller同步 segment完成 target/plan检查、caller→provider参数物化，创建独立
  provider heap、owned provider context与 provider request；
- unary owned wait以 `async move`持有全部 provider状态，绝不借 caller heap/env/
  `EvalContext`/Actor frame，返回 owned `{provider_heap, outcome}`；
- finalize只在 caller Actor resume后导出 fixed service failure或把 provider normal result
  materialize回 caller heap；
- provider cancel/deadline/drop和late result沿现有 request owner exactly once；
- provider Ready由后续 E3首次 poll决定，不得在 owner内预判/预释放。

serverStream只同步完成 source/producer setup并返回 stream handle；producer task和consumer
`next()`各自保留现有 cleanup owner，不进入 unary prepared协议。

现有 `execute_service_call(context,...).await` 在 E4R前可作为薄
prepare→wait→finalize wrapper维持编译；不得复制 materialization、provider request或错误状态机。

## Test-first 与验收

先写 RED；至少覆盖：

- owned unary wait存活时 caller heap/env可独立访问；
- provider立即完成不由 owner强制 cut，pending只启动 provider一次；
- normal/fixed failure/user error/cancel/deadline/drop的 caller import与provider cleanup；
- finalize前不修改 caller heap，失败时不留下部分 import；
- serverStream setup同步，producer/consumer cleanup和late item隔离不回归；
- owned provider context不捕获 caller Actor execution frame。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o3-in-process-service/build/cargo-target \
  cargo test -p skiff-runtime-eval async_stream_cancel -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o3-in-process-service/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o3-in-process-service/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数。

## 写集与停止规则

只允许：

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/**`
- 本 result

不得修改 `assembly_execution/mod.rs`、`eval_context.rs`、provider/request public API、legacy
service dispatch、Actor、host/native或manifest。E4R负责最终 module-root/call-site接线。

若 owned wait仍捕获 caller、provider result必须在resume前写 caller heap、或 unary重构要求吞并
stream owner，立即 `TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o3-in-process-service
branch   codex/p5-f445h-o3-in-process-service
```

先提交 implementation，再提交
`P5-F445H-O3-in-process-service-prepared-operation-result.md`；最终 clean，不
merge/rebase/push，不运行 stable/live/network，不派子 Agent。
