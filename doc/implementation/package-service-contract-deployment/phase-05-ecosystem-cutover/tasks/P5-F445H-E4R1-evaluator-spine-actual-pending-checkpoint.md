# P5-F445H-E4R1 evaluator spine, actual-Pending and checkpoint

状态：Ready。E4R 第一波共享实现检查点；完成后解除 R2 timeout、R3 concurrent 和 R4
stream/activation 三个并行叶子，不代表 E4R/F445H 完成。

## 直接父节点与精确代码状态

- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`
- `P5-F445H-J1-prepared-operation-combined-review-result.md`

本任务文件完整描述本节点需求。需要理解依据时只沿直接父节点引用向上读取，不默认重新解释顶层
设计。任务 base 为 integration commit `5ad126eb`；其中 production tree 与 J1 冻结 checkpoint
`99acfd13` 相同，后续提交只增加 E4R0 task/result 文档。

J1 已证明 native、service、Actor、callback、service DB 的 prepared-operation wait future 均
不借 caller heap/env/context。E1 已提供 owner-aware checkpoint；E3 已提供第一次真实
`Pending` 才释放 Actor segment 的 `await_if_pending`。不得回改这些 owner。

## 本节点唯一职责

本节点必须同时完成：

1. 把 `eval_context.rs` 拆成 root 薄转发和预分配 child surface；
2. 将 function/block、loop、generated/literal chunk 和本节点实际等待点迁到现有 E1
   `ProgramExecutionContext::checkpoint`，真实关闭纯 CPU 路径；
3. 删除静态 `maySuspend` / binding/effect 驱动的 Actor 预释放，关闭九组中的八组
   actual-Pending；
4. 修复 ordinary/Actor 共用 test-only `TestExecutionControl` 缺少 current
   `ExecutionScope` 的 fixture；
5. 为 R2/R3/R4 预声明互不冲突的 child 与测试文件，并保持三个后继 surface 的冻结中间态。

九组实际等待为：

- `Emit` 的三处 send；
- remote interface；
- callback capability；
- activation-relative service；
- Actor dispatch；
- legacy service dependency；
- native。

本节点关闭除 activation-relative service 之外的八组。activation 的旧 pre-suspend 行为移入
`actual_pending/activation.rs` 原样保留，交给 R4 原子关闭；不得提前修改
`async_stream_cancel.rs`。

## 唯一写集

Production：

- `runtime/eval/src/eval_context.rs`
- 新增 `runtime/eval/src/eval_context/checkpoint.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending/activation.rs`
- 新增 `runtime/eval/src/eval_context/timeout.rs`
- 新增 `runtime/eval/src/eval_context/concurrent.rs`
- `runtime/eval/src/assembly_execution/mod.rs`

Tests 与 test-only fixture：

- `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs`
- 新增 `runtime/eval/src/assembly_execution/ordinary/test_runtime/scoped_execution.rs`
- `runtime/eval/src/program_execution/execution_scope_tests.rs`
- 新增
  `runtime/eval/src/program_execution/execution_scope_tests/evaluator_checkpoint.rs`
- 预声明并新增空的或最小可编译的
  `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending/tests.rs`
- 新增 `runtime/eval/src/eval_context/concurrent/tests.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests.rs`
- 新增
  `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs`
- 预声明并新增空的或最小可编译的
  `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_concurrent.rs`

交付文档：

- 新增
  `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`

不得修改：

- `runtime/eval/src/lib.rs`；
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs` 及其 child；
- `program_stream.rs`、`program_invocation.rs`；
- E1/E2/E3、O1–O6 prepared owner core；
- callback provider core、capability-context、host/native adapter；
- artifact/compiler/linker、Router；
- Cargo、manifest、lockfile；
- 其它任务/result 文档。

如果 Rust formatter 只机械改动写集内文件，可以保留；写集外任何格式变化必须还原。

## 结构终态

### Root 与 child

- `eval_context.rs` 只保留既有 dispatch、module 声明和薄转发；不得继续增加新的数百行 helper
  区。
- timeout statement/expression 四个 arm 中对应的两个 timeout arm，薄转发到
  `timeout.rs`；R1 中 child 必须稳定返回当前同义的
  `F445H-E4 evaluator integration is required ...` fail-closed diagnostic。
- concurrent statement/value 两个 arm薄转发到 `concurrent.rs`；R1 中同样保持当前同义的
  fail-closed diagnostic。
- activation-relative service route 转到 `actual_pending/activation.rs`，但 R1 保持当前
  activation pre-suspend 行为，不能将它误算为八组闭合之一。
- R1 完成后，R2/R3/R4 不再需要编辑 `eval_context.rs`、共享 test module declaration 或
  root import。

### Actual-Pending

八组 consumer 必须使用各自 J1 prepared API：

1. prepare 操作及 owned wait future；
2. 第一次 poll 为 `Ready` 时留在当前 Actor segment，不 commit、不释放、不 reacquire；
3. 第一次 poll 为 `Pending` 时才经 E3 `await_if_pending` commit/释放；
4. ready 后在 current checkpoint、scheduler acquire 与 identity fence之后 finalize；
5. error、cancel 和 future drop 依靠既有 prepared/E3 RAII 收束，不复制状态机。

删除 `native_call_suspends`，并删除 production 中使用 `maySuspend`、`may_suspend`、binding
name、effect summary 或调用种类决定是否预释放 segment 的路径。不得加入语言级 `yield`、
Tokio `yield_now`、`nosuspend` 关键字或顺序 fallback。

同步例外必须保持同步：

- WebSocket send；
- serverStream 创建；
- `DbQuery`。

