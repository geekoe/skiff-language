# Router Rust Migration C-model-activation contract

日期：2026-08-02
状态：frozen（contract pack 交付；W-model-activation 为其实现 lane）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md`
  - §2.2 第一类 model：Router↔Runtime wire model，owner 为 `skiff-runtime-transport` 及必要的
    低层 request contract；
  - §3.4 `ActivationId` / `ActivationParticipantBinding`；§3.5 registration variant
    （`assembly.activation:Register` 属 activation envelope 的 registration variant，
    state owner 是 `RuntimeRegistrationDirectory`，不归 `ActivationCoordinator`）；
  - §4.1 live transaction（prepare/ACK/commit/abort wire 步骤）；
  - §5.3 `C-model-activation`（prepared/reject/prepare/commit/abort transaction wire）；
  - §5.5 `AssemblyActivationSinks { registration: RegistrationFrameSink,
    transaction: ActivationTransactionFrameSink }`；
  - §5.4 pack 必填项。
- 批次文档：`doc/implementation/router-rust-migration-batch-3.md`（contracts-activation 节点）。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-activation-leaf.md`。
- 现有实现：`artifact-model/src/assembly_activation_control.rs`（DTO owner）、
  `runtime/transport/src/assembly_activation.rs`（frame codec + direction）、
  `cross-system-fixtures/package-service-ecosystem/control-wire.json` +
  `runtime-wire.json`（golden byte corpus）。

## 1. Owner 与 invariant

- DTO owner：`skiff-artifact-model`（`AssemblyActivationControl`，v2 strict schema）。
- codec/direction owner：`skiff-runtime-transport`（`assembly_activation` module，
  M0 已确立的 activation family）。
- 事务 wire 只含五个 transaction variant：`Prepare` / `Prepared` / `Reject` / `Commit` / `Abort`。
  `Register` 是同 family 的 registration variant，由 C-session/registration lane 冻结
  （byte corpus 同一处维护），本契约不定义其语义。
- Runtime 只消费本 wire projection（prepare/commit/abort 出、prepared/reject 入），
  不消费 durable activation record（§2.2 第三类边界）。
- invariant：帧 schemaVersion == `RUNTIME_FRAME_SCHEMA_VERSION`；frame type == `assembly.activation`；
  payload 必须为空；strict decode（deny unknown fields）；方向矩阵精确：
  Router→Runtime = prepare/commit/abort；Runtime→Router = prepared/reject/register。

## 2. 精确 wire（frozen）

每个 transaction variant 字段（camelCase，`deny_unknown_fields`）：

```text
Prepare  { environment, activationId, expectedGeneration, candidateGeneration,
           assembly: RuntimeAssemblyRef, configSnapshot: RuntimeConfigSnapshotRef,
           replicaId, serviceDb?: { mongoUrl } }        // Router -> Runtime
Prepared { environment, activationId, expectedGeneration, candidateGeneration,
           assembly, configSnapshot, replicaId }         // Runtime -> Router
Reject   { environment, activationId, expectedGeneration, candidateGeneration,
           assembly, configSnapshot, replicaId,
           reason: Resolve|Load|Link|Admission|ParticipantDisconnected }  // Runtime -> Router
Commit   { environment, activationId, expectedGeneration, candidateGeneration,
           assembly, configSnapshot, replicaId, serviceDb?: { mongoUrl } }  // Router -> Runtime
Abort    { environment, activationId, expectedGeneration, candidateGeneration,
           assembly, configSnapshot, replicaId }         // Router -> Runtime
```

校验规则（frozen，来自 `AssemblyActivationControl::validate`）：

- environment / activationId / replicaId 为 token；`expectedGeneration + 1 == candidateGeneration`；
- assembly/config snapshot refs 严格有效；serviceDb.mongoUrl 非空字符串（仅 Prepare/Commit 可携带）；
- transaction variants 不允许携带其它字段；服务 id、build id、artifact root、executable target
  均不可出现在 wire 上。

## 3. Stale ACK 拒绝与 participant binding

`ActivationParticipantBinding { replica_id: ReplicaId, session: RuntimeSessionEpoch }`
（§3.4；`RuntimeSessionEpoch` 类型由 C-session/contracts-session 冻结，本契约不定义 production 类型）。

