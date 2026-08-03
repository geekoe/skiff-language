# Leaf Task: E1 std.task status/cancel 用户面（wire error 帧 + router 投影 + compiler surface + runtime 能力）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；Cancellation 节
  的 before-start cancellation、cancel/claim 同一 CAS、retention 过期后的稳定
  expired/unavailable；Observability 节的 task reference 可跨 request 恢复与
  owner scope 限制）。
- 用户面契约：`doc/reference/dispatch.md` §3（`std.task.TaskRef` /
  `std.task.status(ref) -> TaskStatus` / `std.task.cancel(ref) ->
  TaskCancelResult`；TaskStatus 8 kinds 与 TaskCancelResult 4 kinds 的逐字拼写；
  第一版只有 before-start cancellation）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration` 创建，位于集成分支 `dispatch-e-integration`
  commit `f7e5cf48`；main 上尚未存在，按批次父文档引用）。
- 已合并叶子：D1 `dispatch-d1-wire-leaf.md`（status/cancel wire 帧与 canonical
  kind 投影，无 error 帧）；D2 `dispatch-d2-router-leaf.md`（DurableTaskControl
  投影 status/cancel；transient store 失败投影为 expired + 独立 health 计数，
  记录为 D2 限制）；D3 `dispatch-d3-grammar-leaf.md`（`std.task.TaskRef`
  compiler-known/prelude 机制）；D4 `dispatch-d4-runtime-leaf.md`（TaskRef
  运行时值、canonical `skiff-task-v1:<owner>.<taskId>`、ambiguous acceptance
  处理风格）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`033391baa196ea4250e5276cc1a65a02e2fe555a`（main HEAD，共享主
  worktree 干净，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-e1-std-task`，branch `std-task-surface`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

实现阶段 E1：`std.task.status` / `std.task.cancel` 完整用户面（compiler surface +
runtime 能力 + wire error 帧补齐）：

1. wire（runtime-transport）：新增 `task.status.error` / `task.cancel.error`
   帧，错误码至少区分 `notFound`（TaskId 不可解析/owner scope 外）与
   `storeUnavailable`（暂时性）；更新 frames 语料与 codec 测试；旧 response
   帧保持。
2. router（DurableTaskFrameSink / DurableTaskControl）：status/cancel handler
   按新 error 帧区分 `notFound` 与 `storeUnavailable`（不再把 transient 投影为
   expired）；expired 保留给 retention 过期（权威文档的稳定 expired 结果）；
   health 计数相应调整；既有 `task_control_unit` 测试更新并补 error 分支。
3. compiler：`std.task` 命名空间增加 `status(ref: std.task.TaskRef) ->
   std.task.TaskStatus` 与 `cancel(ref: std.task.TaskRef) ->
   std.task.TaskCancelResult`（native/std surface，与既有 std 函数注册机制
   一致）；TaskStatus / TaskCancelResult 作为 std 类型（discriminated union，
   kind 拼写与 reference 逐字一致）；接入 compiler-known/prelude 机制（D3
   TaskRef 同款路径）；负例（参数类型错误、TaskRef 以外的参数）。
4. runtime：实现 std.task.status/cancel 能力——TaskRef 解码 →
   task.status.request/cancel.request → 响应 kinds 映射为用户可见 union 值；
   task.status.error/cancel.error：notFound → 稳定的 expired（按 reference
   拼写）；storeUnavailable → 平台错误抛给 caller（与 D4 ambiguous 处理风格
   一致）；TaskStatus/TaskCancelResult 运行时值可 recoverable 往返。
5. 测试：compiler surface 正/负例；runtime eval 能力映射（含 error 两分支）；
   transport corpus；router `task_control_unit` 扩展；既有测试全绿。

## 预检结论（只读，锚定 baseline 033391ba）

- wire：`runtime/transport/src/protocol/task.rs` 已有 `task.status.request/
  response` 与 `task.cancel.request/response` 帧（`TaskStatusWire` /
  `TaskCancelResultWire`，kind 拼写与 reference 一致），无 error 帧；
  `ActorTaskRuntimeErrorFrameHeader` 复用 submit error；`validate_error`
  硬编码 `task.submit.error`；direction 表无 status/cancel error。