其中 activation-relative serverStream 的 route 可由 R1 预分配，但旧行为由
`activation.rs` 保留，终态归 R4。DB operation/transaction/lease 已由 O6 消费 E3，本节点
不得重做；只确认 `DbQuery` 没有 external wait。

`assembly_execution/mod.rs` 只允许 callback prepared consumer 所需的 crate-private 窄接线；
不得修改 public export、boundary error 或 callback/provider owner core。

## Checkpoint 精确语义

`checkpoint.rs` 只能组合 E1 已有 `ExecutionCheckpoint` /
`ExecutionCheckpointKind`：

- `FunctionEntry`：executable 与 block entry；
- `LoopCondition`：while/for condition或取得下一 item 前；
- `LoopBackedge`：继续下一 iteration 前；
- `GeneratedChunk`：array/map/object/construct 等 compiler-generated 有界 chunk；
- 本节点 actual-Pending await 进入和恢复使用窄 helper调用现有 checkpoint。

不得新增 kind、改变 instruction budget 公共语义或把 checkpoint 当成调度让出点。E2 已拥有
`LaneStart`、`LaneEnd`、`TailStart`，不得重复计数。

迁移必须替换同一路径原有 `add_instruction_units`、`check_cancelled`、
`poll_execution_budget` 组合，不能叠加两套计数/检查。result 必须记录反向搜索，并至少保留
一个明确 instruction-count assertion。

## Test-only current scope

ordinary/Actor 共用 `TestExecutionControl` 当前缺少 `execution_scope` / `derive_scope`。
只在 `scoped_execution.rs` 与必要的 test module 接线中使用现有
`ExecutionScope::request/derive` 闭合 fixture：

- 测试可观察 current scope、cancellation signals、effective deadline 和 owner；
- 保持既有 scripted clock、取消和 deadline 输入；
- 不给 production control 添加 fallback；
- 不用 request-root scope 冒充调用时 current scope。

## Test-first 与最低矩阵

启动后先用 selector listing 确认当前匹配数，再增加真实 RED；首次安全修改应在五分钟内开始，
此前不得重跑完整设计审计或昂贵 suite。最终 selector：

```text
f445h_e4r_spine
```

至少有 **10 个实际执行的 Rust 测试函数**，并穿过真实 linked evaluator /
`EvalContext::{exec_program_statement,eval_program_expr,eval_program_call}`、真实 E3 frame 与
prepared owner wait/finalize；不得只测 child helper。最低覆盖：

1. scripted clock 终止纯 CPU loop；
2. scripted clock 终止长 array/map/object 或 compiler-generated chunk，并断言 instruction
   count；
3. 三类 `Emit` send 的 Ready/Pending；
4. ordinary native Ready/Pending，以及 WebSocket send 同步例外；
5. `createFromStream` actual-Pending、drop 和配对 finalize；
6. legacy outbound unary Pending 与 serverStream Ready；
7. remote interface Ready/Pending；
8. callback Ready/Pending；
9. Actor dispatch Ready/Pending；
10. `DbQuery` 同步且不切 Actor segment。

测试不得根据 binding/effect 直接返回预期“是否 suspend”；必须以 future 第一次 poll 的
Ready/Pending、真实 Actor frame/store/segment 计数和 finalize/drop 计数断言。可以用参数化
fixture减少重复，但 listing 中必须至少有 10 个实际测试函数。

R2/R3 预分配测试文件只需能够由后继独立编辑，不得在本节点偷做 timeout/concurrent 终态测试。

## 验证与证据

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1-spine/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1-spine/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1-spine/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1-spine/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录 listing 与 execution 的实际非零数；少于 10 或零匹配不算完成。不得运行：

- 完整 `skiff-runtime-eval` suite；
- `program_db::tests::`、`db_actor_`；
- native/service/Actor/callback/service-db prepared owner selector；
- stable、live、network、MongoDB 或其它仓库测试。

result 必须记录：

- implementation commit 和 result commit；
- 每组 consumer 的 prepare/wait/finalize 入口；
- Ready/Pending/同步例外的实际断言；
- checkpoint 替换位置、instruction-count 证据和反向搜索；
- `native_call_suspends`、静态 `maySuspend` 预释放在 production 为零；
- timeout/concurrent/activation 三个 handoff 的精确状态；
- 实际测试数、check/fmt/diff 结果；
- 未决问题和证据失效条件。

## 停止条件

出现任一情况，立即停止并返回 `TASK_SCOPE_EXPANDED`，不得猜测、扩写或派子 Agent：

- 任一 J1 标记 heap/env-free 的 wait 实际捕获 caller heap/env/`EvalContext`；
- callback 接线需要修改 callback owner core，而不是 crate-private consumer/re-export；
- checkpoint 正确实现需要新增 E1 kind 或改变 instruction accounting 公共语义；
- test current scope 只能通过 production fallback 伪造；
- 正确实现需要修改本任务写集外 production owner、公共契约、语言语义、I6 或 host；
- 一次有界探查后仍有多个改变实现方向的未知量。

若 scoped RED 暴露 R1 写集内的单一明确缺陷，完成本节点；若暴露新的 owner/DAG 节点，报告精确
路径、调用链、失败证据、已有提交和最小拆分建议。不要把范围扩张包装成“顺手修复”。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r1-spine
branch   codex/p5-f445h-e4r1-spine
```

不得派子 Agent。先提交 production/tests implementation，再单独提交 result；返回两个 commit、
自验收矩阵、未决问题及 clean worktree 状态。不得 merge、rebase 或 push。

风险：高。R1 是共享实现检查点，不是稳定候选；开发自验收不替代后续 R5 combined acceptance。
R1 implementation 之后任何 root、prepared owner、E1/E3 或 test fixture 变化都会使后继 base
失效。
