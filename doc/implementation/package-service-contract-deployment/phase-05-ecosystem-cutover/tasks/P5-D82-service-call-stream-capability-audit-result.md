# P5-D82：普通 Service Call Stream 能力审计结果

结论：`READY_TO_IMPLEMENT`

## 父节点与权威链

- 直接父节点：`P5-D82-service-call-stream-capability-audit.md`
- 该父节点引用唯一权威设计 `doc/architecture/package-service-contract-deployment.md`。
- 审计代码状态：
  - Skiff `ccc0f2c0fcf2bb8c99ca3c18968b41259f548bdd`
  - skiff-packages `6e6828c38d6634bf0bd538dbcbd2532815f246c2`
  - Internals `2cf2ebd22b502d0b2069dd6ef5db8ee4dd9032f2`

## 真实 consumer 链

1. AIHub public API：`internals-phase-05-integration/aihub/service/api.yml`
2. provider：`aihub/service/internal/aihub_service.skiff` 的 `streamChat`
3. Agine 精确 service dependency：`agine/service/package.yml`
4. caller：`agine/service/internal/agent_bridge_llm_adapter.skiff` 对
   `aihub/managedLlm.streamChat(input)` 的 `for` 消费。

## Owner 与缺口

- Compiler boundary projection：此前把所有 callable 固定为 unary，并把 `Stream<T>` 判为
  `UnsupportedStream`；这是首个 blocker。
- Artifact/contract schema 已拥有 `ServerStream { item_type, item_value_plan }`，不需要新 schema。
- Caller source typing 明确拒绝 server-stream contract call；依赖 projection checkpoint 后闭合。
- Service-call lowering、精确 requirement slot/operation/protocol identity、Runtime assembly resolve 和 in-process dispatch
  已存在，后续需要真实 fixture 证明接线。
- Runtime item/error/end、generic substitution、detached materialization、cancel 和 stream lease 的 production owner 在
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs`。
- 真实 admitted binding、concrete stream registry 和 full-chain 测试 owner 在 `runtime/host`，不是 `runtime/eval`：
  - `runtime/host/src/capability_context/stream_runtime.rs`
  - `runtime/host/src/loader/assembly_admission/tests/execution/`
  - 可复用入口是 `TypedExecutionFixture` 与 `execute_runtime_assembly_addr`。
- `runtime/eval` 的 ordinary test runtime 对 channel stream 操作 panic；eval 不能反向依赖 host，否则形成循环依赖。
- HTTP stream 是独立 owner；不得作为普通 service-call stream adapter 或 fallback。

## 实现 DAG

1. Compiler projection：最外层 `Stream<T>` 投影为既有 `ServerStream`，嵌套/参数 stream 继续 fail closed。
2. Caller source typing：允许 contract server-stream expression 被顺序消费。
3. Lowering/contract/deployment fixture：证明真实 callable 形成精确 `ServiceCallRef` 与 public type closure。
4. Runtime Host full-chain probes：通过 admitted fixture 覆盖顺序、generic substitution、error/cancel/cleanup。
5. AIHub → Agine consumer 重验。

## 最小探针

- 正例：public `Stream<LlmStreamEvent>` → Available ServerStream；caller `for` lowering；Host admitted binding 发出两个
  item 后 end。
- 负例：Stream 参数/嵌套/collection 仍 unavailable；provider error、consumer/request cancel 释放目标 stream lease，
  不影响 peer stream；HTTP event 不进入 service-call adapter。

任何后续任务若需要改变公共类型、item/error/end lifecycle 或错误语义，必须停止并回到权威设计。