- corpus：`runtime/transport/testdata/task-wire/frames.json` 18 帧；
  REQUIRED_FRAMES 同步在 `runtime/transport/tests/task_wire_corpus.rs`、
  `runtime/transport/tests/w_model_task_corpus.rs`、`runtime/tests/
  w_model_task_consumer.rs`、`router/tests/w_model_task_consumer.rs`；
  `router/tests/task_repair_direction.rs` 另维护方向表。
- router：`router/src/task/sink.rs` `DurableTaskFrameSink::handle_status/
  handle_cancel`：owner 未知 → expired response；store Err → expired response
  + `status_unavailable` / `cancel_unavailable` health 计数；store Ok →
  kind 投影（含 retention expired）。D2 限制记录在
  `dispatch-d2-router-leaf.md`。
- compiler：`compiler/core/src/prelude_registry.rs` `COMPILER_BUILTIN_TYPES`
  已有 `TaskRef`（symbol `std.task.TaskRef`，OpaqueHandle）；共享 native
  签名在 `artifact-model/src/native_signature.rs` `STD_NATIVE_SIGNATURES`，
  callable semantics 在 `STD_NATIVE_CALLABLE_SEMANTICS`；std 源码
  `std/*.skiff` 由 `compiler/source/src/prelude_registry/loading.rs` 加载并
  与共享签名严格比对（`compiler_std_native_declarations_match_shared_signatures`
  测试）。`std.task` 尚无源码文件与签名。
- runtime：类型计划 `runtime/model/src/type_plan/builtins.rs`
  `RuntimeBuiltinShape`（TaskRef leaf）；`runtime/linked-type-plan/src/type_plan/
  recoverable.rs` builtin fallback 只投影 leaf 节点；`RuntimeRecoverable
  ExpectedTypeNode` 已支持 Union/Record/LiteralString（shape-only 转换）。
  task submit 能力链路：`request-contract` `TaskSubmitControlRequest/Response`
  → `capability-context` `RequestCapabilityApi::submit_task` → host
  `RequestClient::submit_task`（`OutboundControlMessage::TaskSubmit`）→
  transport `control_mapper::encode_outbound_control_message` → eval
  `task_ops::submit_dispatch_call`（`await_actual_pending` +
  `CapabilityError::TaskSubmitRejected` 分类）。status/cancel 无对应能力。
- runtime native 注册表校验严格：`STD_NATIVE_SIGNATURES` 每个 binding 必须有
  `NativeRequiredContext`、`STD_NATIVE_CALLABLE_SEMANTICS`、runtime route
  （`runtime_shared_native_route_for_validation`）、handler 计数一致；
  `runtime/native/src/dispatch/tests.rs` 断言 route 集合。

## 关键实现决策（本叶子执行范围，不改设计语义）

- **wire 错误码**：新增闭合 `TaskControlRejectionCode`（`notFound` /
  `storeUnavailable`；`storeUnavailable` 为 transient），status/cancel error
  帧共用；不扩张 `TaskSubmitRejectionCode`（submit 语义已定）。
- **router 投影**：owner scope 外（TaskId 在当前 routing epoch 不可解析）与
  store `NotFound` → `task.status.error`/`task.cancel.error` code `notFound`
  （用户面稳定 expired 由 runtime 映射）；store transient/closed → code
  `storeUnavailable`；store Ok + `Expired`（retention 过期）→ 保持稳定
  `task.status.response` / `task.cancel.response` kind `expired`。health 新增
  `status_not_found` / `cancel_not_found` 计数，`status_unavailable` /
  `cancel_unavailable` 语义收窄为 storeUnavailable error 帧。
- **compiler surface**：`std/task.skiff` 新增
  `native function status(ref: TaskRef) -> TaskStatus` 与
  `native function cancel(ref: TaskRef) -> TaskCancelResult`；共享签名
  `Builtin("TaskRef")` 参数、`Builtin("TaskStatus")` /
  `Builtin("TaskCancelResult")` 返回；`COMPILER_BUILTIN_TYPES` 增加
  TaskStatus / TaskCancelResult（symbol `std.task.TaskStatus` /
  `std.task.TaskCancelResult`，Value kind），与 D3 TaskRef 同机制。
- **runtime 类型计划**：`RuntimeBuiltinShape` 增加 TaskStatus /
  TaskCancelResult，`leaf_node()` 返回 discriminated union
  （Record `{kind: LiteralString}` 分支，拼写与 reference 逐字一致）；
  recoverable expected builtin fallback 增加 Union 投影，保证
  TaskStatus/TaskCancelResult 运行时值 recoverable 往返。
