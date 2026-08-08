# Router Rust Migration C-ws：RuntimeGenerationPinLedger + WebSocketRequestBroker 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-WebSocket / E-ws 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md` §2.1
  （WS peer-RPC correlation 只归 `WebSocketRequestBroker`，不与 ordinary
  pending 共用）、§3.2（`ClientConnectionIndex` /
  `RuntimeGenerationPinLedger` / `WebSocketRequestBroker` owner 边界）、
  §3.7（client finalization 四步与 broker detach/tombstone）、§3.8
  （bounded mailbox、frame/byte permit、deadline 重检）、§5.4（必填项）、
  §7（E-ws：broker correlation、deadline、tombstone、captured writer
  fence、late result isolation）。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-ws-leaf.md`。
- 同链契约：`router-rust-migration-c-model-connection-contract.md`、
  `router-rust-migration-c-client-lifecycle-contract.md`。

冲突时以权威设计为准；本文件只冻结契约，不写 production。

## 1. 范围

冻结 `RuntimeGenerationPinLedger`（Runtime→Router 的
`websocket.generation.lifecycle` Acquire/Release/Ack/Reject 消费：
pending/cache/session attachment）与 `WebSocketRequestBroker`（outbound
peer correlation、inbound dispatch correlation、deadline、tombstone、
captured writer fence、capacity）。不定义 client index/replacement
（C-client-lifecycle）、wire 字节（C-model-connection）。

## 2. Identity 与 typed 类型（§3.4，冻结）

```text
RuntimeGenerationPin { router_session_id, service_id, assembly_identity,
                       assembly_generation, websocket_entry_id,
                       connection_id }（wire tuple，冻结）
RuntimeGenerationLeaseId = websocket.generation.lifecycle requestId
BrokerConnectionGeneration { connection_id, socket_generation, service_id,
                             websocket_entry_id, profile }
InboundExecutionToken { connection_id, socket_generation, sequence }
BrokerRuntimeSource { sender, session_token, respond }
OpaquePeerId = string | safe integer（canonical，见 C-model-connection §5.2）
```

- `RuntimeGenerationPin` 是 Router 侧 ledger 状态，不是 `RuntimeSessionEpoch`
  的别名；release 只 pin exact connection。
- `BrokerConnectionGeneration` 与 `ClientSocketGeneration` 同界但用途
  不同：前者是 broker 的 generation 句柄，后者是 lifecycle 的世代。

## 3. RuntimeGenerationPinLedger（冻结）

### 3.1 状态

```text
expected_by_connection_id: connection_id -> PinExpectation
acquired_by_connection_id: connection_id -> { tuple, runtime_sender }
pending_release_by_connection_id / by_request_id
cached_acquire_by_request_id
router_session_by_runtime / runtime_by_router_session
release_ack_count_by_runtime
```

### 3.2 acquire

- `expectConnection`：admission dispatch 前注册 exact expectation；
  duplicate connection_id → fail-stop（进程错误）。
- Runtime 发 Acquire（`sender=runtime`，wire 校验见
  C-model-connection §4）：
  - cached requestId 命中：同 sender 同 tuple → 返回缓存 response；
    否则 reject `request-conflict`。
  - session attachment：`routerSessionId` 已绑其它 runtime → reject
    `sender-mismatch`；当前 runtime 已绑不同 session → reject
    `sender-mismatch`。
  - 无 expectation → reject `not-acquired`；tuple 与 expectation 不匹配
    → reject `tuple-mismatch`；pending admission sender 不匹配 → reject
    `sender-mismatch`；已 acquired 且不同 sender/tuple → reject
    `tuple-mismatch`。
  - 成功：绑定 routerSession↔runtime、写 `acquired`、缓存 response、
    Ack（exact echo）。

### 3.3 release / pending / timeout

- `releaseConnection(connection_id)`：同一 connection 已 pending → 复用
  同一 promise；从 expectation 移除；未 acquired 或 socket 非 OPEN →
  立即 resolve（不写帧）。
- 已 acquired：构造 Release（Router→Runtime），挂 pending
  （by_connection_id + by_request_id），启动 release timeout（默认
  5s）；send 失败 → finish error。
- Runtime Ack（release）：exact echo 校验；ack 计数 +1，resolve；
  Reject → finish error + runtime socket close 1008；timeout →
  finish error + runtime socket close 1008
  （`websocket generation release timed out`）。
- Runtime disconnect：清该 runtime 的 ack 计数与 acquired expectation，
  finish 全部 pending release（resolve），通知 connection lost 回调。
- `flush`：等待全部 pending；任一失败 → AggregateError（gateway
  shutdown 失败）。release timeout 完成 client terminal 后不得保留
  pin；Runtime 侧按 protocol-unavailable 关闭。

### 3.4 health

- `connectionPinCount(runtime)` = acquired + pending release；`releaseAckCount`
  = 该 runtime 累计 ack；pending by connection/request、cached acquire、
  release failures。

## 4. WebSocketRequestBroker（冻结）

### 4.1 状态与 owner 边界

```text
generations: handle -> { identity, uid, owner_token, adapter, writer, open,
                         outbound_active, inbound_active }
