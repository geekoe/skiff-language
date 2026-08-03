# Leaf Task: D4 runtime durable submit（runtime eval/host + TaskRef 运行时值）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；Submission And
  Visibility 的提交顺序与 definite rejection / ambiguous acceptance 区分、Runtime
  submission side 职责、execution image 冻结、payload 是 recoverable boundary；
  TaskRef 只承载 TaskId + owner scope，可跨 request 恢复）。
- 用户面契约：`doc/reference/dispatch.md`（`dispatch` 是表达式，返回
  `std.task.TaskRef`；单独成行按 expression statement 丢弃；after/at 语义；
  参数满足 recoverable boundary；db transaction 内禁止由 compiler 静态检查）。
- 批次父节点：`doc/implementation/dispatch-d-batch.md`（集成 Agent
  `/root/dispatch_d_integration`；本批次节点 D4）。
- 已合并叶子：D1 `dispatch-d1-wire-leaf.md`（timing 三态、taskRef、错误码
  definite/transient 分类、status/cancel 帧、`request.start` taskAttempt 头）；
  D2 `dispatch-d2-router-leaf.md`（router 控制面 durable create / TaskId 幂等 /
  storeUnavailable 暂时性 / unsupportedTarget definite、attempt 走普通
  `request.start`）；D3 `dispatch-d3-grammar-leaf.md`（dispatchSubmit metadata
  timing `{kind, expr?}`、语句位置 `StmtIr::Dispatch`、表达式位置带 metadata 的
  Call 返回 TaskRef）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`5cc1595c84bf132f97168705e904b4280a23665d`
  （`dispatch-d-integration` HEAD，已 `git rev-parse` 确认）。
- worktree：`/Users/geek/workspace/skiff-d4-runtime`，branch
  `runtime-durable-submit`。
