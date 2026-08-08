# Router Rust Migration C-client-lifecycle：client connection 生命周期冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-WebSocket / E-ws 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md` §3.2
  （`ClientConnectionIndex`：logical client connection、business identity
  replacement、`ClientSocketGeneration`）、§3.7（client socket 独立
  finalization protocol，四向竞态：replacement/peer close/runtime
  disconnect/shutdown）、§3.8（boundedness：single writer、frame/byte
  permit、queue full terminal）、§5.4（必填项）、§7（E-ws：slow-client
  saturation、late result isolation）。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-ws-leaf.md`。
- 同链契约：`router-rust-migration-c-model-connection-contract.md`、
  `router-rust-migration-c-ws-contract.md`。

冲突时以权威设计为准；本文件只冻结契约，不写 production。

## 1. 范围

冻结 `ClientConnectionIndex`（logical connection 索引、business identity
replacement、admission policy）、`ClientSocketGeneration` finalization
protocol（§3.7）、single writer、frame/byte budget、slow-client
saturation、四向竞态终态与 release timeout（不静默保留 Runtime pin）。
不定义 broker correlation（C-ws）、wire 字节（C-model-connection）。

## 2. Identity 与 typed 类型（§3.4，冻结）

```text
ClientSocketGeneration { connection_id, generation: u64 }
ClientConnectionIndex
  connection_id -> ClientSocketGeneration + business_key? + rank? + state
  business_key  -> ordered set<connection_id>
BusinessKey  = service_id \0 websocket_entry_id \0 business_identity
              （business_identity 缺失则无 key，不参与 replacement）
