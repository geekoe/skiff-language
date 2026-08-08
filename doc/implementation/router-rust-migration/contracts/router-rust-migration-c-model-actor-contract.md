# Router Rust Migration C-model-actor：actor wire 与 model 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-model-actor / W-actor / M-actor 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`
  - §3.2（`ActorMethodCatalogView` stateless、`ActorOwnershipRegistry`
    claim truth 唯一 owner、`ActorActivationRequestBroker` get-or-create
    dedup、`ActorInvocationRelay`、`ActorOwnerControlBroker`、
    `ActorLeaseExpiryScheduler` 职责表）；
  - §3.3（actor method index 属于 immutable `RoutingEpoch`，无独立 refresh）；
  - §3.4（`ActorIncarnationFence` 等 identity/fence 不可互换）；
  - §5.3（C-model-actor → W-model-actor → M-actor lane）；
  - §5.4（contract pack 必填项）、§5.5（`ActorFrameSink`）、§7
    （E-actor-rust / E-actor-parity）。
- A0 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`
  （`ActorRoutingProjection` 只读消费边界；消费者不得回退读取
  PackageArtifact/File IR）。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-actor-leaf.md`。
- 同链契约：`router-rust-migration-c-actor-contract.md`（handler 级端口）、
  `router-rust-migration-c-model-spawn-contract.md`、`router-rust-migration-c-spawn-contract.md`。

冲突时以权威设计为准；本文件只冻结契约与 corpus，不写 production。

## 1. 冻结范围

冻结 actor 族 wire 模型（`actor_method` / `actor_owner` / actor control
现有 wire）、六个 model 面（catalog view、ownership registry、activation
request broker、invocation relay、owner control broker、lease expiry
scheduler）的 typed 边界，以及 byte-exact actor wire corpus。不定义
`RoutingEpoch` 的构造（C-bootstrap）、不定义 durable activation DTO
（contracts-activation）、不定义 spawn parent 语义
（C-model-spawn/C-spawn）、不写 skiff-router production。

## 2. Actor 族 wire（frozen）

Frame family：`RuntimeFrameFamily::Actor`（M0 已冻结 closed registry），
direction `Either`，payload presence `Optional`（帧级规则见下表）。
wire prefix：`actor.`。schemaVersion：
`RUNTIME_FRAME_SCHEMA_VERSION`（`skiff-runtime-frame-v3`）。

### 2.1 帧集合与 direction / payload

| 帧 | direction | payload | decodeAs（canonical codec） |
| --- | --- | --- | --- |
| `actor.getOrCreate.request` | Runtime→Router | bootstrap bytes（语义必填） | `ActorGetOrCreateRequestFrameHeader` |
| `actor.getOrCreate.response` | Router→Runtime | 空 | `ActorGetOrCreateResponseFrameHeader` |
| `actor.getOrCreate.error` | Router→Runtime | 空 | `ActorSpawnRuntimeErrorFrameHeader`（type=`actor.getOrCreate.error`） |
| `actor.replace.request` | Runtime→Router | bootstrap bytes（语义必填） | `ActorReplaceRequestFrameHeader` |
| `actor.replace.response` | Router→Runtime | 空 | `ActorReplaceResponseFrameHeader` |
| `actor.replace.error` | Router→Runtime | 空 | `ActorSpawnRuntimeErrorFrameHeader` |
| `actor.find.request` | Runtime→Router | 空 | `ActorFindRequestFrameHeader` |
| `actor.find.response` | Router→Runtime | 空 | `ActorFindResponseFrameHeader` |
| `actor.find.error` | Router→Runtime | 空 | `ActorSpawnRuntimeErrorFrameHeader` |
| `actor.remove.request` | Runtime→Router | 空 | `ActorRemoveRequestFrameHeader` |
| `actor.remove.response` | Router→Runtime | 空 | `ActorRemoveResponseFrameHeader` |
| `actor.remove.error` | Router→Runtime | 空 | `ActorSpawnRuntimeErrorFrameHeader` |
| `actor.method.invoke` | Runtime→Router（caller）/ Router→Runtime（owner relay 内嵌） | arguments payload | `ActorMethodInvokeFrameHeader` |
| `actor.method.return` | Runtime→Router（owner）→Runtime（caller） | return payload | `ActorMethodReturnFrameHeader` |
| `actor.method.error` | Runtime→Router（owner）→Runtime（caller） | 空 | `ActorMethodErrorFrameHeader` |
| `actor.method.cancel` | Runtime→Router（caller）→Runtime（owner） | 空 | `ActorMethodCancelFrameHeader` |
| `actor.owner.invoke` | Router→Runtime（owner） | arguments payload | `ActorOwnerInvokeFrameHeader` |
| `actor.owner.control` | Router→Runtime（owner） | 空 | `ActorOwnerControlFrameHeader` |
| `actor.owner.control.ack` | Runtime→Router | 空 | `ActorOwnerControlAckFrameHeader` |
| `actor.owner.failure` | Runtime→Router | 空 | `ActorOwnerFailureFrameHeader` |