- 集成 Agent：`/root/dispatch_d_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

实现阶段 D4：runtime durable submit（runtime eval/host + 必要 native/std 接线；
不改 syntax、不改 router、不改 task-control、不改 `doc/reference/` 与
`doc/architecture/`）：

1. runtime task 提交能力重写（`runtime/eval/src/task_ops.rs` 与相关模块）：
   - 求值顺序：receiver / 参数各求值一次 → timing 表达式一次（D3 plan 的
     `timing.expr` 指针，类型 Duration/Instant）→ recoverable encode →
     生成 TaskId（提交前生成，幂等重试复用）→ 发 `task.submit.request`（携带
     timing 三态）→ 等待 durable ack。
   - `task.submit.response` → 构造 `std.task.TaskRef` 运行时值（opaque，内容承载
     wire taskRef 的 canonical 编码，可 recoverable 往返）。
   - `task.submit.error`：definite 码（invalidTiming / payloadInvalid /
     quotaExceeded / rejected / unsupportedTarget）→ 以明确平台错误抛给 caller，
     不产生 task；storeUnavailable（暂时性）→ 按权威文档 ambiguous acceptance
     处理：内部用同一 TaskId 有界重试（退避/次数限制），仍失败则向 caller 抛
     “结果不确定”平台错误（不得谎报成功，也不得生成第二个 TaskId）。
   - 语句位置（`StmtIr::Dispatch`）与表达式位置（带 dispatchSubmit metadata 的
     Call）共用同一提交实现；表达式位置返回 TaskRef，语句位置丢弃。
   - 移除 runtime 侧旧易失提交路径（被 durable submit 取代的部分；删除清单见下）。
2. `std.task.TaskRef` 运行时值：类型接入已有 compiler-known handle
   （`std.task.TaskRef`，OpaqueHandle）；recoverable encode/decode（可进 DB
   stored field / persistent payload 的边界类型）。
3. 测试（reference 矩阵 15–16 的 runtime 部分 + 提交语义）：
   - dispatch 语句与表达式各产生一次 `task.submit.request`，payload / timing
     正确（immediate/after/at 三态），receiver / 参数 / timing 各只求值一次
     （副作用计数：嵌套 dispatch 参数计数提交次数）；
   - 同一 TaskId 在 transient 重试中复用；definite rejection 抛错且无 task；
   - TaskRef 往返（wire taskRef ↔ 运行时值 ↔ recoverable encode）；
   - 表达式位置返回 TaskRef、语句位置不返回值；
   - 既有 runtime eval/host 测试与 test-runner service flow 保持全绿。

## 预检结论（只读，锚定 5cc1595c）

- D3 plan：`compiler/lowering/src/function_lowering.rs` 的 `lower_task_call` 把
  timing 表达式 lower 进 body 表达式表，`dispatchSubmit` metadata 增加
  `timing` object（`kind` 三态 + after/at 的 `expr:<u32>`）；`compiler/core` 的
  `validate_task_timing_metadata` 已校验形状。
- runtime 现有提交：`task_ops::submit_task_statement` 只处理语句位置、固定
  `TaskSubmitTimingControl::Immediate`、`task_id: None`、`submit_task` 能力返回
  `()`（response taskRef 被丢弃）。表达式位置（`LinkedExprIr::Call`）没有
  dispatchSubmit 分支，会按普通调用执行。
- 执行侧（`execute_runtime_assembly_task_target` / `runtime/request/src/task_execution.rs`）
  是 D2 之后 durable attempt 的普通 `request.start` 执行车道，**保留**；D4 只移除
  旧的易失提交路径。
- 错误链路：`task.submit.error` 帧经 host `router_session.rs`
  `dispatch_control_error` → `OutboundResponse::Error(ResponseError)` →
  `capability_context/actor.rs` `finish_control_response` → host
  `RuntimeError::ProviderUnavailable`（wire code 丢失）。D4 需要保留
  `TaskSubmitRejectionCode` 到 eval 侧。
- TaskRef 运行时类型：`RuntimeValue` 无 TaskRef 变体；`std.task.TaskRef` 作为
  linked builtin 目前落到 `RuntimeTypeNode::Unknown` / recoverable
  `Unresolved`。`RuntimeBuiltinShape::of_name` + `leaf_node` 是 builtin 类型计划
  的统一入口（linked `builtin_node`、native `native_builtin_plan`、
  recoverable `recoverable_expected_builtin_node` 都走它）。
- recoverable 编解码：`RecoverableBoundaryCodec` 的
  `precheck_expected_type_with_policy` 是 encode/decode 两侧的期望类型门；
  String 节点可承载 canonical taskRef，decode 不需要新状态。
- 测试 harness：`runtime/eval/src/task_ops/tests/canonical.rs` 的
  `RecordingActor`（artifact fixture + `RuntimeAssemblyEvalTarget`）适合扩展为
  D4 提交语义测试；`RequestCapabilityApi::submit_task` 返回类型变化会机械影响
  5 个测试 impl（canonical / prepared_operation / execution_scope / test_runtime /
  f445h capability_harness）。
- 与兄弟节点无重叠：D1/D2/D3 已合入集成分支；本叶子只改 runtime（eval/host/
  transport 契约层边界）/ doc 新叶子，不碰 router、task-control、syntax。

## 关键实现决策（本叶子执行范围）

- **TaskRef 运行时值承载 canonical 字符串**：`dispatch` 求值结果为
  `RuntimeValue::String`，内容是 `skiff-task-v1:<owner>.<taskId>` canonical
  编码。opaque 语义由 compiler 类型系统保证（无公开构造路径）；recoverable
  期望计划中的 `TaskRef` 节点在 encode/decode 时要求 String 且必须通过 canonical
  taskRef 校验，普通字符串不能冒充。
- **类型计划**：`RuntimeBuiltinShape::TaskRef` + `RuntimeTypeNode::TaskRef` +
  `RuntimeRecoverableExpectedTypeNode::TaskRef`；linked/native/recoverable builtin
  入口统一经 `RuntimeBuiltinShape::of_name(...).leaf_node()` 落到新节点。
- **canonical taskRef 校验**：recoverable 边界（`skiff-runtime-boundary`）新增
  本地 canonical 校验（前缀 + base64url 无 pad 两段 + UTF-8 非空），不引入
  transport 依赖（boundary 依赖链保持叶子）。wire 侧仍由 transport `TaskRef`
  校验。
- **提交响应 DTO**：`TaskSubmitControlResponse`（taskRef / taskId / requestId
  canonical 字符串）定义在 `skiff-runtime-request-contract`；
  `RequestCapabilityApi::submit_task` 返回它（原来是 `()`）。
- **拒绝码分类**：host 在 `finish_control_response` 里把
  `TaskSubmitRejectionCode::parse(code)` 命中的 `task.submit.error` 映射为 host
  `RuntimeError::TaskSubmitRejected { code, message }`；
  `ordinary_root_error_into_capability` 把它投影成
  `CapabilityError::TaskSubmitRejected { code, message }`；eval task_ops 直接
  匹配该变体：definite → `ProviderUnavailable`（明确平台错误，不产生 task）；
  `storeUnavailable` → 同 TaskId 有界重试（3 次，50ms/100ms 退避），仍失败 →
  “结果不确定”平台错误。其它能力层错误（断连等）视为 ambiguous，同 TaskId
  有界重试后同样抛“结果不确定”。
- **TaskId 生成**：提交前 `uuid::Uuid::new_v4()`（eval 已依赖 uuid v4）；
  `task_id: Some(...)` 随每次重试复用。
- **求值顺序**：先解析 target/plan（不 eval），再求值 receiver/参数一次，再求值
  timing 表达式一次，最后 recoverable encode + 提交。`after` 只携带
  `durationMs`（D1 wire 契约），`at` 携带 `utcMillis`（RuntimeValue::Date）。

## 旧易失路径删除清单（以编译与测试为准）

```text
runtime/eval/src/task_ops.rs  submit_task_statement（语句专用旧提交实现）→ 由
                              submit_dispatch_call（语句/表达式共用）取代
