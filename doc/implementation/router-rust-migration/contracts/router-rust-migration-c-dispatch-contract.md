# Router Rust Migration C-dispatch：admission + RequestDispatcher 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-dispatch / W-http / E-dispatch /
E-http 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md` §3.2
  （`RuntimeAdmissionPool`：per-session capacity permits、selection
  cursor/policy；`RequestDispatcher`：ordinary unary/stream 与 derived
  function-spawn correlation、terminal、reservation token；actor-method
  spawn 归 actor lane）、§3.3（capture → query → reserve → revalidate →
  enqueue → terminal 一次释放；pending 持 epoch/lease/permit）、§3.6
  （session cancellation 是所有基于 session 的 pending 的共享 terminal
  观察）、§3.8（boundedness、deadline dequeue/admission/dispatch 前重检）、
  §5.4（C-dispatch + M-request → W-dispatch + W-http + E-dispatch/E-http）。
  冲突时以权威设计为准。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-request-leaf.md`。
- 同链契约：`router-rust-migration-c-model-request-contract.md`
  （request wire 帧与 stream 顺序）、
  `router-rust-migration-c-routing-query-contract.md`
  （`RegisteredSessionLease` 候选投影）、
  `router-rust-migration-c-session-contract.md`
  （session cancellation/barrier、consumer manifest）。

## 1. 冻结范围

冻结 ordinary request 的 **admission 流水线** 与
`RequestDispatcher` 的 **pending/terminal/function-spawn correlation**：

- capture → candidate query → reserve permit → enqueue 前 revalidate →
  enqueue → terminal 恰好释放一次（§3.3 第 1/4/5/6 步）；
- per-session capacity permit、selection cursor/policy、queue full；
- pending 生命周期：unary / stream / derivedSpawn 三种 dispatcher-owned
  kind；全部 terminal source 与 cancel 帧规则；
- function-spawn correlation：request parent（dispatcher pending）与
  actor-method parent（actor lane）的不可碰撞 typed 命名空间；派生 spawn
  的 admission；
- timeout/disconnect/replacement/shutdown terminal；
- health fields 与 fake seam；真实边界 probe。

非目标：不定义 `RuntimeCandidateQuery` 投影规则（C-routing-query）；
不定义 actor-method invocation relay/ownership（contracts-actor）；
不定义 client WS broker/generation pin（contracts-ws）；不写 production。

## 2. Owner / invariant（§3.2 冻结）

- `RuntimeAdmissionPool`：**唯一拥有** per-session capacity permits 与
  selection cursor/policy；**不拥有** session truth、request pending、
  active routing epoch。
- `RequestDispatcher`：**唯一拥有** ordinary unary/stream 与 derived
  function-spawn correlation、pending/terminal、reservation token；
  **不拥有** actor-method invocation（归 `ActorInvocationRelay`/actor
  authority path）、peer WS correlation、socket。
- Invariant：pending 始终持有 capture 的 routing epoch + registered session
  lease + admission permit；permit 在 terminal 时恰好释放一次；任何
  session 取消（disconnect/replacement）都终结该 session 的全部 pending；
  queue full / revalidate 失败 / 无候选都 fail closed（不排队、不降级）。

## 3. Admission 流水线（冻结）

```text
1. capture: 一次原子捕获当前 Arc<RoutingEpoch>
2. query:   RuntimeCandidateQuery -> Vec<RegisteredSessionLease>
3. select:  AdmissionPool 从 exact leases 中按 cursor 选择有容量 session
4. reserve: 对该 session 原子 reserve 一个 capacity permit
5. revalidate: enqueue 前原子重检 session epoch、registration revision、
   exact tuple、cancellation（§3.3 第 5 步）
6. enqueue: 成功则 pending 持有 epoch + lease + permit，并向 runtime
   writer 发送 request.start
7. terminal: 终态到达时 detach pending 并恰好释放 permit
```

- capture 与 revalidate 之间旧 epoch 通过已捕获 `Arc` 延续（无全局 pin）。
- revalidate 失败（cancelled / revision 变化 / tuple 变化）：**释放 permit
  并重选**下一个 exact candidate；无剩余候选则 fail closed
  （`ProviderUnavailable`），不 enqueue。
