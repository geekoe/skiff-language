# P5-F445H-E4R1 evaluator spine, actual-Pending and checkpoint result

状态：`READY_FOR_E4R2_E4R3_E4R4`。

R1 已在 implementation commit
`b1faea534654c2ee2109f444a6cad6b1168b8445` 闭合 evaluator shared spine、八组
actual-Pending、owner-aware checkpoint 和 test-only current scope fixture。本文件所在提交为
独立 result commit；其实际 hash 随交付回报记录。

本结果只解除 R2 timeout、R3 concurrent 和 R4 stream/activation 三个并行叶子，不代表
E4R、F445H 或 Phase 05 完成。三个后继必须从上述 implementation commit 建立，不得以 result
文档提交代替 frozen production base。

## 1. Test-first 证据

- 修改前
  `cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --list`
  为 `0 tests`。
- 首个 scoped RED 使用真实 linked evaluator 和 scripted clock；local deadline 已越过时，
  evaluator 仍返回 Continue，证明纯 CPU 路径没有消费 E1 checkpoint。
- 最终 selector listing 为 `23 tests, 0 benchmarks`；execution 为
  `23 passed; 0 failed`。没有用 helper-only test 充当 selector 数量。

所有实际等待测试均进入真实
`EvalContext::{exec_program_statement,eval_program_expr,eval_program_call}` 路径，并使用真实
E3 `ActorExecutionFrame` lease 状态判断首次 poll 的 Ready/Pending。

## 2. Root、checkpoint 与 fixture 终态

`eval_context.rs` 现在只声明 `checkpoint`、`actual_pending`、`timeout`、`concurrent` child，
并把 Emit、interface/call、timeout/concurrent arm 薄转发到对应 child。原 root
`suspend_actor_segment` / `resume_actor_segment` helper 已删除。

`checkpoint.rs` 只组合 E1 既有 kind：

| evaluator 位置 | checkpoint |
| --- | --- |
| executable entry、每个 block entry | `FunctionEntry` |
| for-in array/map 取得下一项前 | `LoopCondition` |
| for-in 继续下一 iteration 前 | `LoopBackedge` |
| statement/expression、match arm、array/object/map item | `GeneratedChunk` |
| actual-Pending wait 进入与恢复后 | 窄 `GeneratedChunk(0)` checkpoint |

同一路径原有 `add_instruction_units`、`check_cancelled`、
`poll_execution_budget` 组合已被替换，没有双重计数。真实 evaluator instruction-count test
断言一个 executable + block + statement + array expression + 两个 literal 精确计为 `6`。
scripted clock 另分别终止纯 CPU for-in 和 generated array chunk。

test-only `TestExecutionControl` 现在持有真实 current `ExecutionScope`：

- request fixture 使用 `ExecutionScope::request`；
- derived fixture 使用当前 scope 的 `derive`，没有用 request-root 冒充 child；
- test 实际断言 request/child nesting、effective deadline、deadline owner
  (`Request` / 带 site 的 `Scope`) 和 request cancellation 向 child signals 的继承。

## 3. 八组 actual-Pending consumer

统一 wait 入口 `actual_pending::await_operation` 在 wait 前后执行 checkpoint，并把 owned future
交给 E3 `ActorExecutionFrame::await_if_pending`。第一次 poll Ready 不 commit、不释放；第一次
poll Pending 才释放，完成后由 E3 acquire/fence，再进入 consumer finalize。

| consumer | prepare | wait | finalize / 同步 Ready |
| --- | --- | --- | --- |
| Emit projected internal | `project_runtime_item` | `send_internal_with_cancellation` + `await_actual_pending` | send future result |
| Emit detached internal | clone 到独立 item heap | `send_internal_with_cancellation` + `await_actual_pending` | send future result |
| Emit canonical wire | `runtime_to_wire_required_plan` | `send_with_cancellation` + `await_actual_pending` | send future result |
| remote interface | `prepare_outbound_service_operation` | `ExternalWait::into_wait` + `await_actual_pending` | `completed.finalize(heap, env)`；`Ready` 直接返回 |
| callback capability | `prepare_callback_capability_call` | `prepared.wait` + `await_actual_pending` | `completed.finalize(heap)` |
| Actor dispatch | `prepare_actor_method` | `prepared.into_wait` + `await_actual_pending` | `completed.finalize(heap)` |
| legacy dependency | `prepare_outbound_service` | `ExternalWait::into_wait` + `await_actual_pending` | `completed.finalize(heap, env)`；serverStream `Ready` 直接返回 |
| native | `prepare_resolved_native_call` | `ExternalWait::into_parts().0` + `await_actual_pending` | native finalizer `finalize(outcome, heap)`；`Ready` 直接返回 |

`std.file.createFromStream` 的组合路径先建立 supervised stream producer，再 prepare native
operation；native owned wait 在 producer consumer 内经同一 actual-Pending helper，native
finalizer先结算结果，外层 `exec_prepared_native_stream_producer_arg` 继续拥有 producer
success/error/drop cleanup。没有复制 prepared/E3 状态机。