outbound_by_peer / outbound_by_runtime / inbound_by_peer
outbound_tombstones / inbound_tombstones（FIFO + TTL + capacity）
```

- peer correlation 只在这里；`RequestDispatcher` 的 ordinary pending 与
  WS pending 不得共用 map。
- 每个 generation 一个 captured writer；writer 失败只 settle exact
  correlation，不关闭其它 generation。

### 4.2 outbound（Runtime request → peer）

1. generation closed → respond `connectionUnavailable`；
2. service/entry/ownerToken 不匹配 → `connectionUnavailable`；
3. 空 requestId/method 或 profile 不匹配 → `protocolError` +
  runtime protocol violation；deadline 非法 → 同上；
4. 重复 runtime key（sender+sessionToken+requestId）→ `protocolError` +
  violation；
5. capacity：`outbound_by_peer.size >= outboundGlobalCapacity`（默认
  4096）或 per-generation `outboundActive >= 128` → `resourceLimit`，
  不写 peer、不加 tombstone；
6. 生成 peer id（`<socketGeneration>:<seq>`），检查 peer key 与 active/
  tombstone 冲突 → `protocolError` + violation；
7. 注册双索引 + outboundActive；deadline 存在则 arm timer；
8. 写 peer；writer 失败 → settle `transportUnavailable`（恰好一次）。

### 4.3 cancel / runtime disconnect

- `connection.request.cancel`：按 runtime key detach exact outbound
  （tombstone、不写 peer），返回是否命中。
- runtime disconnect：detach 该 sender+sessionToken 的全部 outbound
  （不写 peer、加 tombstone），返回数量。

### 4.4 inbound（peer request → Runtime）

1. duplicate peer key（active 或 tombstone）→ close generation 1002
   `duplicate JSON-RPC request id`；
2. method 未声明 → tombstone + 写 `methodNotFound`；capacity 满 →
   tombstone + `serverBusy`（-32000）；二者都不 dispatch；
3. 接受：创建 `InboundExecutionToken` + AbortController + inbound 条目，
   arm deadline（默认 120s）；
4. dispatch 结果映射：success→result；invalidParams→-32602；
   internalError/runtimeUnavailable→-32603；deadlineExceeded→timeout
   （-32001）+ abort；
5. detach 先于写 terminal（写失败不会 reopen）；late dispatcher
   completion 幂等忽略。

### 4.5 response / platform error / close

- peer response：命中 outbound → map 并 settle（runtime respond 成功
  与否都不 reopen）；未命中且 tombstoned → 静默忽略；未命中且无
  tombstone → close generation 1002 `unknown JSON-RPC response id`；
- peer platform error：id=null → 写 platform error 帧；id 合法 →
  先 tombstone 再写错误；
- binary → close 1003；peer close → close generation
  `transportUnavailable`（abort inbound、settle outbound，不写 close
  帧）；peer 文本超预算/非法 → close 1009/1002。

### 4.6 close generation（finalization 第 3 步）

```text
open=false；detach 全部 outbound/inbound（各加 tombstone）；
删除 generation；removeGeneration tombstones；abort 全部 inbound；
respond 全部 outbound transportUnavailable/protocolError；
writer.close(code, reason)（仅显式 close 时）
```

### 4.7 tombstone

- capacity 默认 4096、TTL 默认 60s；FIFO eviction；`removeGeneration`
  在 generation close 时清理；late response 在 tombstone 存续期忽略，
  eviction 后同一 peer id 可复用（旧 execution token 仍由
  `InboundExecutionToken` fence）。

### 4.8 capacity / budget

- outbound global 4096、per-generation 128；inbound global 4096、
  per-generation 128；tombstone 各 4096/60s；inbound timeout 120s；
  profile limits 见 C-model-connection §5.3；writer byte budget 见
  C-client-lifecycle §3.3。

## 5. §5.4 contract pack 必填项

### 5.1 owner / invariant

- Owner：`RuntimeGenerationPinLedger`（Runtime generation pin 唯一
  owner）、`WebSocketRequestBroker`（peer correlation 唯一 owner）。
- Invariant：pin release 至多一次且超时/失败不静默保留；peer id 在
  active+tombstone 窗口内唯一；detach/settle 恰好一次；所有 pending/
  tombstone/timer 在 terminal 后归零；late result 被 tombstone fence
  隔离。

### 5.2 typed inputs / outputs

- Inputs：`expectConnection`、Acquire/Release/Ack/Reject、
  `attachGeneration`、`handleRuntimeRequest`、`handleRuntimeCancel`、
  `handleRuntimeDisconnect`、`handlePeerText`、`handlePeerBinary`、
  `handlePeerDisconnect`、`dispatchInbound` result、clock。
- Outputs：Ack/Reject、`BrokerRuntimeResponse`、peer text frame、
  `InboundDispatchAction`、close 码、`WebSocketRequestBrokerSnapshot`、
  flush 结果。

### 5.3 capacity

见 §4.8；release timeout 5s、connectionLimit/slow-client budget 见
C-client-lifecycle §6.3。

### 5.4 queue full

- broker mailbox/data 满 → `resourceLimit`（outbound）/ `serverBusy`
  （inbound），tombstone 后写 terminal；terminal 保留容量不受 data
  满影响；writer 满 → 1011 slow-client（C-client-lifecycle）；
  barrier ACK 超时 → fail-stop（C-process-lifecycle）。

### 5.5 timeout / disconnect / replacement / shutdown terminal

- deadline：outbound → `deadlineExceeded`（一次）；inbound →
  timeout + abort；release timeout → 1008 + flush failure。
- disconnect：runtime → outbound detach + pin release；peer →
  close generation + finalizer（C-client-lifecycle §4）。
- replacement：old generation detach + pin release，new generation 独立；
  old finalizer 不触碰 new。
- shutdown：drain finalizers → flush ledger → broker 全 detach；
  残留 pending 归零；barrier 超时 fail-stop。

### 5.6 health fields

- generationCount、outbound/inbound active（global + per-generation）、
  tombstone 数（outbound/inbound）、timerCount、terminalLeaseCount、
  pin acquired/pending release/release ack、flush failures；日志/health
  不含业务 payload/params/result/完整 query/Authorization/cookie。

### 5.7 fake seam

- `FakePeerWriter`（write/close 记录、注入失败）、`FakeClock`（injected
  time + timer 手动触发）、`FakeRuntimeSource`（respond 记录）、
  `FakeDispatchInbound`（同步/异步/迟到 terminal）、`FakeLedgerPeer`
  （ack/reject/timeout/disconnect 注入）。参考模型见
  `runtime/transport/tests/ws_generation_ledger_contract.rs`、
  `ws_broker_contract.rs`。

### 5.8 real boundary probe（定义）

- 真实 client WS → fake Runtime：attach + acquire + outbound RPC
  roundtrip；并发 peer close / runtime disconnect / replacement /
  shutdown 各方向，断言单 terminal、late result 隔离、pin/pending/
  tombstone 归零；真实 Runtime 场景由 E-ws 的
  `router-rust-ws-live` 覆盖。
