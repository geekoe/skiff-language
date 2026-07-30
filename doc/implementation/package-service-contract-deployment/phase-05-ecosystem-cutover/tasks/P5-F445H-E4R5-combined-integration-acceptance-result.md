# P5-F445H-E4R5 combined integration acceptance result

状态：

```text
FAIL
E4R_COMPLETE = NO
I6_UNBLOCKED = NO
TASK_SCOPE_EXPANDED = NO
```

`f445h_e4r_combined` listing 与 execution 都精确为 `5/5`，locked check、fmt、diff 和全部
production 结构反向检查也通过；但唯一完整 eval gate 的 lib binary 在
`f445h_e4r_spine_callback_pending_reacquires_before_finalize` 中 stack overflow 并以
`SIGABRT` 终止。完整 suite 因此 exit `101`，没有形成 395 个 lib tests 的完整结果汇总。

这是一项 blocking failure。E4R 不完整，I6 继续 blocked。本验收没有修改 production/tests，
也没有重跑 focused selector或尝试修复。

## 1. 冻结候选与验收身份

| 项 | 值 |
| --- | --- |
| 验收开始时 HEAD | `ce2ca5c329d6c971b975d65062fdb310602a8552` |
| 验收开始时 tree | `78efeb9943284465b448e5becf028e3a6e35dbcf` |
| 冻结 production/tests commit | `da49c17cb6e3c479ea649b936aab8614d3beface` |
| 冻结 production/tests tree | `0bdff47fad52aa52fea27bfd753db4bbf1213b6c` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance` |
| branch | `codex/p5-f445h-e4r5-acceptance` |

`ce2ca5c3` 的唯一 parent 精确为 `da49c17c`。`da49c17c..ce2ca5c3` 只有新增
`P5-F445H-E4R5-combined-integration-acceptance.md`，没有 production/tests/Cargo/lockfile
变化。验收开始时 `git status --short --branch` 只输出 branch header，无 tracked 或 untracked
写入；因此被验代码状态精确等于任务冻结候选，没有当前 worktree 内的在途写入。

## 2. 合同 gate 命令

所有 Cargo 命令使用合同指定的独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target
```

| # | 精确命令 | exit | 实际结果 |
| ---: | --- | ---: | --- |
| 1 | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list` | `0` | combined binary精确列出 `5 tests, 0 benchmarks`；其它三个 test binary均为0匹配。 |
| 2 | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture` | `0` | combined `5 passed / 0 failed / 0 ignored / 0 filtered`。 |
| 3 | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target cargo test -p skiff-runtime-eval --locked --no-fail-fast` | `101` | FAIL；lib target在一个 R1 callback actual-Pending test中 stack overflow/SIGABRT。其它 integration和doc targets继续完成。 |
| 4 | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target cargo check -p skiff-runtime-eval --locked` | `0` | PASS。 |
| 5 | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target cargo fmt --check` | `0` | PASS，无输出。 |
| 6 | `git diff --check` | `0` | PASS，无输出。 |

没有单独运行 R1/R2/R3/R4 focused selector；完整 eval suite只运行一次。

### 2.1 Combined listing 与 execution

listing 的五条实际测试为：

```text
f445h_e4r_combined::r1_case::f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable
f445h_e4r_combined::r2_timeout_case::f445h_e4r_combined_r2_timeout_statement_and_expression_execute
f445h_e4r_combined::r3_concurrent_case::f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute
f445h_e4r_combined::r4_activation_case::f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment
f445h_e4r_combined::r4_stream_case::f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end
```

execution 的每个 binary 汇总：

| binary | passed | failed | ignored | filtered |
| --- | ---: | ---: | ---: | ---: |
| `skiff_runtime_eval` | 0 | 0 | 0 | 395 |
| `catch_fixture_closure` | 0 | 0 | 0 | 4 |
| `f445h_e4r_combined` | 5 | 0 | 0 | 0 |
| `representation_wrap_consumer` | 0 | 0 | 0 | 6 |

五条 combined tests 均通过真实 public/integration入口：

- R1 通过 `ActorMethodExecutor` 进入 linked Actor evaluator，覆盖 first-Ready、
  actual-Pending/reacquire 和 checkpoint；
- R2 通过同一真实 Actor executable 执行 timeout statement/expression；
- R3 通过真实 Actor frame执行 concurrent statement/value；
- R4 activation 通过 compiler/artifact/linker/runtime assembly路径取得真实
  activation-relative operation，并验证 first-Ready不预释放 segment；
- R4 stream 通过 `Interpreter::exec_program_stream_for_in` 的真实 pending `next()`观察
  current child scope，并验证非-End cleanup initiation一次。

当前五个 case文件没有 `#[ignore]`、`should_panic`、`assert!(false)` 或改名。R5A记录的
R1-only基线是 `1 GREEN / 4 RED`；四条原失败断言/错误文本仍在对应 case中，当前依次转为
R2、R3、R4 activation、R4 stream GREEN。`2c0eab53` 只把 combined test拆为 test-only child，
其 changed paths全部位于 `runtime/eval/tests/f445h_e4r_combined*`；production diff为零。