### 2.2 frozen 校验面（canonical codec 已实现，本 pack 固化）

- `ActorLogicalRefFrameHeader` / `ActorRefFrameMetadata` /
  `ActorKeyFrameMetadata`：serviceId + actorTypeIdentity +
  actorIdTypeIdentity + actorIdEncodingVersion + canonical key bytes
  （canonical base64）+ actorIdHash（`sha256:` + 64 位小写 hex）；
  logical ref 的 epoch 必须为正且为 JS safe integer。
- identity 字段：`skiff-actor-abi-v1:sha256` /
  `skiff-actor-implementation-v1:sha256` / `skiff-actor-method-v1:sha256`，
  后接 64 位小写 hex；`actor.method.invoke` / `actor.owner.*` 在
  decode/encode 双侧校验，getOrCreate/replace 的 identity 语义校验在
  admission 面（W-actor），wire 面只做 typed decode。
- `actor.method.invoke`：argumentsEncodingVersion 精确
  `skiff-actor-arguments-v1`；deadline = `{timeoutMs > 0, expiresAt 非空}`；
  cancellationCorrelation 为 canonical token；testCaseCapability 与
  testCaseParentRequestId 必须成对出现（canonical token）。
- `actor.method.return`：returnEncodingVersion 精确 `skiff-actor-return-v1`；
  `actor.method.error` / `actor.method.cancel` payload 必须为空。
- `actor.owner.invoke`：targetRuntimeId 必须等于 ownerFence.ownerRuntimeId；
  invoke.actorRef.epoch == ownerFence.epoch；invoke 的 abi/impl/owner 与
  fence 精确相等；routeAuthority 的 assemblyIdentity/generation 严格校验。
- `actor.owner.control`：operations 为 closed enum
  `MarkUpgrading | Discard | Activate | ActivateInitial | IdleEvict`；
  Activate 必须带 transition（newEpoch == fence.epoch，
  oldEpoch < newEpoch）；ActivateInitial 必须带 bootstrap + deadline；
  IdleEvict 必须带 fence.evictionRequestId，不得带 transition/bootstrap/
  deadline/testCase 字段；MarkUpgrading/Discard 不得带任何 optional 字段。
- `actor.owner.control.ack`：accepted=true 时不得带 failure reason。
- `actor.owner.failure`：ownerRuntimeId/ownerLeaseId/epoch/implementation
  必须与 admitted fence 精确匹配（相关性由 ActorInvocationRelay 冻结）。

### 2.3 actor control 帧（getOrCreate/replace/find/remove）

`ActorGetOrCreateRequestFrameHeader` 冻结字段：schemaVersion、type、rpcId、
runtimeId、activationIdentity（assemblyIdentity / generation /
runtimeReplicaId / deploymentRevision）、actorKey、actorAbiIdentity、
actorImplementationIdentity、bootstrapEncodingVersion、declarationOwner、
可选 deadline、可选 testCaseCapability + testCaseParentRequestId（成对）。
getOrCreate 的 payload 是 actor activation bootstrap bytes（不透明；
codec 不解释内容）。replace 同 getOrCreate（无 testCase 字段）。find/remove
无 payload。错误帧统一 `spawn.submit` family 的
`ActorSpawnRuntimeErrorFrameHeader` 形态但 type 为对应 `actor.*.error`，
payload 为空。