- **runtime 能力链路**：`request-contract` 增加 `TaskStatusControlRequest/
  Response` 与 `TaskCancelControlRequest/Response`、
  `OutboundControlMessage::TaskStatus/TaskCancel`；`RequestCapabilityApi`
  增加 `status_task` / `cancel_task`；host `RequestClient` 走既有
  `send_control_request`，`finish_control_response` 按 target 区分
  `TaskSubmitRejectionCode` 与 `TaskControlRejectionCode`；新增 host
  `RuntimeError::TaskControlRejected` 与 capability
  `CapabilityError::TaskControlRejected`；eval 在 `eval_native_prepared_call`
  拦截 `std.task.status` / `std.task.cancel`（不进入 NativeDispatch
  context 机制），TaskRef 解码 → await 能力 → 按 kind JSON 经
  `runtime_from_wire_required_plan` 映射为用户可见 union 值；notFound →
  `{kind:"expired"}`，storeUnavailable → `ProviderUnavailable` 平台错误。
- 与兄弟节点无重叠：E2（actor task target）与 E3（e2e observability）均
  pending；本叶子不碰 scheduler/store 提交路径、不改 `doc/reference/` 与
  `doc/architecture/`，不改 `doc/implementation/**` 既有文件（本叶子文件为
  新增）。

## 禁止

- 不改 dispatch 提交路径（task_ops submit 语义、D4 已定）；不改
  `doc/reference/` 与 `doc/architecture/` 既有文档；不改
  `doc/implementation/**` 既有文件（本叶子为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。

## 实际写集（commit 后与交接报告一致）

```text
artifact-model/src/native_signature.rs          # STD_NATIVE_SIGNATURES + semantics（std.task.status/cancel）
compiler/core/src/prelude_registry.rs           # COMPILER_BUILTIN_TYPES TaskStatus/TaskCancelResult
compiler/core/src/prelude_registry/tests.rs
compiler/source/src/prelude_registry/tests.rs   # prelude identity snapshot 刷新
compiler/source/src/prelude_registry/tests/p5_f18a.rs
compiler/source/src/tests/dispatch_source_semantics.rs  # 正/负例
compiler/tests/dispatch_grammar.rs              # full pipeline 正/负例
router/src/task/sink.rs                         # status/cancel error 帧投影（notFound/storeUnavailable）
router/src/task/health.rs                       # status_not_found/cancel_not_found
router/src/health/{aggregator,counters}.rs
router/tests/task_control_unit.rs               # error 分支 4 例 + retention expired 例
router/tests/task_repair_direction.rs           # error 帧方向表
router/tests/w_model_task_consumer.rs
runtime/boundary/src/recoverable.rs             # pub is_canonical_task_ref_string
runtime/boundary/src/recoverable/tests.rs       # TaskStatus/TaskCancelResult recoverable 往返
runtime/capability-context/src/{capability_error,lib,outbound_control,request}.rs
runtime/eval/src/error.rs
runtime/eval/src/eval_context/actual_pending.rs # eval_native_prepared_call 拦截
runtime/eval/src/task_ops.rs                    # eval_task_control_native_call（notFound→expired、storeUnavailable→平台错误）
runtime/eval/src/task_ops/tests/canonical.rs    # 6 例能力映射
runtime/eval/src/{actor_dispatch/tests/prepared_operation,assembly_execution/ordinary/test_runtime,program_execution/tests/execution_scope}.rs
runtime/eval/tests/f445h_e4r_combined/{capability_harness,imports}.rs
runtime/host/src/capability_context/actor.rs    # RequestClient::status_task/cancel_task + 响应校验
runtime/host/src/error.rs                       # RuntimeError::TaskControlRejected
runtime/host/src/eval_capability_adapter/{actor,error,mod}.rs
runtime/host/src/host/router_session.rs         # task.status/cancel response/error 帧分发
runtime/host/src/host/router_session/tests/runtime_assembly_request{,.rs,fixture.rs}  # fixture identity 刷新
runtime/linked-type-plan/src/type_plan/{recoverable.rs,tests.rs}  # builtin union recoverable 投影
runtime/model/src/type_plan.rs
runtime/model/src/type_plan/builtins.rs         # RuntimeBuiltinShape TaskStatus/TaskCancelResult + union 计划
runtime/native-contract/src/required_context.rs
runtime/native/src/dispatch/{core.rs,mod.rs,task.rs,tests.rs}
runtime/native/src/error.rs
runtime/native/src/registry/table.rs
runtime/request-contract/src/{lib,outbound,outbound_control}.rs
runtime/request/src/{lib,outbound}.rs
runtime/tests/{h_task_parent_cut_corpus,w_model_task_consumer}.rs
runtime/transport/src/control_mapper.rs
runtime/transport/src/protocol.rs
runtime/transport/src/protocol/task.rs          # TaskControlRejectionCode + status/cancel error 帧 codec
runtime/transport/src/protocol/task/tests.rs
runtime/transport/testdata/task-wire/frames.json  # +4 error 帧 golden
runtime/transport/tests/{task_wire_corpus,w_model_task_corpus}.rs
std/api.yml                                     # std.task 模块 root 投影
std/task.skiff                                   # native function status/cancel
doc/implementation/dispatch-e1-std-task-leaf.md  # 本叶子
```