### 2.2 完整 eval inventory 与失败结果

完整 suite 启动的 inventory 共 `411` tests：

| binary | inventory | passed | failed | ignored | filtered | 结果 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `skiff_runtime_eval` | 395 | 未产生汇总 | 未产生汇总 | 未产生汇总 | 未产生汇总 | `SIGABRT`；target FAIL |
| `catch_fixture_closure` | 4 | 4 | 0 | 0 | 0 | PASS |
| `f445h_e4r_combined` | 5 | 5 | 0 | 0 | 0 | PASS |
| `representation_wrap_consumer` | 6 | 6 | 0 | 0 | 0 | PASS |
| doc tests | 1 | 1 | 0 | 0 | 0 | PASS |

libtest在 abort 前打印 `running 395 tests`，但 stack overflow会终止整个进程，因而没有合法的
`passed/failed/ignored/filtered` summary。不能把 abort前已打印的零散 `ok` 行伪装成完整
395-test结果。其它 binaries合计形成 `16 passed / 0 failed / 0 ignored / 0 filtered` 的完整
汇总。

失败原文：

```text
thread 'actor_executor::tests::actor_concurrent_continuation::evaluator_actual_pending::callback_matrix::f445h_e4r_spine_callback_pending_reacquires_before_finalize' has overflowed its stack
fatal runtime error: stack overflow, aborting
error: test failed, to rerun pass `-p skiff-runtime-eval --lib`

Caused by:
  process didn't exit successfully: `.../skiff_runtime_eval-d47a3a59d9e65bfd` (signal: 6, SIGABRT: process abort signal)

error: 1 target failed:
    `-p skiff-runtime-eval --lib`
```

为在不重跑测试的前提下记录 abort target的实际 inventory，验收后只对已构建
`skiff_runtime_eval-*` executable执行一次 `--list` 并聚合名称；该 read-only listing
exit `0`，没有执行 test：

```text
INVENTORY total=395 spine=23 timeout=11 concurrent=11 stream=22
```

combined位于独立 integration binary，inventory为5。因此合同下限全部存在且非零：

| selector | inventory |
| --- | ---: |
| `f445h_e4r_spine` | 23 |
| `f445h_e4r_timeout` | 11 |
| `f445h_e4r_concurrent` | 11 |
| `f445h_e4r_stream` | 22 |
| `f445h_e4r_combined` | 5 |

source反向搜索在 `runtime/eval/src` 与 `runtime/eval/tests` 中发现0个 `#[ignore]`，但这不能替代
aborted lib target缺失的实际运行汇总。

## 3. Blocking failure、调用链与唯一 owner

唯一 blocking failure位于：

```text
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/
  evaluator_actual_pending/callback_matrix.rs:533
```

该 scenario的静态 production调用链为：

```text
callback_matrix.rs:533
  -> EvalContext::exec_program_executable
  -> eval_context.rs:1139 CallbackCapability branch
  -> eval_context/actual_pending.rs:121 eval_callback_interface_call
  -> assembly_execution::prepare_callback_capability_call
  -> PreparedCallbackInvocation::wait
  -> Interpreter::call_program_executable_with_self
  -> owner-local linked callback executable
  -> std.time.sleep(20) native ExternalWait
  -> outer await_actual_pending / ActorExecutionFrame::await_if_pending
  -> caller-heap finalize after reacquire
```

进程 abort没有提供 backtrace，因此不能诚实声称具体溢出 frame已定位；可以精确定位的是上述
R1 callback actual-Pending完整场景和它的 production route。该测试的语义在 R1实现提交中建立，
`aa464c62` 只做 test module拆分；R2/R3/R4 production changed-path列表均不包含 callback
consumer、prepared callback owner或 `program_execution.rs`。

**唯一责任叶子：P5-F445H-E4R1 callback actual-Pending integration 的 full-suite stack-safety
closure**，覆盖上述 failing test与调用链。它不是 R2 timeout、R3 concurrent或 R4
stream/activation owner。本验收 task不是修复 owner。

当前证据失效范围：