## 3. ActorMethodCatalogView（只读 A0 projection，不读 File IR）

`ActorMethodCatalogView` 是 stateless typed query：

- 构造：只接受显式 `Arc<RoutingEpoch>` 捕获的 immutable actor index
  （index 一次构造自 A0 `ActorRoutingProjection`，属于 epoch，无独立
  index/refresh/publication/mailbox）。
- 查询 key（frozen）：精确 method entry key =
  `{ actor: ActorRoutingRef, actor_implementation_identity, method_identity }`，
  即 A0 `ActorRoutingMethod` 的完整 typed key；`has_method(key)` 与
  `method_for(key)` 返回该 epoch 的精确 entry（含 deployment + package
  binding）。空 methods 的投影合法：所有查询返回未命中。
- 禁止：读取 PackageArtifact/File IR/source/executable payload；把
  modulePath/actorName/methodName/sourceSpan/unit/file 坐标作为查询输入；
  建立独立 index、refresh 或 mutable cache；访问 actor live state。
- 与 wire `declarationOwner` 的关系：declarationOwner 是 wire 上的声明
  归属事实（codec 校验其 shape）；catalog view 本身不使用它做 admission
  key——(abi, implementation, method) identity 已 canonical 覆盖声明形状。
  wire declarationOwner 与 epoch 派生事实的交叉校验由 W-actor 的
  invocation admission 冻结，不属于本投影消费面。

corpus：`runtime/transport/testdata/actor-routing-projection.json`
（正例 + 反例，测试内 mirror struct 严格对照 A0 schema；
权威类型校验由 `skiff-deployment::projection::actor_routing` 自己的 A0 测试
承担）。

## 4. ActorOwnershipRegistry（claim truth 唯一 owner）

唯一 owner 语义（权威设计 §3.2）：concurrent first-owner 的 authoritative
transition 只发生在 `ActorOwnershipRegistry`；broker 不得各存一份 claim
truth。

### 4.1 ActorClaimToken（router-local typed，不上 wire）

```text
ActorClaimToken {
  claim_id: ActorClaimId,            // "actor-claim-<uuid>"（canonical token）
  actor_key: ActorLogicalKey,        // 完整 logical key（含 canonical bytes/hash）
  expected_epoch: u64,               // reserve 时 registry 的当前 epoch
  owner_runtime_id: RuntimeReplicaId,
  route_authority: ActorOwnerRouteAuthority,  // 捕获的 immutable epoch 权威
}
```

### 4.2 registry 操作（frozen）

1. `reserve(actor_key, expected_epoch, owner_runtime_id, route_authority)`
   -> `Ok(ActorClaimToken)` | `Conflict { current_fence }` |
   `NotPresent` | `EpochMismatch { current_epoch }`。每个 actor key 同时
   最多一个 in-flight reservation；已有有效 owner fence（lease 未过期）时
   reserve 返回 Conflict，不覆盖。
2. `commit(token, activation_fence_facts)` -> `Ok(ActorOwnerFence)`：
   只接受未 abort 的 reservation；commit 后该 actor key 的 current owner
   fence 原子替换为新 fence（epoch 递增），reservation 清除。commit 必须
   带 token 回 registry；任何 broker 不能直接写 owner 字段。
3. `abort(token)` -> `Ok(())`：清除 reservation，不产生 owner 效果。
4. lease 面：`renew(fence, ttl)`、`release(fence, reason)`、
   `expire(now)`（返回过期 fence）——仍只由 registry 修改 owner truth。

invariant：一个 actor key 在任何时刻至多一个 current owner fence；
reservation 不构成 owner；token 被 commit 或 abort 后立即失效（二次
commit/abort 拒绝）；owner fence 的 epoch 单调递增。

## 5. ActorActivationRequestBroker（get-or-create dedup）