## 自验收矩阵（提交后与交接报告一致）

| 条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| wire error 帧 + 错误码（notFound/storeUnavailable） | `protocol/task.rs` `TaskControlRejectionCode` + `task.status.error` / `task.cancel.error` codec；direction 表补齐；corpus +4 golden 帧 byte-exact | 旧 response/request 帧 hex 未变；`rg "task.status.error"` 覆盖 codec/corpus/router/host | `cargo test -p skiff-runtime-transport`（全过）+ `cargo test -p runtime --test w_model_task_consumer --test h_task_parent_cut_corpus` + `cargo test -p skiff-router --test w_model_task_consumer --test task_repair_direction` |
| router 不再把 transient 投影 expired；notFound/storeUnavailable 分帧；expired 保留 retention | `router/src/task/sink.rs` handle_status/handle_cancel：owner 未知/store NotFound → notFound error；store Err → storeUnavailable error；store Ok Expired → 稳定 response expired；health 新增 status_not_found/cancel_not_found | `rg "status_unavailable"` 只出现在 error 帧计数路径 | `cargo test -p skiff-router --test task_control_unit`（18 PASS，含 transient/NotFound/unknown-owner/retention 4 新例） |
| compiler surface：std.task.status/cancel + TaskStatus/TaskCancelResult std 类型 | `std/task.skiff` native 声明；`STD_NATIVE_SIGNATURES`/`STD_NATIVE_CALLABLE_SEMANTICS`；`COMPILER_BUILTIN_TYPES`；`std/api.yml` std.task root | `compiler_std_native_declarations_match_shared_signatures` 全过；负例逐条命中 | `cargo test -p skiff-compiler-source --lib tests::dispatch_source_semantics`（14 PASS）+ `cargo test -p skiff-compiler --test dispatch_grammar`（5 PASS）+ `cargo test -p skiff-compiler-core --lib`（70 PASS） |
| runtime 能力：TaskRef 解码 → request → kinds 映射 union；notFound→expired；storeUnavailable→平台错误；recoverable 往返 | `task_ops::eval_task_control_native_call`；`RuntimeBuiltinShape::TaskStatus/TaskCancelResult` union 计划；recoverable builtin Union 投影；`is_canonical_task_ref_string` | 无第二套 status/cancel 协议；invalid TaskRef 不发 request | `cargo test -p skiff-runtime-eval --lib`（458 PASS，canonical +6 例）+ `cargo test -p skiff-runtime-boundary --lib`（172 PASS，union 往返 4 例）+ `cargo test -p skiff-runtime-linked-type-plan --lib`（19 PASS type_plan） |
| 全链路编译/既有测试 | 写集清单；host/router/compiler 全量 lib 测试 | `git diff --check` PASS；Cargo.lock 无 diff | `cargo check -p skiff-runtime-transport -p skiff-router -p skiff-compiler-core -p skiff-compiler-source -p skiff-compiler-lowering -p skiff-runtime-eval -p skiff-runtime-host` PASS；`cargo test -p skiff-runtime-host --lib`（427 PASS）；`cargo test -p skiff-router`（全过）；`cargo test -p skiff-test-runner --test test_service_flow`（16 PASS） |

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_e_integration`，并通知主 Agent `/root`。