- 完整 `skiff-runtime-eval` gate失效；
- R1 spine `23/23` 完整执行证据失效；
- 九组 actual-Pending 的 combined acceptance失效，具体 blocker是 callback Pending组；
- T05–T12虽有完整 inventory与结构映射，但“完整 suite全部通过”的执行证据整体失效；
- `E4R_COMPLETE` 与 `I6_UNBLOCKED` 均不得签发。

该 blocker没有显示需要修改公共 owner或扩张设计；因此当前 verdict不是
`TASK_SCOPE_EXPANDED`。若修复调查证明必须改变 E1/E2/E3/O1–O6公共 owner，再由修复叶子按合同
升级 scope，不能由本只读验收预判。

## 4. production 语义与结构反向检查

除完整 suite blocker外，合同的12项静态/只读检查均通过。

### 4.1 Diagnostic、预释放和静态 effect

1. 对 `runtime/eval/src` 与 `runtime/eval/tests` 搜索
   `F445H-E4 evaluator integration is required`：**0 hits**。timeout statement/expression与
   concurrent statement/value四个冻结 diagnostic均已消失。
2. `native_call_suspends`：共4 hits，全部为 `#[cfg(test)]`：
   `eval_context.rs:83` 的 test-only re-export，以及 `actor_executor.rs:827,837,855` 的旧 unit
   expectation。production function/decision为0。
3. `suspend_actor_segment|resume_actor_segment`：全 `runtime/eval/src` 与 integration tests
   **0 hits**。`eval_context` child中的 `.resume(` 唯一命中是
   `concurrent.rs:210` 对 E3 Actor lane bridge的合法 resume，不是 actual-Pending预释放 helper。
4. `maySuspend`为0 hits；`may_suspend`为65 hits，路径分类为：
   - integration/unit fixture中的 artifact字段；
   - gateway/link projection检查及 production builder中的 executable ABI/artifact metadata；
   - test-only `legacy_native_call_expected_to_suspend`。
   没有一个 hit参与 Actor segment释放。production `binding_key` 在
   `eval_context.rs:1532` 只识别 `std.file.createFromStream` composite preparation；其真实
   `ExternalWait` 仍在 `actual_pending::await_operation` 上 first-poll判定。effect summary不参与
   release。
5. 语言级 `yield`为0，`nosuspend|no_suspend`为0，`sequential.*concurrent`为0。
   `yield_now`共13 hits，全部位于 unit/test module；production evaluator/concurrent/stream
   组合为0。concurrent production没有 fallback；唯一 `fallback` 命中是测试名
   `...fail_closed_without_fallback`。

### 4.2 Timeout owner与internal carrier

`eval_context/timeout.rs` 的 statement/expression均：

- 从 parent clone精确 `derive_timeout_child`；
- 保留 child `ExecutionScope`和 owner context；
- 只对 `ScopeTerminalCarrier::is_owned_by(child_scope)` 成立的 local owner物化
  `TimeoutError`；
- inherited carrier在 owner checkpoint仍不归当前 wrapper时原样返回；
- ancestor cancel不物化为 ordinary exception。

`ScopeTerminalCarrier`只持有 `ExecutionScopeTerminal`，visibility为 eval crate内部。
`RuntimeError::ordinary_payload` 对 `RuntimeError::ScopeTerminal(_)` 明确返回 `None`，
diagnostic/wire投影同样不导出它。只有精确 timeout wrapper在确认 owner后创建 request-local
`UserException`；没有把 internal carrier作为 ordinary payload或 wire error传播。

### 4.3 Current scope与cleanup

以下三个 child都从调用时的精确 execution control取得 `ExecutionScope`：

- `program_stream/current_scope.rs`
- `program_invocation/current_scope.rs`
- `assembly_execution/async_stream_cancel/current_scope.rs`

它们使用 `cancellation_signals()`、`effective_deadline()`和
`terminal_at(Instant::now())`。两个 consumer传给 `next_with_cancellation` 的额外 token iterator
均为 `std::iter::empty()`。针对这些路径搜索
`request-root|root-scope|generic token|cancellation_token()`为0，没有 request-root/generic
request-token fallback；scope不可取得时 fail closed。

`StreamConsumerCleanup` 的唯一 disarm协议在
`runtime/capability-context/src/stream_cleanup.rs`：

- `reached_end()`先记录 runtime natural End，再 `disarm_after_end()`；
- `disarm_after_end()`明确要求 `has_reached_end()`；
- 其它 return/break/error/drop路径让 guard drop，只同步发起本地 cancel/cleanup；
- supervised owner通过共享 state只接受一次 finalization/cleanup claim。