- dedup key：actor logical key；同一 key 的并发 getOrCreate 合并到同一
  claim：第一个 caller 执行（从 registry reserve token），joiners await
  同一 promise。
- lineage：ordinary 与 testCapability lineage 不互混；同 key 并发创建属于
  不同 test capability lineage -> `ActorCreateLineageConflict`（两个调用
  都失败，不发布 owner）。
- 执行流：find 已存在（present）-> 直接 resolve 现有 ActorRef（不 reserve）；
  否则 registry.getOrCreate -> `ActorOwnerControlBroker` 发
  activateInitial（持 token）-> ACK accepted -> registry.commit(token) ->
  resolve；failure/timeout/disconnect -> registry.abort(token) -> 全部
  waiters 失败。
- activation request/ACK correlation：requestId + 精确 owner runtime
  connection + operation=`activateInitial` + route authority；ACK 不匹配
  拒绝；owner 断开时 waiters 保持挂起直到 activation deadline（默认
  30s）后失败，不提前 resolve。
- late ACK tombstone：已 settle 的 requestId 记 tombstone（默认上限 1024，
  满时拒绝新 late ACK 并计数）。
- broker 不拥有 claim truth：只持有 token，commit/abort 必须回 registry。

## 6. ActorInvocationRelay

- pending key：invocationId；每条 pending 记录 caller connection、owner
  fence（epoch/impl/lease/connection）、cancellationCorrelation、deadline。
- settle 规则：`actor.method.return` / `actor.method.error` 只接受来自
  精确 admitted owner（fence 全字段匹配 + 同 connection）；cancel 只接受
  来自 caller 且 cancellationCorrelation 匹配；同一 invocationId 二次
  settle 拒绝；settle 后转发给对端并进入 tombstone（默认上限 1024）。
- terminal：owner 断开 -> 该 owner 的全部 pending invocation 失败并通知
  caller（owner failure 或 terminal error）；caller 断开 -> 向 owner 发
  cancel；deadline 到 -> cancel + terminal；shutdown -> 全部 pending
  归零。
- relay 不改 registry（不 acquire/release/commit/abort），不处理
  owner-control ACK；admission（catalog + registry.admit）在 dispatch 前由
  W-actor 串行完成，relay 只做相关性。

## 7. ActorOwnerControlBroker

- pending key：requestId；记录 operation、fence、targetRuntimeId、精确
  connection、deadline timer。
- ACK 相关性（frozen）：`actor.owner.control.ack` 必须满足 requestId +
  runtimeId + operation + connection 全等；accepted -> resolve；
  拒绝/超时/断开 -> resolve(false)；late ACK tombstone（默认 1024）。
- 操作面：`MarkUpgrading`、`Discard`、`Activate`、`ActivateInitial`、
  `IdleEvict` 全部经 broker 关联；broker 不解释 idle 时机（归
  ActorLeaseExpiryScheduler），不改 registry truth（activateInitial 的
  commit/abort 由 activation broker 携 token 回 registry）。

## 8. ActorLeaseExpiryScheduler

- 拥有：lease/idle deadline 的调度与 eviction trigger；不拥有 actor
  registry truth、不拥有 control correlation。
- 默认：sweep tick 1s（FakeClock 可注入）；owner lease TTL 30s；
  idle TTL 30s；spawned actor method 使用其独立 deadline（300s）并延长
  lease（330s，TS 现有常量冻结，不新增 wire 字段）。
- sweep：`registry.expire(now)` 释放过期 lease -> 每个 expired fence 触发
  owner-control 清理；`registry.idle_candidates(now, idle_ttl)` ->
  对每个 candidate mint `evictionRequestId` + `registry.request_eviction`
  -> 经 `ActorOwnerControlBroker` 发 `IdleEvict`；eviction ACK ->
  `registry.acknowledge_eviction`；未 ACK 的 eviction 在后续 sweep 重试
  （有界重试，FakeClock 推进可测）。
- shutdown：所有 scheduler timer 取消、在飞 sweep 归零。

## 9. Byte-exact corpus 规格