`assembly_execution/mod.rs` 唯一变化是 callback consumer 所需的 crate-private
`prepare_callback_capability_call` 窄 re-export；没有公共 export 或 provider owner 变化。

## 4. Ready、Pending 与同步例外断言

- Emit：
  - detached 和 projected 两类各自验证 Ready 保留 lease、Pending 首 poll 释放并在完成后
    reacquire；
  - canonical bounded sink 验证一项时 `starts=1/completions=1`；两项时先达到
    `starts=2/completions=1`，消费首项后才到 `2/2`，证明第二个同一 send 首 poll Pending，
    且只恢复一次。
- ordinary native：`sleep(0)` Ready 保留 segment；`sleep(50)` 首 poll Pending 释放并在
  return finalize 前 reacquire。
- WebSocket send：同步 capability error 不切 segment。
- `createFromStream`：Pending success 为 `starts=1/completions=1/drops=0`，且 native return
  finalize 前已 reacquire；丢弃 evaluator future 为 `completions=0/drops=1`，只结算一次。
- legacy outbound：unary Pending 释放/reacquire；serverStream setup 即使 outbound fixture
  没有 response 也同步 Ready 且不切 segment。
- remote interface：buffered Ready 不切；Pending start 只发生一次，response 后在
  decode/finalize 前 reacquire。
- callback：真实 RuntimeAssembly + callback carrier 的 Ready/Pending 均通过；Pending 在
  caller-heap finalize 前 reacquire。
- Actor dispatch：Ready/Pending 各 start 一次；Pending 在 return finalize 前 reacquire。
- `DbQuery`：第一次 poll 即 `Ready(Return)`，只物化 query IR，不切 segment。

## 5. 反向搜索

对 `eval_context.rs` 和其 production child 搜索
`add_instruction_units|check_cancelled|poll_execution_budget` 为零；checkpoint call site 只使用
E1 既有 `ExecutionCheckpoint` / `ExecutionCheckpointKind`。

production 的 native dispatch 已不读取 callable binding、effect summary、`maySuspend` 或
`may_suspend` 决定预释放；`STD_NATIVE_CALLABLE_SEMANTICS` 也不再进入 production
`eval_context`。`native_call_suspends` production function 已删除。

当前同名反向搜索有且只有 test compilation compatibility：

- `eval_context.rs` 的 `#[cfg(test)]` re-export；
- `actual_pending/tests.rs` 中供既有、写集外 actor unit tests 编译的 legacy expectation。

它们不进入 production cfg，也不参与本节点 23 个真实 Ready/Pending 断言；因此 production
静态预释放路径为零。全 `runtime/eval/src` 的其它 `may_suspend` 命中是 ABI 一致性检查或 test
fixture 字段，不是 Actor pre-release decision。

## 6. 后继 handoff

- **R2 / timeout**：`timeout.rs` 已拥有 statement/expression 两个 arm，当前稳定返回原同义
  `F445H-E4 evaluator integration is required ...` fail-closed diagnostic；
  `evaluator_timeout.rs` 已预声明。R2 只编辑这两个 child。
- **R3 / concurrent**：`concurrent.rs` 已拥有 statement/value 两个 arm，当前稳定返回原同义
  fail-closed diagnostic；`concurrent/tests.rs` 和 `evaluator_concurrent.rs` 已预声明。
  R3 不编辑 root/module declaration。
- **R4 / stream + activation**：`actual_pending/activation.rs` 原样保留 test-effect dispatch 和
  activation-relative pre-suspend/resume，未把第九组误算为本节点闭合；serverStream 行为及
  `async_stream_cancel.rs` 均未改。R4 原子替换 activation child，并拥有其冻结 stream surface。

R2/R3/R4 共同 frozen production base：
`b1faea534654c2ee2109f444a6cad6b1168b8445`。

## 7. 最终验证

在独立 target
`/Users/geek/workspace/skiff-p5-f445h-e4r1-spine/build/cargo-target` 执行：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --list` | PASS，23 tests |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --nocapture` | PASS，23/23 |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

check 只报告基线已有的 compiler/linker dead-code/unused warnings、写集外 ordinary tests 的
unused import，以及写集外 service error channel 的 unreachable pattern；本节点写集无新增
warning。按任务约束没有运行完整 eval suite、DB prepared selector、stable/live/network、
MongoDB 或其它仓库测试。

## 8. 未决问题与证据失效条件

本节点写集内无未决 blocker。activation 第九组、timeout 和 concurrent 是明确的后继冻结
中间态，不是 R1 遗漏。

以下变化会使本结果相应证据失效，必须由责任叶子重跑 scoped matrix，并最终由 R5 combined
acceptance 汇总：

- 修改 `eval_context.rs` root/module declaration、`checkpoint.rs`、
  `actual_pending.rs` 或 shared `TestExecutionControl`；
- 修改 E1 checkpoint accounting、E3 `await_if_pending`，或任一 J1 prepared owner 的
  wait/finalize ownership；
- 修改 callback narrow re-export 或 consumer/provider boundary；
- R2/R3/R4 合入各自终态后，继续用本 R1 selector 结果声称 E4R combined closure。