`program_stream.rs`和两个 invocation loop只在真实 `StreamPoll::End`调用 `reached_end()`。
binary HTTP payload中的逻辑 `HttpBoundaryResponseStreamEvent::End`直接退出而不 disarm，因而仍
发起本地 cleanup。异常/drop只承诺本地 initiation，不等待或保证远端 ack。

### 4.4 DB O6与同步例外

DB operation/transaction/lease全部继续通过唯一
`runtime/eval/src/program_db/wait.rs::await_operation` 进入 E3
`ActorExecutionFrame::await_if_pending`。从 R1 base到冻结候选，对
`program_db.rs`、`program_db/**`和 `db_eval.rs` 的 diff为零；E4R没有复制 DB wait、
operation、transaction或 lease owner。

`DbQuery` route是：

```text
eval_context.rs LinkedExprIr::DbQuery
  -> program_db.rs::eval_program_db_query_value
  -> db_eval.rs::eval_query_value
  -> materialize query IR
```

该 route没有 DB store operation、`wait::await_operation`或 `ExternalWait`。
`f445h_e4r_spine_db_query_is_first_poll_ready_and_keeps_actor_segment`仍在 inventory中，作为同步
例外；完整 suite abort使其不能替代完整 R1 `23/23` gate。

### 4.5 写集边界与root拆分

R2/R3/R4/R4S implementation commits的实际 changed paths分别为：

- `88209415`：`eval_context/timeout.rs`和 timeout tests；
- `57422ab1`：`eval_context/concurrent.rs`和两个 concurrent test paths；
- `bb64a182`：activation child、program stream/invocation/provider current-scope consumers及
  对应 tests；
- `e39e242f`：`async_stream_cancel.rs`与 private
  `async_stream_cancel/activation_relative.rs`的等价抽取。

从 R1 base `b1faea53` 到候选 `da49c17c`，以下公共 owner diff精确为空：

```text
runtime/eval/src/eval_context.rs
runtime/eval/src/program_execution.rs
runtime/eval/src/env/concurrent_scheduler.rs
runtime/eval/src/actor_executor.rs
runtime/eval/src/program_db.rs
runtime/eval/src/program_db/**
runtime/capability-context/**
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/service_dispatch/**
```

因此 R2/R3/R4没有回改 root、E1/E2/E3或 O1–O6公共 owner。

长 root的新增责任已进入 child：

- `eval_context.rs`只声明/转发 `checkpoint`、`actual_pending`、`timeout`、`concurrent`；
- timeout为126行，concurrent为303行，activation consumer为52行；
- current-scope组合分别为74/102/58行；
- 1997行的 `async_stream_cancel.rs` 已把264行 activation prepared owner抽入 private child；
- combined root为2行，最大 test child为395行；
- actual-Pending test root为77行，各 group进入独立 child。

R1 test structure commit `aa464c62` changed paths全部位于
`actor_executor/tests/.../evaluator_actual_pending*`；R5AS `2c0eab53` changed paths全部位于
`runtime/eval/tests/f445h_e4r_combined*`。两者 production diff均为零。

## 5. T05–T12 与 actual-Pending inventory映射

以下是完整 binary中实际存在的测试/production路径映射。因为 lib target abort，这一节只证明
inventory和路径完整，不声称这些 tests在本次完整 gate中全部通过。