位置：`runtime/transport/testdata/actor-wire/`。

### 9.1 frames.json（帧目录）

```json
{
  "schemaVersion": 1,
  "corpus": "actor-wire-v1",
  "frames": {
    "<frame-name>": {
      "direction": "RouterToRuntime | RuntimeToRouter",
      "frameType": "<actor 帧 type>",
      "decodeAs": "<typed header>",
      "payloadPresence": "empty | optional | required",
      "payloadBase64": "<payload bytes，absent 时为空>",
      "frameHex": "<完整二进制帧 hex>",
      "header": { "...": "typed header 语义 JSON" }
    }
  }
}
```

- `frameHex` 是本契约 byte-exact 事实；测试用 canonical codec
  decode 后 re-encode，必须逐字节相等（`encode(decode(hex)) == hex`）。
- 必选帧（测试断言存在）：getOrCreate.request/response/error、
  replace.request/response/error、find.request/response/error、
  remove.request/response/error、method.invoke/return/error/cancel、
  owner.invoke、owner.control（activateInitial）、owner.control.ack、
  owner.failure。

### 9.2 scenarios/（语义序列）

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "domain": "ownership | activation | invocation | control | lease",
  "events": [ { "op": "<domain op>", "...": "<args>" } ],
  "expect": { "...": "<frozen state assertion>" }
}
```

必选场景（测试同文件断言存在）：

- ownership：`claim-reserve-commit-single-owner`、
  `claim-reserve-conflict-while-owner-held`、`claim-abort-no-effect`、
  `claim-commit-twice-rejected`、`claim-reservation-not-owner`、
  `lease-expire-releases-fence`。
- activation：`get-or-create-first-joins-same-outcome`、
  `get-or-create-lineage-conflict`、`get-or-create-existing-no-reserve`、
  `get-or-create-ack-timeout-aborts-token`。
- invocation：`invoke-return-exact-owner`、`invoke-error-caller-forward`、
  `invoke-cancel-correlation`、`invoke-duplicate-settle-rejected`、
  `invoke-owner-disconnect-terminals-pending`。
- control：`control-ack-exact-correlation`、`control-ack-timeout-rejected`、
  `control-late-ack-tombstone`、`control-ack-wrong-operation-rejected`。
- lease：`lease-sweep-expire-and-idle-evict`、
  `lease-eviction-ack-clears-request`、`lease-eviction-retry-bounded`。

## 10. 与当前 TS/Rust wire 的差异记录（冻结目标）

| 表面 | 当前 wire/TS（main@7683b7c8） | 目标（本契约） | 收敛动作 |
| --- | --- | --- | --- |
| claim 语义 | TS registry store 以 epoch+fence+lease 原子获取 owner；无显式 `ActorClaimToken` 类型 | `ActorClaimToken` reserve/commit/abort 是唯一 claim 通道 | W-actor 实现 typed token；TS 语义作为 parity 参考 |
| catalog admission key | TS `RuntimeAssemblyActorMethodCatalog` 匹配 declarationOwner JSON + (abi, impl, method) | Rust view 用 A0 typed key（不含 File IR 坐标） | W-actor 按本契约实现；A2 parity 在 E-actor-parity 收敛 |
| wire 帧 | 与 §2.1 表一致（M0 后已有） | 不变 | corpus 固化 |
| activation dedup | `ActorGetCreateActivationCoordinator` claims map + lineage 冲突 | 同语义，token 化 | W-actor 实现 |

## 11. §5.4 contract pack 必填项

### 11.1 owner / invariant

- Owner：`ActorOwnershipRegistry`（claim truth）、`ActorMethodCatalogView`
  （只读查询）、`ActorActivationRequestBroker`（dedup/ACK 相关性）、
  `ActorInvocationRelay`（invocation 相关性）、`ActorOwnerControlBroker`
  （control 相关性）、`ActorLeaseExpiryScheduler`（deadline 调度）。
- Invariant：一个 actor key 至多一个 current owner fence；reservation 不是
  owner；token 只能 commit/abort 一次；broker 不持有 claim truth；catalog
  view 不读 File IR / 不建立独立 index；所有 pending 在 success/error/
  disconnect/saturation/shutdown 后归零。

### 11.2 typed inputs / outputs

- Inputs：actor wire 帧（§2.1）、`ActorClaimToken`、`RoutingEpoch` 捕获、
  `ActorOwnerFence`、lease/idle deadline 输入、`ActorActivationRequest`
  （get-or-create）、`ActorMethodInvokeFrameHeader`。
- Outputs：`ActorOwnerFence`、`ActorRef`、`ActorMethodFrame` 转发、
  `ActorOwnerControlAck` 结果、`ActorLeaseExpirySweepResult`、
  terminal/tombstone 事件、health 计数。

### 11.3 capacity

- writer queue：每 runtime session 沿用 C-session §5.3 预算（outbound
  256 帧 / 4 MiB，inbound 64 帧 / 1 MiB；owner 只 non-blocking reserve）。
- pending 上限：invocation relay 共享 `runtime.maxConcurrency`；
  activation claims / control pending 各 4096；late-ACK / settled
  tombstone 各 1024；eviction 重试有界（默认每 fence 至多 3 次未 ACK
  重试后 fail closed 上报）。

### 11.4 queue full

- data mailbox 满：reserved terminal slot 仍可入队（C-session 语义）；
- writer queue 满 / byte budget 超限：通过独立 abort handle 关闭 exact
  owner session，pending 按 disconnect terminal 收敛；
- relay/broker pending 达到上限：新 get-or-create / invoke / control
  立即拒绝（`Saturated` terminal），不排队无限等待。

### 11.5 timeout / disconnect / replacement / shutdown terminal

- timeout：activation deadline（默认 30s）、control ACK deadline（默认
  10s）、invocation deadline（wire `deadline.expiresAt`）、lease 过期 /
  idle TTL；超时对应精确 terminal。
- disconnect：owner/caller runtime 断开 -> relay pending 失败、
  control pending resolve(false)、activation waiters 保持到 deadline 后
  失败；registry 释放该 runtime 的 owner fence（lease 失效路径）。
- replacement：新 connection generation / 新 owner fence 到来时，旧
  fence 的 pending 按 IncarnationReplaced / OwnerUnavailable fail closed；
  old finalizer 不得删除 replacement。
- shutdown：全部 pending/tombstone/timer 归零；C-process-lifecycle
  第 5 步覆盖。

### 11.6 health fields

- catalog：epoch 捕获计数、query 命中/未命中、投影 schemaVersion；
- ownership：current owner fences、in-flight reservations、commit/abort
  计数、conflict 计数、lease expired/released；
- activation：dedup 合并数、lineage conflict、ACK pending/late/tombstone
  占用、activation timeout 计数；
- invocation：pending/active、duplicate settle 拒绝、owner disconnect
  terminal、deadline cancel、tombstone 占用；
- control：pending、timeout、late ACK、wrong-correlation 拒绝；
- lease：sweep 次数、expired、idle candidates、eviction pending/acked/
  retried/failed-closed。
- 日志/health 不含 Mongo URL、secret、业务 payload。

### 11.7 fake seam

- `FakeClock`（推进时间触发 deadline/lease/sweep）、`FakeActorRegistry`
  （内存 reference registry）、`FakeOwnerSocket`（可注入 ACK/断连/写失败）、
  `FakeCatalog`（内存 A0 projection mirror）。corpus 测试直接消费 fixtures
  + 参考模型；W-actor 实现必须用同一 fixtures 通过真实 codec/state。

### 11.8 real boundary probe（定义，W-actor/E-actor-rust 执行）

- `router-live:actor-ownership` probe：真实 Router + 两个 fake Runtime
  replica，完成 get-or-create（activateInitial ACK）→ claim commit →
  invoke/return → owner disconnect → replacement 注册；断言 claim truth
  唯一、pending 归零、tombstone 上限触发、lease sweep 后 eviction ACK
  链路。该 probe 成为 `router-rust-actor-live` 的 required managed CI。
