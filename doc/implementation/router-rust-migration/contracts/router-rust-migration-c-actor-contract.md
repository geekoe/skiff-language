# Router Rust Migration C-actor：actor lane handler 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-actor / E-actor-rust /
E-actor-parity 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`
  - §3.2（actor owner 职责表：catalog view / ownership registry /
    activation request broker / invocation relay / owner control broker /
    lease expiry scheduler 的唯一拥有与明确不拥有）；
  - §3.3（`ActorMethodCatalogView` 只查 caller 显式捕获的 epoch lease）；
  - §3.4（`ActorIncarnationFence`）、§3.6（session disconnect 的 consumer
    terminal）、§3.8（boundedness）、§5.4（C-actor + M-actor lane）、
    §5.5（`ActorFrameSink`）、§7（E-actor-rust / E-actor-parity）。
- A0 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-actor-leaf.md`。
- 同链契约：`router-rust-migration-c-model-actor-contract.md`（wire +
  model 边界）、`router-rust-migration-c-model-spawn-contract.md`、
  `router-rust-migration-c-spawn-contract.md`。

冲突时以权威设计为准；本文件只冻结契约与 corpus，不写 production。

## 1. 范围

冻结 actor lane 的 handler 级端口：`ActorMethodCatalogView` +
`ActorOwnershipRegistry` + `ActorActivationRequestBroker` +
`ActorInvocationRelay` + `ActorOwnerControlBroker` +
`ActorLeaseExpiryScheduler` 的 §5.4 必填项，作为 W-actor 实现与
E-actor-rust 验收的冻结目标。wire shape / typed model 见
C-model-actor；spawn 侧见 C-model-spawn / C-spawn。

## 2. Owner / invariant

| Owner | 唯一拥有 | 明确不拥有 |
| --- | --- | --- |
| `ActorMethodCatalogView` | 对显式捕获 `Arc<RoutingEpoch>` 中 A0 actor index 的 typed query | 独立 index、mailbox、refresh/publication、actor live state、File IR 读取 |
| `ActorOwnershipRegistry` | actor identity、incarnation、current owner fence、claim reservation/commit/abort | activation request correlation、invocation correlation、timer |
| `ActorActivationRequestBroker` | get-or-create dedup、activation request/ACK correlation | actor key 上的 claim truth、invocation、lease scheduling |
| `ActorInvocationRelay` | method invocation/return/error/cancel correlation | owner registry mutation、owner-control ACK |
| `ActorOwnerControlBroker` | claim/renew/evict 等 owner-control correlation | method invocation、idle timing |
| `ActorLeaseExpiryScheduler` | lease/idle deadline 调度与 eviction trigger | actor registry truth、control correlation |

Invariant（全部必须有 sequence test）：

1. 一个 actor key 至多一个 current owner fence；`ActorClaimToken`
   reserve/commit/abort 是唯一 authoritative transition；broker 不持有
   claim truth。
2. catalog view 只读 immutable epoch；epoch 内 index 一次构造，不存在
   独立 refresh；查询结果与 epoch 捕获一致。
3. invocation/control/activation 相关性全部 exact-fence；任何 stale /
   duplicate / wrong-correlation 帧 fail closed，不产生 registry 效果。
4. success/error/disconnect/saturation/shutdown 后所有 pending、tombstone、
   timer 归零。

## 3. Typed inputs / outputs

### 3.1 ActorMethodCatalogView

- Inputs：`Arc<RoutingEpoch>`（含 A0 `ActorRoutingProjection`）、
  `CatalogQuery { actor: ActorRoutingRef, actor_implementation_identity,
  method_identity }`。
- Outputs：`Option<ActorRoutingMethod>`（含 deployment/package exact
  binding）；`has_method` 布尔。
- 非目标：不接收 declarationOwner / modulePath / actorName / methodName /
  sourceSpan / File IR 坐标作为查询输入。

### 3.2 ActorOwnershipRegistry

- Inputs：`ReserveClaim { actor_key, expected_epoch, owner_runtime_id,
  route_authority }`、`CommitClaim { token, activation_fence_facts }`、
  `AbortClaim { token }`、`RenewLease { fence, ttl }`、
  `ReleaseLease { fence, reason }`、`ExpireLeases { now }`。
- Outputs：`ActorClaimToken`、`ActorOwnerFence { epoch, owner_runtime_id,
  owner_lease_id, lease_expires_at, actor_abi_identity,
  actor_implementation_identity, declaration_owner }`、`ClaimConflict`、
  `ExpiredOwner { fence, ... }`。

### 3.3 ActorActivationRequestBroker

- Inputs：`ActorGetOrCreateRequest`（wire header + bootstrap payload）、
  lineage、`ActorClaimToken`、activateInitial ACK（经 control broker）。
- Outputs：`ActorRef`（成功）、`ActorGetCreateFailure`（含 lineage
  conflict）、join 结果。

### 3.4 ActorInvocationRelay

- Inputs：`ActorMethodInvokeFrameHeader` + arguments payload、
  `ActorMethodReturnFrameHeader` + payload、`ActorMethodErrorFrameHeader`、
  `ActorMethodCancelFrameHeader`、`ActorOwnerFailureFrameHeader`、
  owner/caller disconnect、deadline 事件。
- Outputs：转发帧（caller/owner 方向）、`InvocationTerminal`、
  `InvocationSettled`、tombstone 事件。

### 3.5 ActorOwnerControlBroker