- 选择策略（冻结参考策略）：round-robin cursor 遍历 exact candidates，
  跳过无容量 session；W-dispatch 的等价实现必须通过同一 corpus 的可观测
  结果。capacity = `runtime.maxConcurrency`（per session/connection，
  C-config 冻结值），`RuntimeDispatcherOptions.maxConcurrency` 与 TS
  `assertConnectionAdmission` 一致。
- requestId 唯一：同 requestId 已 pending → `ServiceProtocolBoundaryError`
  fail closed（TS `assertRequestIdAvailable` 语义）。
- 发送前 deadline 重检：`deadline.timeoutMs`/`expiresAt` 已过期或剩余为 0
  → 拒绝 enqueue（TS `runtimeDispatchTimerMs` 语义）。

## 4. Pending / terminal（冻结）

### 4.1 pending kinds

| kind | 触发 | terminal 语义 |
| --- | --- | --- |
| `unary` | HTTP unary `request.start` | `response.end`（completed）/ `response.error`（failed）/ 取消类 terminal |
| `stream` | HTTP serverStream `request.start` | `response.start` → chunks → `response.end`（completed）/ `response.error` / 取消类 terminal |
| `derivedSpawn` | function spawn 派生请求（§5） | 空 `response.end`（completed）/ `response.error` / 取消类 terminal |

stream 状态机沿用 C-model-request §5.4：waitingStart → streaming →
terminal；任何违规 → `protocol_error` terminal 并发送
`request.cancel`（reason `protocol_error`）。

### 4.2 terminal sources（冻结，含 TS 语义）

`runtime_response_end`、`runtime_response_error`、`runtime_request_cancel`
（Runtime 主动取消）、`timeout`、`caller_abort`、`client_disconnect`、
`backpressure`、`protocol_error`、`callback_error`（写失败）、
`runtime_disconnect`（含 session replacement 触发的旧 session 取消）、
`router_shutdown`。

### 4.3 cancel 帧规则（Router→Runtime）

| terminal source | 发送 request.cancel？ | wire reason |
| --- | --- | --- |
| runtime_response_end / runtime_response_error / runtime_request_cancel / runtime_disconnect | 否 | — |
| timeout | 是 | `timeout` |
| caller_abort | 是 | `caller_cancel`（或调用方指定 reason） |
| client_disconnect | 是 | `client_disconnect` |
| backpressure | 是 | `backpressure` |
| protocol_error / callback_error | 是 | `protocol_error` |
| router_shutdown | 是 | `router_shutdown` |

unknown cancel reason（非 C-model-request §4 词表）：拒绝帧并把该 pending
按 `protocol_error` terminal（不猜测 owner）。

### 4.4 stale / fence

- response 帧的 requestId 必须命中 pending，且来自同一 session socket
  （exact fence）；否则忽略，不产生副作用（TS `isPendingRuntimeSocket`
  语义）。
- 同一 session 的 response.end 必须通过 unary/stream 各自的 phase 校验
  （C-model-request §5.2）；失败 → `protocol_error` terminal。

## 5. Function-spawn correlation（冻结）

### 5.1 两个 typed parent 命名空间

```text
request parent:      callerRequestId ∈ RequestDispatcher pending
actor-method parent: callerRequestId ∈ ActorInvocationRelay 活跃 invocation
```

- 解析顺序与互斥（TS `requireSpawnParent` 语义）：请求 parent 与 actor
  parent **同时命中 → fail closed**（ambiguous）；两者都未命中 → fail
  closed；只命中其一 → 使用该 parent。
- request parent 必须满足 exact authority：pending.request 的
  routing（assemblyIdentity/assemblyGeneration/deployment 全字段）与
  capture 的 `RuntimeSpawnParentAuthority` 一致，且 parent 所在 runtime
  连接与当前连接 exact 相同。
- actor-method parent 由 `ActorMethodSpawnControl.activeActorInvocationParent`
  （actor lane）提供；dispatcher 不持有 actor invocation pending。

### 5.2 derived function spawn