WebSocketPolicyAdmission = accepted | rejected{ close }
WebSocketLifecycleClose  = { code: u16, reason: string(≤123 字节) }
ObservedWrite            = { bytes, promise }（captured writer 在途写）
```

- `ClientSocketGeneration` 与 `RuntimeSessionEpoch` 不互换；replacement
  只换 client generation，不换 Runtime session（pin 走 ledger）。
- business replacement 的 old generation finalizer 不得删除 new
  generation。

## 3. ClientConnectionIndex（§3.2/§3.7 冻结）

### 3.1 状态与索引

```text
state: reserved -> admitted -> attached -> closed（单一方向）
connections_by_id:        connection_id -> Connection
connections_by_business:  business_key -> ordered set（admission 序）
connections_by_runtime:   runtime -> set
```

- reserve：总量 `connectionLimit`（默认 5000）满 → 拒绝，不进入握手。
- admit：先按 business key 处理 replacement（3.2），成功后索引并进入
  admitted；pre-attach 关闭（abort-upgrade / close-after-upgrade）。
- attach：admitted 后绑定 socket；socket close/error → finish 恰好一次。
- finish：deindex（id/business/runtime）、settle 全部 observed write
  （reject）、调用 finalizer 恰好一次、关闭 transport（close 或
  terminate，CLOSED 跳过）。

### 3.2 business replacement（冻结）

同一 `business_key` 的并发连接按 admission 语义收敛：

1. `reject-new`：已有连接数 ≥ `maxConnections` → 新连接以 policy close
   （默认 1008）拒绝，旧连接保留。
2. `close-oldest`：超过 `maxConnections` 的最旧连接以 policy close
   关闭，新连接 admit；关闭在索引新连接前完成（旧 finalizer 不能删除
   新记录）。
3. ranked high-water：带 `admissionRank`（正 safe integer）的响应先到
   先占位；低 rank 或等 rank 后到 → 4009 superseded；高 rank 后到 →
   关闭低 rank/无 rank 旧连接并写入 high-water fence
   （retention = admissionHighWaterRetentionMs，默认 120s）；fence 保留
   期内同 key 不再接受 ≤ rank 的新连接；fence 只在无 active 连接时
   回收；未知新 key 在 high-water 容量满时进入 quarantine
   （1013 close）。
4. `admissionRank` 只允许配合 `maxConnections == 1` 的 `close-oldest`
   policy。

### 3.3 single writer（§3.8 冻结）

- 每个 `ClientSocketGeneration` 只有一个 captured writer；writer 的
  `writeText(frame)` 返回 promise，send callback 成功才 settle。
- writer 在途写计入 `observedWriteBytes`；预算检查
  `bufferedAmount + observedWriteBytes + messageBytes <= slowClientBudgetBytes`
  （默认 16 MiB）。
- close-vs-send 竞态：settle 只发生一次；late callback 忽略；
  close 期间未完成的写全部 reject。
- captured writer 带 generation fence：replacement 后旧 writer 的
  write/close 不得影响新 generation（测试：
  `keeps a captured writer fenced from a replacement connection id`）。

### 3.4 slow-client saturation（冻结）

- 任意 send/write 超预算：finish connection（1011
  `websocket client is too slow`），writer promise reject；不等待队列
  接受 close frame，transport 直接 close/terminate。
- 客户端不消费时 outbound queue 持续增长只能由 budget 终止；不得
  unbounded 排队。

## 4. ClientSocketGeneration finalization protocol（§3.7 冻结）

所有终态（peer close、business replacement、slow-client overflow、
Runtime pin loss、shutdown）进入同一 finalizer：

1. exact generation 标记 closing，先从 business/current index 撤销；
   new generation 可独立安装；
2. 原子触发 client cancellation（broker 的 inbound controller abort，
   即使 mailbox 饱和也可观察 terminal）；
3. `WebSocketRequestBroker` detach exact generation：terminal outbound、
   cancel inbound、安装 bounded tombstone、ACK；
4. `RuntimeGenerationPinLedger` release exact generation，writer
   close/drain 并 ACK；
5. finalization barrier 完成后删除 old generation record；old finalizer
   不得删除 replacement generation。

release timeout：release 请求默认 5s；超时/reject/send 失败 → 完成
client terminal 并把 exact Runtime session 视为 protocol-unavailable/
关闭（close 1008），**不得静默保留 pin**。flush 聚合全部 release
failure；任一失败 → gateway shutdown 失败（AggregateError）。

## 5. 四向竞态终态（§3.7 冻结）

replacement / peer close / runtime disconnect / shutdown 四向并发：

| 竞态 | 终态不变量 |
| --- | --- |
| replacement vs peer close | 只有一个 finalizer；old 终态为 Replacement 或 PeerClose 之一；new generation 不受影响 |
| replacement vs runtime disconnect | old pin 释放一次；new generation 若已 admit 由 runtime disconnect 独立终态 |
| replacement vs shutdown | shutdown 只 finish 当前 active generation；old finalizer 不触碰 new |
| peer close vs runtime disconnect | 任一先到即 finish；后到事件幂等 no-op；runtime disconnect 对已关闭连接不重复 close |
| peer close vs shutdown | 同上；shutdown 等待 finalizer 全 ACK |
| runtime disconnect vs shutdown | shutdown 时已断开连接不再入 shutdown 集合；pending release 全清 |

所有竞态要求：external terminal 恰好一次、cancel 至多一次、permit/
lease/timer 最终归零、health 归零、无 fail-stop（除 barrier ACK 超时
走 C-process-lifecycle fail-stop）。

corpus 场景：`runtime/transport/testdata/client-ws/scenarios/*.json`
（四向两两竞态各方向有序事件 + 单终态场景）。

## 6. §5.4 contract pack 必填项

### 6.1 owner / invariant

- Owner：`ClientConnectionIndex`（logical connection 与 business
  replacement 唯一 owner）、per-generation finalizer barrier。
- Invariant：一个 connection_id 任一时刻至多一个 active generation；
  business key 的连接序收敛到 policy；finalizer 恰好一次；old
  finalizer 不删除 replacement；slow-client 只以 budget 终止；
  release 超时后 pin 不保留。

### 6.2 typed inputs / outputs

- Inputs：`reserve(id, value, runtime?)`、`admit(id, {business_key,
  policy?, admission_rank?})`、`attach(id, socket)`、`close(id, close?)`、
  `runtimeDisconnected(runtime)`、`sendToConnection(id, message)`、
  `capturePeerWriter(id)`、`shutdown(close?)`。
- Outputs：`WebSocketPolicyAdmission`、`WebSocketLifecyclePeerWriter`、
  `WebSocketLifecycleClose`、finalizer promise、health snapshot。

### 6.3 capacity

- `connectionLimit` 默认 5000；admission high-water 默认同 limit；
  `slowClientBudgetBytes` 默认 16 MiB；shutdown 等待超时默认 1s；
  release timeout 默认 5s；business policy `maxConnections` 1..=2^32-1。

### 6.4 queue full

- 单 connection outbound 无独立帧数上限（以字节 budget 为界）；预算
  满 → 1011 slow-client 终态；shutdown 时 sockets 超时未关 → terminate。

### 6.5 timeout / disconnect / replacement / shutdown terminal

- peer close：finish（无 close 写）；transport error：1011。
- runtime disconnect：该 runtime 全部 connection finish 1011
  `websocket runtime disconnected`；ledger 侧 pending release 清空。
- replacement：policy close（1008/自定义）或 4009 superseded /
  1013 high-water capacity。
- shutdown：1001 `websocket gateway shutting down`；等待 finalizer 全
  ACK，超时 terminate，失败聚合报错。
- release timeout/reject：1008，pin 不保留。

### 6.6 health fields

- connectionCount、admissionHighWaterSize、per-connection observed
  writes、observedWriteBytes、slow-client 终态计数、finalizer
  pending/failures、release ack 计数、shutdown residue。

### 6.7 fake seam

- `FakeSocket`（readyState/bufferedAmount/send callback/close 记录）、
  `FakeClock`、`FakePolicyAdmission`、`FakeFinalizerAck`（注入缺失/超时
  ACK）；参考模型见 `runtime/transport/tests/client_ws_corpus.rs`。

### 6.8 real boundary probe（定义）

- 真实 client WS 连接：admit → attach → 注入 slow-client 流量 →
  断言 1011 与 writer reject；同 business key 二次 admit（close-oldest）
  断言旧 socket close 且新 socket 可 RPC；真实 Runtime pin 释放超时
  断言 runtime session 关闭且 pin 计数归零。该 probe 由 W-WebSocket/
  E-ws 执行，成为 `router-rust-ws-live` 的一部分。