- Inputs：`OwnerControlRequest { request_id, operation, fence,
  target_runtime_id, connection, deadline }`、
  `ActorOwnerControlAckFrameHeader`、timeout/disconnect。
- Outputs：`OwnerControlOutcome { accepted: bool, reason? }`、late-ACK
  事件。

### 3.6 ActorLeaseExpiryScheduler

- Inputs：`now`/`FakeClock`、sweep tick、eviction ACK。
- Outputs：`ActorLeaseExpirySweepResult { expired, eviction_requests }`、
  `EvictionAcknowledged`、`EvictionRetryExhausted`。

## 4. Capacity

- invocation relay pending：共享 `runtime.maxConcurrency`（C-config
  required）；超出立即 `Saturated` 拒绝。
- activation claims：4096；control pending：4096；每个 broker 的
  late/settled tombstone：1024（满时拒绝新 late 帧并计数）。
- writer queue：每 runtime session outbound 256 帧 / 4 MiB、inbound
  64 帧 / 1 MiB（C-session §5.3 同预算）；owner 只 non-blocking
  reserve/`try_send`。
- lease scheduler：默认 sweep tick 1s、owner lease TTL 30s、idle TTL 30s、
  spawned actor method deadline 300s / lease 330s（TS 现有常量，冻结）；
  eviction 未 ACK 重试上限 3 次。

## 5. Queue full

- data mailbox 满：reserved terminal slot 仍可入队（C-session）；writer
  queue 满 / byte budget 超限：abort exact owner/caller connection，pending
  按 disconnect terminal 收敛，不等待队列接受 close 帧。
- broker/relay pending 上限：立即拒绝新操作（`Saturated`），不排队。
- tombstone 满：拒绝新 late ACK / settled 帧并计数，不影响 live
  correlation。

## 6. Timeout / disconnect / replacement / shutdown terminal

| 事件 | terminal（frozen） |
| --- | --- |
| activation deadline（默认 30s） | waiters 全部失败；registry.abort(token)；claim 清除 |
| control ACK deadline（默认 10s） | resolve(false)；late ACK 只进 tombstone |
| invocation deadline（wire expiresAt） | 向 owner 发 cancel；caller 收到 deadline terminal |
| lease 过期 | registry.expire 释放 fence；失败 invocation 以 OwnerUnavailable/Upgrading 语义收敛 |
| idle TTL | scheduler mint evictionRequestId；IdleEvict 经 control broker；ACK 后 registry.acknowledge |
| owner disconnect | relay pending 全部失败并通知 caller；control pending resolve(false)；activation waiters 保持到 deadline 后失败；registry release owner lease |
| caller disconnect | 向 owner 发 cancel；relay pending 清除 |
| replacement（新 connection/新 fence） | 旧 fence pending 按 IncarnationReplaced / OwnerUnavailable fail closed；old finalizer 不删除 replacement |
| shutdown | 全部 pending/tombstone/timer 归零（C-process-lifecycle 第 5 步） |

## 7. Health fields

- catalog：epoch 捕获数、query 命中/未命中、schemaVersion 计数；
- ownership：current fences、in-flight reservations、commit/abort/
  conflict/expired/released 计数；
- activation：dedup 合并、lineage conflict、pending/tombstone 占用、
  timeout；
- invocation：pending/active、duplicate settle 拒绝、owner disconnect
  terminal、deadline cancel、tombstone 占用；
- control：pending、timeout、late ACK、wrong-correlation；
- lease：sweep、expired、idle candidates、eviction pending/acked/retried/
  exhausted。
- 不暴露 Mongo URL、secret、业务 payload。

## 8. Fake seam

- `FakeClock`、`FakeActorRegistry`（reference registry）、
  `FakeOwnerSocket` / `FakeCallerSocket`（可注入 ACK/断连/写失败）、
  `FakeCatalog`（A0 projection mirror）、`FakeLeaseSchedulerTransport`。
- corpus：`runtime/transport/tests/actor_wire_corpus.rs`（byte-exact +
  场景状态机）、`actor_owner_contract.rs`（六个 owner 的参考模型 sequence
  tests）、`actor_catalog_view_contract.rs`（A0 projection mirror +
  catalog view）。

## 9. Real boundary probe（定义，W-actor/E-actor-rust 执行）

- `router-live:actor-ownership`：真实 Router + 两个 fake Runtime replica：
  get-or-create（activateInitial ACK）→ claim commit → invoke/return →
  owner disconnect → replacement 注册；断言 claim truth 唯一、pending
  归零、tombstone 上限、lease sweep → eviction ACK 链路。
- `router-live:actor-invocation`：真实 codec 帧流：invoke → owner.invoke →
  method.return；错序/duplicate/owner 断连/替换竞态全部 fail closed；
  shutdown 后 pending 归零。
- 以上 probe 成为 `router-rust-actor-live` 的 required managed CI
  （M-actor + W-actor 完成后解锁）。

## 10. W-actor 交付义务（非本 pack 实现）

1. 消费 C-model-actor corpus：全部 positive 通过、negative fail closed。
2. 实现六个 owner 的 Rust 类型与 port conformance（工厂/端口固定形态，
  见 §5.5），并保持 §3.2 invariant 与 §5.4 必填项。
3. M-actor gate：真实 Router consumer 直接消费同一 corpus；actor
  invocation/control/lease/timer 在 success/error/disconnect/
  saturation/shutdown 后全部归零（E-actor-rust 证据）。
4. A2 parity：TS/Rust differential 不再拿 File IR reader 作为 baseline
  （E-actor-parity）。