| ID | 实际 tests | production路径 | 本次状态 |
| --- | --- | --- | --- |
| T05 | `...timeout_statement_normal_return_max_duration_and_parent_restore`、`...timeout_expression_value_uses_child_and_restores_parent`、`...timeout_zero_millis_statement_and_expression_use_real_root_arms` | `eval_context.rs` timeout root → `eval_context/timeout.rs` child scope | inventory存在；combined R2通过；完整执行证据失效 |
| T06 | `...timeout_local_owner_inner_catch_misses_outer_catch_hits_and_continues`、`...timeout_ordinary_catch_rethrow_preserves_materialized_owner` | `timeout.rs::materialize_owned_timeout` → request-local exception/catch/rethrow | inventory存在；完整执行证据失效 |
| T07 | `...timeout_nested_inner_earlier_materializes_inner_only`、`...nested_outer_earlier_passes_inner_and_materializes_outer`、`...equal_absolute_deadline_materializes_outer_only` | nested timeout wrappers + `is_owned_by` | inventory存在；完整执行证据失效 |
| T08 | `...timeout_inherited_request_deadline_is_not_extended_materialized_or_caught` | inherited `ExecutionScopeTerminal`透传 | inventory存在；完整执行证据失效 |
| T09 | `...timeout_ancestor_cancel_wins_same_poll_and_lifecycle_returns_zero`、`...timeout_future_drop_keeps_parent_scope_and_zero_lifecycle` | E1 scope priority/lifecycle + timeout wrapper | inventory存在；完整执行证据失效 |
| T10 | `...checkpoint_scripted_clock_terminates_pure_cpu_for_loop`、`...terminates_generated_array_chunk`、`...checkpoint_instruction_count_replaces_legacy_accounting`（含 literal/generated chunk accounting）、`...shared_test_control_exposes_current_and_derived_scope` | `eval_context/checkpoint.rs`与真实 evaluator entry/chunk/backedge | inventory存在；完整执行证据失效 |
| T11 | `...concurrent_serial_dependency_gates_and_runs_the_complete_block`、`...value_tail_waits_for_fence_and_hands_heap_value_to_parent`、`...same_turn_errors_choose_source_order`、`...outer_terminal_wins_over_same_turn_lane_completion`，以及3条 `...concurrent_actor_{ready,pending,error}...` | `eval_context/concurrent.rs` → E2 scheduler → E3 Actor bridge | inventory存在；combined R3通过；完整执行证据失效 |
| T12 | `...winner_stops_unstarted_lane_and_drops_running_loser`、`...loser_late_heap_write_isolated_and_outer_scope_restored`、Actor error/parent restore；22条 stream inventory中的 natural End、break/return/error、future drop、logical End、current child scope、provider publication | concurrent winner/cleanup；三个 stream `current_scope.rs`；`StreamConsumerCleanup` | inventory存在；combined R4两条通过；完整执行证据失效 |

九组 actual-Pending/同步例外的实际 inventory：

| # | group | 实际 tests |
| ---: | --- | --- |
| 1 | Emit projected | `...emit_projected_ready_keeps_actor_segment`、`...pending_reacquires_before_completion` |
| 2 | Emit detached | `...emit_detached_ready_keeps_actor_segment`、`...pending_cuts_actor_segment_once` |
| 3 | Emit canonical wire | `...emit_canonical_wire_ready_completes_first_poll`、`...pending_resumes_same_send_once` |
| 4 | remote interface | `...remote_interface_ready_keeps_actor_segment`、`...pending_reacquires_before_finalize` |
| 5 | callback | `...callback_ready_keeps_actor_segment`、`...callback_pending_reacquires_before_finalize`；后者是本次 blocker |
| 6 | Actor dispatch | `...actor_dispatch_ready_keeps_actor_segment`、`...pending_reacquires_before_finalize` |
| 7 | legacy outbound | `...legacy_unary_pending_and_server_stream_ready`，同时覆盖 serverStream同步例外 |
| 8 | native/composite | native Ready/Pending、WebSocket sync error、DbQuery first-poll Ready、`createFromStream` Pending success/drop |
| 9 | activation-relative | activation unary Ready/Pending/failure import与 activation serverStream同步 Ready |

WebSocket、legacy/activation serverStream与 `DbQuery`均保持同步、不触发静态预释放；但 callback
Pending crash使九组 combined closure不成立。

## 6. Blocking、non-blocking、warning与残余风险

### Blocking

- 完整 eval命令 exit `101`；
- lib target中的 R1 callback actual-Pending test stack overflow/SIGABRT；
- 因此 `E4R_COMPLETE`、完整 T05–T12执行覆盖与 `I6_UNBLOCKED` 全部不可签发。

### Non-blocking

Cargo输出的 warning均未被本 gate配置为 deny，且不位于本次 failing call chain：

- `skiff-compiler-source`：既有 unused import/dead-code类，共报告27 warnings；
- `skiff-runtime-linker`：既有 dead-code类，共报告32 warnings；
- `runtime/eval/src/assembly_execution/service_error_channel.rs`：
  `PlatformBuiltinErrorIdentity` match的既有 unreachable-pattern warning；
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`：test-only unused
  `LinkedCallTarget` import。

combined execution为0 ignored；其余完成的 binaries同样为0 ignored。全 eval source/tests
没有 `#[ignore]`。lib target因 abort没有运行时 ignored summary，不能写成一个伪造数字。

### 未运行与残余风险

依任务禁止面，没有运行或访问：

- stable instance；
- live selector或本地服务；
- network；
- MongoDB；
- 长压力/loop-risk stress；
- 其它仓库。

因此这些面保留未验证风险；它们不是本次 FAIL的原因。最直接的残余风险是 lib abort后其余
395-test inventory没有完整执行汇总。

## 7. 交付

唯一 tracked写入是本 result文件。没有修改 production、tests、fixture、Cargo、manifest、
lockfile或其它文档；没有 merge、rebase、push，也没有派子 Agent。result commit hash由交付消息
记录。result提交后再次检查 worktree为 clean。
