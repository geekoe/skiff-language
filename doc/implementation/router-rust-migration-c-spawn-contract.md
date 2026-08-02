# Router Rust Migration C-spawn：spawn lane handler 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；`H-spawn-parent-cut` 后解锁
W-spawn / E-actor-rust）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md`
  - §5.3（`C-model-spawn → W-model-spawn → M-spawn → H-spawn-parent-cut`；
    `callerKind` 决策与无字符串猜测 fallback）；
  - §5.4（`C-spawn + M-spawn + H-spawn-parent-cut`：
    `FunctionSpawnParentResolver` + `ActorSpawnParentResolver` +
    stateless `SpawnSubmitRouter`；collision、parent terminal、replacement
    竞态必须测试）；
  - §5.5（`SpawnSubmitRouter` 在 stable sink bundle，sink 不拥有 pending）；
  - §3.4（authority/fence 类型）、§7（E-actor-rust：function spawn 与
    actor-method spawn parent authority 明确，accepted spawn 与 parent
    生命周期分离）。
- 父批次：`doc/implementation/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-actor-leaf.md`。
- 同链契约：`router-rust-migration-c-model-spawn-contract.md`（wire
  shape 与 cut 前置）、`router-rust-migration-c-model-actor-contract.md`、
  `router-rust-migration-c-actor-contract.md`。

冲突时以权威设计为准；本文件只冻结契约与 corpus，不写 production。

## 1. 范围

冻结 spawn lane 的 handler 级端口：`FunctionSpawnParentResolver`、
`ActorSpawnParentResolver`、stateless `SpawnSubmitRouter` 的 §5.4 必填项，
以及 `H-spawn-parent-cut` 的解锁前置（C-spawn 在 hard-cut 后才解锁，本
pack 冻结 cut 语义与 corpus，不实现 cut）。

## 2. Owner / invariant

- Owner：
  - `FunctionSpawnParentResolver`：从 `RequestDispatcher` 返回 fenced
    authority snapshot 解析 `callerKind=request` parent；
  - `ActorSpawnParentResolver`：从 `ActorInvocationRelay` 返回 fenced
    authority snapshot 解析 `callerKind=actorInvocation` parent；
  - `SpawnSubmitRouter`：按 exact parent kind 选择 resolver + 按
    `targetKind` 分类目标 + 生成 `requestId`/acceptance（stateless）。
- 明确不拥有：parent pending（RequestDispatcher / ActorInvocationRelay
  拥有）、accepted spawn 的执行（各执行 owner 拥有）、parent-child
  pending 映射（不存在）。
- Invariant：
  1. parent correlation 严格 typed `(callerKind, callerRequestId)`；
     同一字符串跨 namespace 不碰撞（`callerKind` 决定唯一解析路径）；
  2. 不存在 fallback：缺 `callerKind` / 非法 `callerKind` 一律
     `CallerKindRejected`，不做字符串前缀猜测或默认 request；
  3. resolver 返回的 authority snapshot 必须 exact（runtime connection、
     assembly identity/generation、deployment tuple、testCaseCapability）；
     任一不满足 fail closed；
  4. accepted spawn 与 parent 生命周期分离：acceptance 后 parent
     terminal/replacement 不影响已 accepted spawn；
  5. router 无 pending 残留：success/error/disconnect/saturation/shutdown
     后所有 submit 查询与相关 correlation 归零。

## 3. Typed inputs / outputs

### 3.1 FunctionSpawnParentResolver

- Inputs：`ParentQuery { caller_kind: request, caller_request_id,
  request_dispatcher_snapshot: RequestDispatcherSnapshot }`；
  `RequestDispatcherSnapshot` 含 fenced authority
  （`RuntimeSpawnParentAuthority`）+ parent request correlation 查询 port。
- Outputs：`SpawnParentResolution { kind: request, parent_request_id,
  authority, origin_runtime_connection }` 或
  `ParentRejected { code }`（closed set：ParentNotFound / ParentTerminal /
  ParentReplaced / ParentConnectionMismatch / AuthorityMismatch /
  TestCapabilityMismatch）。

### 3.2 ActorSpawnParentResolver

- Inputs：`ParentQuery { caller_kind: actorInvocation,
  caller_request_id, actor_invocation_snapshot:
  ActorInvocationRelaySnapshot }`；snapshot 含 fenced authority +
  invocation 相关性查询 port。
- Outputs：`SpawnParentResolution { kind: actorInvocation,
  parent_invocation_id, authority, origin_runtime_connection }` 或
  `ParentRejected { code }`（同 closed set）。

### 3.3 SpawnSubmitRouter

- Inputs：`SpawnSubmitRequestFrameHeader`（target shape，C-model-spawn
  §3.1）+ payload + 两个 resolver port。
- Outputs：`SpawnSubmitAcceptance { spawn_id, request_id, status:
  submitted }` 或 `SpawnSubmitError`（closed set：ParentNotFound /
  ParentTerminal / ParentReplaced / ParentConnectionMismatch /
  CallerKindRejected / TargetKindMismatch / AuthorityMismatch /
  Saturated / UnknownTarget）。
- Stateless：除 shared capacity counter（原子计数，非 pending 映射）外
  不保存跨 submit 状态。

## 4. 行为（frozen）

1. decode + 校验 `callerKind` closed enum；缺失/非法 ->
   `CallerKindRejected`（旧 shape，cut 语义）。
2. 按 `callerKind` 精确选择 resolver；**不跨 namespace 查找**。
3. resolver 校验 parent 存在且 active、origin connection 精确一致、
   authority exact（assembly/generation/deployment/testCaseCapability）；
  任一失败 -> 对应拒绝码。
4. 按 `targetKind` 分类：`function` 要求无 `actorMethod` 字段；
   `actorMethod` 要求 `actorMethod` 存在且 identity 字段完整（
   `TargetKindMismatch` 反之）。
5. 生成 `requestId`（Router 侧 opaque token），返回
   `spawn.submit.response`；accepted spawn 的 `requestId` 交给执行 owner
   关联，router 不保留 parent-child 映射。
6. accepted spawn 与 parent 生命周期分离：parent terminal/replacement
   只影响未 accepted 的 submit 查询。

## 5. Capacity / queue full

- submit 并发上限：共享 `runtime.maxConcurrency` 预算（原子计数）；
  超出 -> `Saturated` 立即拒绝，不排队。
- 单次 resolver 查询 deadline：默认 5s，超时 -> `ParentTerminal`。
- spawn writer queue：C-session per-session 预算（outbound 256 帧 /
  4 MiB）；non-blocking enqueue；满 -> abort exact runtime connection。
- authority snapshot 只携带 typed refs，不复制 payload。

## 6. Timeout / disconnect / replacement / shutdown terminal

| 事件 | terminal |
| --- | --- |
| resolver 查询超时 | `ParentTerminal`（未 accepted）；不产生部分 acceptance |
| parent origin runtime disconnect | 未 accepted submit -> `ParentConnectionMismatch`；已 accepted spawn 由执行 owner 按 disconnect 收敛 |
| parent replacement | 旧 authority snapshot 的 submit -> `ParentReplaced`；new parent 不继承旧 submit；已 accepted spawn 不受影响 |
| testCaseCapability authority 漂移 | `TestCapabilityMismatch` / `AuthorityMismatch`，fail closed |
| shutdown | 全部 submit 查询 / shared counter 归零；C-process-lifecycle 覆盖 |

## 7. Health fields

- 按 `callerKind` 的 submit accepted/rejected 计数；错误码分布（closed
  set）；resolver 查询占用/超时；legacy-cut 拒绝计数（
  `CallerKindRejected`）；spawn writer queue 占用；accepted 后
  `requestId` 生命周期计数（转交执行 owner 数）。

## 8. Fake seam

- `FakeRequestParentStore` / `FakeActorInvocationParentStore`：内存 pending
  + 可注入 terminal/replacement/connection 变化/authority 漂移；
- `FakeClock`：推进 resolver 查询 deadline；
- `FakeSpawnWriter`：可注入 queue full / write fail。
- corpus：`runtime/transport/testdata/spawn-wire/` +
  `runtime/transport/tests/spawn_wire_corpus.rs`（target wire mirror +
  resolver/router 参考模型，覆盖 collision / parent terminal /
  replacement 竞态）。

## 9. Real boundary probe（定义，M-spawn/W-spawn/E-actor-rust 执行）

- `router-live:spawn-parent-cut`（C-model-spawn §7.8）：真实 Router +
  fake Runtime，同一 `callerRequestId` 双 namespace 提交互不碰撞；parent
  断开/替换后 submit 拒绝；旧 shape 帧 cut 后无 fallback。
- `router-live:spawn-lifecycle`：accepted spawn 在 parent terminal 后继续
  执行并正常返回；shutdown 后 spawn pending 归零。
- 两者成为 `router-rust-actor-live` 的 required managed CI。

## 10. H-spawn-parent-cut 依赖与解锁

- C-spawn 依赖 C-model-spawn（wire target）+ M-spawn（shared corpus 真实
  consumer gate）。
- `H-spawn-parent-cut` 让 current Runtime 与 TS Router 同时硬切消费
  `callerKind` wire，随后删除旧 shape（无兼容 reader）。
- 本 pack 冻结 cut 前置：corpus 中旧 shape 帧（
  `submit.request.legacy-no-caller-kind`）必须是 `legacyCut`，target
  mirror 拒绝且不存在 fallback reader；W-spawn 在 cut 后实现 resolver/
  router 并消费同一 corpus。