- wire 只携带 `replicaId`；session epoch 属于 coordinator 内部 binding，绝不上 wire；
- ACK（`Prepared`/`Reject`）按**当前 live binding**校验：
  - replicaId 不在 participant set → 拒绝（stale）；
  - replicaId 在 participant set 但来源 session 的 `RuntimeSessionEpoch` 与 binding 不等 → 拒绝
    （stale/new session）；
  - 同一 session 重复 ACK → 拒绝（stale duplicate）；
  - ACK 的 generation/tuple 与 pending 不匹配 → 拒绝；
  - durable commit CAS 已发出后到达的 ACK → 拒绝（decision 后不再接受 live ACK）。
- 拒绝的 ACK 不产生 durable effect；coordinator 记录 stale ACK counter（health）。

## 4. 方向、容量与 Queue full

- 方向矩阵见 §1；错误方向 encode/decode 一律失败（golden corpus 已断言 reverse direction
  error）。
- 每个 session 的 writer queue 有界（frame/byte permit）；prepare/commit/abort enqueue
  必须 non-blocking（`try_send`），不跨 `.await` 持有 owner state；
- queue full（enqueue 失败）：decision 前 → coordinator durable abort + abort exact session；
  decision 后（durable commit 已成功）→ abort exact session，不回滚 durable state
  （§4.1 step 5/9；本契约把 writer queue failure 按 exact session fence 处理）。

## 5. Timeout / Disconnect / Replacement / Shutdown terminal

| 事件 | decision 前（durable commit CAS 前） | decision 后（CAS 已发出/成功） |
| --- | --- | --- |
| ACK timeout | durable abort，向已 staged 的 exact session enqueue abort | 读 durable state reconcile；不直接假定 abort |
| session disconnect | durable abort（同左） | durable outcome authoritative；reconnect 按 committed bootstrap 收敛，丢弃 staged candidate |
| session replacement | durable abort（新 epoch 未绑定 transaction） | 不向新 session enqueue；按 committed bootstrap 收敛 |
| shutdown | 不再发起新 transaction；in-flight 若已 prepare → durable abort（或按已读 durable 决定）；enqueue 归零 | 读 durable state 后按 committed/aborted 收敛 enqueue，session abort，pending 归零 |

## 6. Health fields

activation wire/transaction health 至少暴露：

- 每个 direction 的 frame counter（prepare/prepared/reject/commit/abort enqueued/received）；
- 当前 transaction 的 ACK 计数与 stale ACK 计数；
- pending ACK set 与 participant binding 数；
- writer queue 占用（frame/byte）与 saturation terminal 次数；
- decision 状态（prepared / committing / committed / aborted）。

## 7. Fake seam

`AssemblyActivationTransactionSink`（§5.5 demux 的 transaction 分支）：

```text
transaction: {
  prepare(participant: ActivationParticipantBinding, prepare: Prepare) -> EnqueueResult,
  commit(participant: ActivationParticipantBinding, commit: Commit) -> EnqueueResult,
  abort(participant: ActivationParticipantBinding, abort: Abort) -> EnqueueResult,
}
```

corpus 的 fake harness 用该 seam 的 test double 驱动；wire codec 始终走真实
`encode/decode_assembly_activation_frame`。

## 8. 真实边界 probe（至少一条）

`router-live:activation-full-chain`（E-activation）：真实 Router + Mongo + compiler artifact +
Runtime，断言 prepare→prepared→commit→epoch swap→runtime.registered re-register→
new-generation HTTP request；byte-exact golden corpus（`control-wire.json` +
`runtime-wire.json`）在 baseline 已由 `runtime/transport/src/assembly_activation/tests.rs`
持续执行。

## 9. 验证映射

- golden byte + direction + mutation corpus：现有 `assembly_activation/tests.rs`（frozen）；
- 本批次新增 `activation-transaction-cases.json` +
  `runtime/transport/tests/activation_transaction_corpus.rs`：事务语义 corpus
  （stale ACK、binding、decision 前后 terminal、live vs cold），wire 事件经真实 codec
  encode/decode。