runtime/eval/src/task_ops.rs  旧 dispatchSubmit metadata 解析（无 timing）→
                              新 task_submit_target + task_submit_timing_plan
```

保留：`execute_runtime_assembly_task_target` /
`resolve_runtime_assembly_task_target` / `runtime/request/src/task_execution.rs`
（durable attempt 的 `request.start` 执行车道，D2 控制面依赖）；`task_submit.rs`
wire encoder（提交帧编码）。

## 禁止

- 不改 syntax / compiler / router / task-control；不改
  `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**` 既有文件。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。

## 自验收矩阵（提交后与交接报告一致）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 语句与表达式共用 durable submit；表达式返回 TaskRef、语句丢弃 | `task_ops::submit_dispatch_call`；`eval_context.rs` `exec_statement_dispatch` 与 `eval_program_expr` dispatchSubmit 分支共用 | 无 `submit_task_statement` 引用 | `cargo test -p skiff-runtime-eval task_ops::tests` |
| 求值顺序与只求值一次 | args 求值一次 + timing expr 求值一次后才 encode/submit；metadata `timing.expr` 单指针 | 嵌套 dispatch 参数计数测试（总提交数 = 2） | 同上 |
| timing 三态 | `task_submit_timing_plan` / `evaluate_dispatch_timing` → `TaskSubmitTimingControl` | `rg "TaskSubmitTimingControl::Immediate" runtime/eval` 仅默认/测试构造点 | 三态用例断言 wire request timing |
| 提交前生成 TaskId、重试复用 | `new_task_id()` + `task_id: Some`；transient 重试循环不重建 | retry 用例断言两次请求 taskId 相等 | 同上 |
| definite rejection / ambiguous acceptance | `CapabilityError::TaskSubmitRejected` 分类；storeUnavailable 有界重试；仍失败抛“结果不确定” | definite 用例断言错误 + 0 submissions | 同上 |
| TaskRef 运行时值 + recoverable 往返 | dispatch 返回 canonical 字符串；`RuntimeTypeNode::TaskRef` / `RuntimeRecoverableExpectedTypeNode::TaskRef` / boundary precheck | `rg "TaskRef" runtime/model runtime/boundary` | recoverable roundtrip 用例 + boundary 测试 |
| 旧易失路径移除 | 删除清单见上；编译通过 | `rg "submit_task_statement"` 为空 | `cargo check -p skiff-runtime-eval` |
| 既有测试全绿 | 机械更新 5 个 `RequestCapabilityApi` impl | test-runner service flow 全绿 | `cargo test -p skiff-runtime-eval --lib`、`cargo test -p skiff-runtime-host --lib`、`cargo test -p skiff-test-runner --test test_service_flow` |

## 实际写集

```text
Cargo.lock
doc/implementation/dispatch-d4-runtime-leaf.md
runtime/boundary/Cargo.toml                       # base64（canonical taskRef 校验）
runtime/boundary/src/json.rs
runtime/boundary/src/json_convert/{coerce,materialize,wire_decode}.rs
runtime/boundary/src/recoverable.rs               # TaskRef expected-node precheck + canonical 校验
runtime/boundary/src/recoverable/tests.rs         # TaskRef 往返 + malformed/plain-string fail-closed
runtime/boundary/src/service_value_plan/matcher.rs
runtime/capability-context/src/capability_error.rs # CapabilityError::TaskSubmitRejected
runtime/capability-context/src/{lib,outbound_control,request}.rs
runtime/eval/Cargo.toml                            # dev-deps base64
runtime/eval/src/actor_dispatch/tests/prepared_operation.rs
runtime/eval/src/actor_executor/actor_concurrent_continuation.rs
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/src/db_eval.rs
runtime/eval/src/error.rs                          # CapabilityError::TaskSubmitRejected 投影
runtime/eval/src/eval_context.rs                   # 语句/表达式 dispatch 接入 + return 位置非尾调用
runtime/eval/src/program_execution/tests/execution_scope.rs
runtime/eval/src/task_ops.rs                       # 核心重写（见下）
runtime/eval/src/task_ops/tests/canonical.rs       # D4 提交语义测试（11 例）
runtime/eval/tests/f445h_e4r_combined/{capability_harness,imports}.rs
runtime/host/src/capability_context/actor.rs       # 提交响应 DTO + rejection code 分类
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/error.rs                          # RuntimeError::TaskSubmitRejected
runtime/host/src/eval_capability_adapter/{actor,error}.rs
runtime/host/src/host/router_session/tests/control_response_lifecycle.rs
runtime/linked-type-plan/src/type_plan/{recoverable.rs,tests.rs}
runtime/model/src/recoverable.rs
runtime/model/src/type_plan.rs
runtime/model/src/type_plan/builtins.rs
runtime/native/src/dispatch/resource.rs
runtime/native/src/error.rs
runtime/request-contract/src/{lib,outbound,outbound_control}.rs  # TaskSubmitResponseControl
```

`runtime/eval/src/task_ops.rs` 关键变化：

- 删除 `submit_task_statement`（旧语句专用提交实现）；新增
  `submit_dispatch_call`（语句/表达式共用）、`is_dispatch_submit_call`、
  `task_submit_timing_plan` / `evaluate_dispatch_timing`（immediate/after/at）、
  `new_task_id`、`submit_task_durable`（definite 拒绝 / storeUnavailable 与
  传输不确定的有界同 TaskId 重试）。
- `encode_task_request_payload` / `encode_task_function_payload` /
  `encode_task_actor_method_payload` 改为接收已求值参数（`Vec<RuntimeValueCarrier>`），
  保证 receiver/参数 → timing → recoverable encode 的顺序与单次求值。

## 验证记录

- `cargo check` 受影响 crates：model / linked-type-plan / boundary /
  request-contract / capability-context / native / eval / host / service-db /
  request / linker / transport / router / task-control 全部 PASS。
- `cargo test -p skiff-runtime-eval`：452 lib + 15 integration PASS（新增
  canonical 提交语义 11 例：immediate/after/at 三态、表达式返回 TaskRef、
  嵌套 dispatch 单次求值、transient 同 TaskId 重试、definite 拒绝、
  ambiguous 有界重试）。
- `cargo test -p skiff-runtime-host --lib`：427 PASS。
- `cargo test -p skiff-runtime-boundary`：171 PASS（新增 TaskRef recoverable
  往返 + fail-closed 2 例）。
- `cargo test -p skiff-runtime-transport`：task wire corpus 等全过。
- `cargo test -p runtime --test w_model_task_consumer --test
  h_task_parent_cut_corpus`：PASS。
- `cargo test -p skiff-test-runner --test test_service_flow`：16 PASS。
- `git diff --check` PASS。

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_d_integration`，并通知主 Agent `/root`。
