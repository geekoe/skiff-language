# 叶子任务：B1 拆分 ActorCapabilityContext，新增 RequestCapabilityContext

## 引用链

- 直接父节点：主 Agent `/root` 派发的任务信封（`/root/request_capability_dev`）；本任务只依据信封“直接父节点/设计决策”一节已确认的决策执行。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`、`/Users/geek/workspace/skiff/AGENTS.md`。
- baseline：`612c2503`（skiff main HEAD，零 worktree 只读预检锚定；worktree 建于 `/Users/geek/workspace/skiff-wt-request-capability`，branch `request-capability-split`）。

## 设计条款（信封给定，本文件不得覆盖）

1. 在 `skiff-runtime-capability-context` 拆出新的
   `RequestCapabilityApi` trait + `RequestCapabilityContext<'a>` +
   `OwnedRequestCapabilityContext`，承载 11 个请求元数据访问器
   （`runtime_id` / `service_id` / `service_version` / `request_id` /
   `request_target` / `request_build_id` / `spawn_service_protocol_identity` /
   `request_service_protocol_identity` / `operation_service_protocol_identity` /
   `activation_identity` / `trace_id`）与 `submit_spawn`。
2. `ActorCapabilityApi` / `ActorCapabilityContext` 只保留 owned/borrow 与 5 个
   actor model 操作（`get_or_create_actor` / `replace_actor` / `find_actor` /
   `remove_actor` / `invoke_actor`）。
3. 删除 `SpawnClient`（`runtime/capability-context/src/spawn.rs` 及导出）；
   `submit_spawn` 直接放 `RequestCapabilityContext`，`spawn_ops` 直接调用它。
4. eval：`ProgramExecutionContext` 等结构的 `spawn` 字段改名为
   `request: RequestCapabilityContext`（`actor` 字段保持
   `ActorCapabilityContext`）；元数据消费方（`program_db.rs`、
   `program_invocation.rs`、native 投影、`program_execution.rs` 的 bundle 校验）
   改从 request context 读取；同一元数据校验收敛为一次。
5. host：`runtime/host/src/capability_context/actor.rs` 的 concrete
   `ActorClient` 同样拆分：`ActorClient` 只留 actor model 操作；新增
   `RequestClient`（含 `submit_spawn` + 元数据袋，原 `ActorClientContext`
   更名为 `RequestClientContext`）；`eval_capability_adapter/actor.rs`
   同时实现两个新 trait；`factory.rs` / `assembly_execution_context.rs` /
   `activation_execution_rebinder.rs` 构造两个 context；
   `driver/capability_context/mod.rs` 再导出同步。

## 硬约束（禁止面）

- `control_mapper.rs` / `protocol.rs` / router schema / envelope target 一律不动。
- `std.actor` 语义与 `actor.method.invoke` 帧不变；wire/artifact/ABI 不得动。
- owned/borrow 机制在 `ActorCapabilityContext` 与 `RequestCapabilityContext`
  两个 context 上都要保留。
- 不得触碰 internals 的 `agine_actor.skiff`、service-db 命名、
  `impl/type-ref-phase7` worktree/branch（`/Users/geek/workspace/skiff-phase7`）。
- 不 push、不合并 main；完成本任务后向集成 Agent `/root/skiff_integration2` 交接，
  并通知主 Agent `/root`。

## 改动范围（自主闭合后实际写集，见 result）

- `runtime/capability-context/src/actor.rs`：收缩 `ActorCapabilityApi` /
  `ActorCapabilityContext`；新增 `request.rs` 承载
  `RequestCapabilityApi` / `RequestCapabilityContext` /
  `OwnedRequestCapabilityContext`；`lib.rs` 同步模块声明与导出，删除
  `spawn.rs` 与 `SpawnClient` 导出。
- `runtime/eval`：`capabilities.rs`（再导出、
  `EvalRequestExecutionCapabilities`、`RuntimeNativeActorCapabilityContext`）、
  `native_capability.rs`、`program_execution.rs`、`spawn_ops.rs`、
  `program_db.rs`、`program_invocation.rs`，以及因果相关的测试 double /
  fixture（`test_runtime.rs`、spawn_ops/execution_scope/actor_dispatch/
  f445h_e4r_combined 等测试）。
- `runtime/host`：`capability_context/actor.rs`（`ActorClient` /
  `ActorClientContext` / `RequestClient` / `RequestClientContext`）、
  `capability_context/mod.rs`、`capability_context/native_projection.rs`、
  `eval_capability_adapter/{actor,factory,downcast,assembly_execution_context,
  activation_execution_rebinder}.rs`，及 host 侧相关测试。
- `runtime/driver/capability_context/mod.rs`：再导出同步（`RequestClient` /
  `RequestClientContext` 等）。

测试 double 处理遵循“自主闭合”规则：同一测试 struct 需要时同时实现
`ActorCapabilityApi` 与 `RequestCapabilityApi`（例如
`TestActor` / `HarnessActor` / 各 `RecordingActor` / `CarrierReceiptActor`），
元数据与 `submit_spawn` 只出现在 `RequestCapabilityApi` impl 中。

## 验证命令（证据 owner：/root/request_capability_dev）

```bash
cd /Users/geek/workspace/skiff-wt-request-capability/runtime
cargo check -p skiff-runtime-capability-context
cargo check -p skiff-runtime-eval
cargo check -p skiff-runtime-host
cargo check -p runtime   # driver（runtime 根 package，信封中的 skiff-runtime-driver）
cargo test -p skiff-runtime-capability-context
cargo test -p skiff-runtime-eval spawn_ops
cargo test -p skiff-runtime-eval actor
cargo test -p skiff-runtime-host actor
cargo test -p skiff-runtime-host control_response_lifecycle
git diff --check
```

反向搜索证明（提交后、最终代码状态）：

```bash
rg -n 'fn (runtime_id|service_id|service_version|request_id|request_target|request_build_id|spawn_service_protocol_identity|request_service_protocol_identity|operation_service_protocol_identity|activation_identity|trace_id|submit_spawn)' runtime/capability-context/src/actor.rs
# 期望：无输出（ActorCapabilityApi 不再有元数据访问器与 submit_spawn）
rg -n 'SpawnClient' runtime --glob '*.rs'
# 期望：无输出（SpawnClient 无残留引用）
rg -n 'submit_spawn' runtime/eval/src/spawn_ops.rs
# 期望：仅通过 request context 调用，无 SpawnClient 包装
rg -n 'spawn_context\(\)|\.spawn\b' runtime/eval/src --glob '*.rs'
# 期望：无输出（request context 是元数据 + spawn 的唯一入口）
```

## 交接目标

完成提交后向集成 Agent `/root/skiff_integration2` 报告 branch、worktree 路径、
implementation/result commit/tree、实际写集、自验收矩阵；并通知主 Agent `/root`
状态。不 push、不合并 main。