- `targetKind == function` 的 spawn.submit 由 dispatcher 建立
  `derivedSpawn` pending（独立 requestId，`spawn-request-*` 命名空间），
  走同一 admission 流水线（占 per-session capacity permit）。
- 派生请求的 deadline 从 parent request 的 deadline 派生（取 parent
  剩余时间与默认派生 timeout 的较小者，TS `derivedSpawnDeadline` 语义）。
- 派生 spawn 的 `response.end` 必须为空（payload/metadata 均不得出现）；
  违规 → `protocol_error` terminal（TS `SpawnResponseProtocolError`）。
- `targetKind == actorMethod`：由 `ActorMethodSpawnControl.submitSpawn`
  直接处理，**不进入 dispatcher pending、不占 dispatcher permit**。

## 6. Corpus 规格

位置：`runtime/transport/testdata/dispatch-admission/scenarios/`。

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "maxConcurrency": 1,
  "epoch": { "environment": "prod", "generation": 42,
             "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 hex>",
             "configSnapshotId": "skiff-runtime-config-snapshot-v1:<32 hex>",
             "deployment": { "serviceId": "example.com/service-1",
               "contractVersion": "1.0.0", "deploymentRevision": "deployment-1",
               "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:<64 hex>" } },
  "sessions": [
    { "id": "s1", "replicaId": "runtime-a", "connectionGeneration": 1,
      "revision": 1, "cancelled": false,
      "tuple": { "environment": "prod", "generation": 42,
                 "assembly": "skiff-runtime-assembly-v3:sha256:<64 hex>",
                 "configSnapshot": "skiff-runtime-config-snapshot-v1:<32 hex>" },
      "capabilities": ["unary", "serverStream"] }
  ],
  "actorInvocationParents": { "invocation-1": { "session": "s1" } },
  "events": [
    { "kind": "request", "requestId": "req-1", "mode": "unary",
      "preferSession": "s1", "revalidateOutcome": "ok" },
    { "kind": "responseEnd", "requestId": "req-1" }
  ],
  "expect": {
    "requestOutcomes": { "req-1": "completed" },
    "terminalSources": { "req-1": "runtime_response_end" },
    "cancelFrames": [],
    "permitsHeld": 0,
    "releases": 1,
    "actorLaneSpawns": 0,
    "derivedSpawns": 0,
    "failStop": false
  }
}
```

事件 kinds：`request`（含 `revalidateOutcome = ok | fail-cancelled |
fail-stale-revision`，`preferSession` 可选）、`responseStart`、
`responseChunk`、`responseEnd`（`payloadPresent` 可选）、`responseError`、
`runtimeCancel`、`timeout`、`clientAbort`、`disconnect`、`replacement`
（old/new session）、`shutdown`、`spawnFunction`、`spawnActorMethod`。

必选场景：

- `unary-completed-releases-permit`
- `unary-response-error-failed`
- `stream-start-chunk-end-completed`
- `stream-protocol-error-terminates-and-cancels`
- `queue-full-fail-closed`
- `request-id-duplicate-fail-closed`
- `no-candidate-fail-closed`
- `revalidate-fail-cancelled-reselect`
- `revalidate-fail-stale-revision-reselect`
- `selection-cursor-round-robin`
- `timeout-terminates-and-cancels`
- `runtime-cancel-no-cancel-frame`
- `client-abort-cancels`
- `runtime-disconnect-terminates-all-pending`
- `replacement-terminates-old-pending`
- `shutdown-terminates-all-pending`
- `function-spawn-derived-pending`
- `actor-method-spawn-actor-lane`
- `spawn-ambiguous-parent-rejected`

消费测试：`runtime/transport/tests/dispatch_admission_corpus.rs`
（reference admission/pending 模型逐场景断言）。

## 7. §5.4 contract pack 必填项

### 7.1 owner / invariant

- Owner：`RuntimeAdmissionPool` + `RequestDispatcher`（§2 边界）。
- Invariant：permit 生命周期严格配对（reserve→terminal 恰好一次释放）；
  pending 集合 = dispatcher-owned correlation 全集；queue full/无候选/
  revalidate 失败一律 fail closed；session 取消终结该 session 全部 pending；
  actor-method spawn 永不进入 dispatcher pending。

### 7.2 typed inputs / outputs

- Inputs：`DispatchRequest { header: RuntimeAssemblyRequestStartFrameHeader,
  payload_bytes, timeout, cancel_signal }`、response 帧（
  ResponseStart/Chunk/End/Error）、`SpawnSubmit { caller_request_id,
  target_kind, target }`、`SessionClosed(RuntimeSessionEpoch)`、
  `Shutdown`。
- Outputs：`PendingTerminal { request_id, source, kind }`、`ReservationToken`
  / `PermitReleased`、`request.cancel` 帧、`DerivedSpawnResult`、
  `ActorMethodSpawnDispatch`（转发 actor lane）、
  `DispatcherHealthSnapshot`。

### 7.3 capacity

- per-session in-flight ≤ `runtime.maxConcurrency`（per connection）；
  pending 总量 ≤ sessions × maxConcurrency；requestId map 为 bounded
  mailbox（无 unbounded queue）。
- derivedSpawn 占同一 per-session capacity；actor-method spawn 不占。

### 7.4 queue full

- 容量满时新 request **立即 fail closed**（`ProviderUnavailable`），不排队、
  不 reserve 后悬挂；permit 不泄漏。
- writer queue 满 → `callback_error` terminal + abort session
  （C-session writer 契约），cancel 帧不等待队列接受。

### 7.5 timeout / disconnect / replacement / shutdown terminal

- timeout：deadline 到点 → terminal + `request.cancel(timeout)`；deadline 在
  dequeue/admission/dispatch 前重检（§3.8）。
- disconnect：session cancellation token 触发 → 该 session 全部 pending
  terminal（`runtime_disconnect`），不发送 cancel 帧；barrier 归 C-session。
- replacement：旧 session cancel + barrier 后新 session 才 current；旧
  session pending 走 disconnect terminal；新 session 独立容量。
- shutdown：`close()` 终结全部 pending（`router_shutdown` + cancel 帧），
  之后拒绝新 admission；dispatcher 计数归零可观测（C-process-lifecycle
  第 6 步）。

### 7.6 health fields

- `dispatcher.pending.{unary,stream,derivedSpawn}`；
- `dispatcher.terminal.{bySource}`（11 类 source 计数）；
- `admission.{permitsHeld,releases,queueFullRejects,revalidateFailures,
  reselects,noCandidateRejects,duplicateRequestIdRejects}`；
- `spawn.{derivedSpawns,actorLaneSpawns,ambiguousRejects}`。
- health 不暴露 payload/requestId/secret。

### 7.7 fake seam

- `FakeCandidateQuery`（固定 leases）、`FakeAdmissionPool`、
  `FakeRuntimePeer`（帧级读写）、`FakeFrameSender`（可注入写失败）、
  `FakeClock`、`FakeActorMethodSpawnControl`（固定 parent/ambiguity 注入）、
  `FakeSessionCancellation`。corpus 测试使用 fixtures + reference 模型；
  W-dispatch 必须用同一 fixtures。

### 7.8 real boundary probe（定义）

- `router-live:dispatch`（W-dispatch 交付后成为 `router-rust-dispatch-live`
  managed probe）：loopback 启动真实 listener（C-net），fake ingress →
  `RequestDispatcher` → fake Runtime peer；按 request-wire +
  dispatch-admission corpus 断言：unary/stream 完成、timeout/disconnect/
  replacement/shutdown terminal、cancel 帧字节（`request.cancel` reason）、
  function-spawn derived pending 与 actor-method 转发、permit/release
  计数归零。E-http 再接真实 Runtime。

## 8. W-dispatch / W-http 交付义务（非本包实现）

1. 实现 `RuntimeAdmissionPool` + `RequestDispatcher` 并消费本 corpus 全部
   场景 + C-routing-query corpus。
2. 实现 §4.3 cancel 帧规则、§5 spawn correlation（含 ambiguous 拒绝与
   actor lane 转发）。
3. 与 C-model-request/W-model-request 共用同一 frame corpus；不复制 codec。
4. 与 C-session 的 session cancellation/barrier 对接：pending 观察
   session terminal，permits 归零；与 C-process-lifecycle 停机顺序对接。
