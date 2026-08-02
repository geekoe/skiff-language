# Router Rust Migration Batch 7 — W-WebSocket Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_w_websocket`
集成目标：`/root/router_rust_integration_b7`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-7.md`
  （W-WebSocket 节点；baseline `main@7d8779c4`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（`ClientConnectionIndex` / `RuntimeGenerationPinLedger` /
  `WebSocketRequestBroker` owner 边界）、§3.4（`ClientSocketGeneration` 独立
  newtype）、§3.7（client socket 独立 finalization protocol，四向竞态）、
  §3.8（boundedness：single writer、frame/byte permit、deadline 重检）、
  §5.4（C-client-lifecycle + C-ws + M-connection → W-WebSocket）、§7
  （E-ws：peer correlation、deadline、tombstone、captured writer fence、
  late result isolation、JSON-RPC numeric id 词法 22 case、slow-client
  saturation）。冲突时以权威设计为准。
- 冻结契约（contracts-ws 链）：
  - `doc/implementation/router-rust-migration-c-model-connection-contract.md`
  - `doc/implementation/router-rust-migration-c-client-lifecycle-contract.md`
  - `doc/implementation/router-rust-migration-c-ws-contract.md`
  - `doc/implementation/router-rust-migration-c-net-contract.md`
    （listener/upgrade 机制，真实 socket probe 依据）
- 同链 corpus / 参考模型（test-only，W-WebSocket 消费同一 fixtures）：
  - `runtime/transport/testdata/client-ws/`（frames.json、jsonrpc-ids.json、
    scenarios/*.json 共 23 个场景）
  - `runtime/transport/tests/client_ws_corpus.rs`、
    `ws_generation_ledger_contract.rs`、`ws_broker_contract.rs`
- 已合入的 W-session / W-dispatch / W-routing-query 交付（只消费不改）：
  - `router/src/session/`：`RuntimeSessionEpoch`、
    `ConsumerKind::{RuntimeGenerationPinLedger, WebSocketRequestBroker}`、
    `SessionConsumer` / `RuntimeSessionClosed`
  - `router/src/routing/`：`RuntimeCandidateQuery`（本节点不消费 routing 投影，
    只保留 seam）
  - `router/src/dispatch/`：`RuntimePeer` / `SessionAbortControl` 等 ordinary
    pending 端口（本节点只定义自己的 WS 接缝，不共享 pending map）

## 零 worktree 只读预检结论（锚定 main@7d8779c4）

1. 基线：`git rev-parse main` = `7d8779c4b96c90c4d2d23748112ec1c0328091d7`；
   主 worktree 在 `integration/router-rust-migration-batch-7`（仅批次父文档
   commit `efde0dd9` 位于 main 之上，不进入本节点基线）。
2. C-net WS upgrade 机制：hyper 1 `http1::Builder + with_upgrades` →
   `hyper::upgrade::on` + `derive_accept_key` 写 101 →
   `tokio_tungstenite::WebSocketStream::from_raw_socket(Role::Server, None)`；
   `router/tests/net_probe.rs` 已有真实 socket probe 模式（client 侧
   `connect_async`）。本节点真实边界 probe 复用该机制。
3. contracts-ws 生产 codec 已冻结且可直接消费（不改 transport）：
   - `runtime/transport/src/connection_protocol.rs`：
     `ClientSocketGeneration`（typed newtype）、`OpaquePeerId` +
     `canonical_key()`、`ProfileAction`、
     `classify_jsonrpc_20_text_frame`（词法分类 + safe-integer lexeme
     canonicalize：`1e0→1`、`-0→0`）、`ConnectionRequestFrameHeader` /
     `ConnectionRequestCancelFrameHeader` / `ConnectionResponseFrameHeader` /
     `ConnectionResponseOutcome` / `ConnectionRemoteErrorFrameHeader` 与
     对应 encode/decode。
   - `runtime/transport/src/websocket_generation_lifecycle.rs`：
     `WebSocketGenerationLifecycleControl`（Acquire/Release/Ack/Reject）、
     `WebSocketGenerationLifecycleTuple`、encode/decode、
     `assert_websocket_generation_lifecycle_response_matches`。
   - `runtime/transport/testdata/client-ws/`：17 帧 byte-exact + 22 个
     JSON-RPC id 词法 case + 23 个参考状态机场景；`client_ws_corpus.rs` /
     `ws_generation_ledger_contract.rs` / `ws_broker_contract.rs` 为
     TEST-ONLY 参考模型，明确声明“W-WebSocket 必须实现冻结语义并消费同一
     fixtures”。
4. W-session 交付的 runtime generation 相关端口：`RuntimeSessionEpoch`（typed
   session identity）、`ConsumerKind` 已含 `RuntimeGenerationPinLedger` 与
   `WebSocketRequestBroker`、`SessionConsumer` / `RuntimeSessionClosed` /
   `ConsumerManifest`（默认仅 HealthLedger；本节点实现两个 consumer 的
   `SessionConsumer`，由集成 Agent 后续加入 manifest）。SessionLayer 的
   outbound writer 是 `pub(crate)`，本节点不触碰，只定义自己的 typed
   runtime-sender seam。
5. W-dispatch 交付的 pending 端口：`RequestDispatcher` ordinary pending 与
   WS peer pending 明确不共用 map；dispatch 侧不持有 WS 状态。本节点 broker
   的 inbound dispatch 通过 `DispatchInbound` seam 出界，terminal 由
   dispatcher 回调 `complete_inbound`（带 `InboundExecutionToken` fence +
   cancellation），与 C-ws §4.4 一致。
6. JSON-RPC response 的 opaque result/error.data 词法 slice：transport 的
   `classify_jsonrpc_20_text_frame` 只返回 `Response { id }`，不携带 payload
   slice；TS 语义（`jsonRpc20TextProfileImplementation.ts`）要求结果/错误
   data 保持 lexical opaque。因此本节点在 `router/src/ws/profile.rs` 实现
   response terminal 的**词法 slice 提取**（不解析业务 schema），分类仍消费
   transport 的冻结分类器。
7. 场景事件与生产映射：`attach.socketGeneration` 为 corpus 字符串令牌
   （`g1`），生产 `ClientSocketGeneration.generation` 为 u64；harness 把
   `g1` 映射为 generation=1，broker 的 `BrokerConnectionGeneration
   .socket_generation` 保留字符串令牌（peer id `<socketGeneration>:<seq>`
   与 corpus 一致）。runtime 字符串映射为 typed `RuntimeSessionEpoch`。

## 任务目标

在 `router/src/ws/`（新模块，仅本节点）实现 W-WebSocket：

- `ClientConnectionIndex`：logical client connection、business identity
  replacement（reject-new / close-oldest / ranked high-water）、
  `ClientSocketGeneration` 世代、single captured writer（fence + frame/byte
  budget）、slow-client saturation（1011）、shutdown（1001）；
- `ClientSocketGeneration` finalizer（§3.7 独立协议）：exact generation 标记
  closing → business/current index 撤销 → client cancellation（broker inbound
  abort）→ broker detach + bounded tombstones + ACK → ledger release →
  writer close/drain → barrier 删除 old record；old finalizer 不删
  replacement；release timeout/reject/send failure 完成 client terminal 且
  **不静默保留 pin**（1008 + runtime session 视为 protocol-unavailable）；
- `RuntimeGenerationPinLedger`：expect/acquire（cached requestId、
  session attachment、reject 码、duplicate expectation fail-stop）、release
  pending（dedupe）/Ack/Reject/timeout/send failure/runtime disconnect、
  flush 聚合失败；实现 `SessionConsumer`（kind=RuntimeGenerationPinLedger）；
- `WebSocketRequestBroker`：outbound/inbound peer correlation、deadline
  timer、tombstone FIFO/TTL/capacity、captured writer fence、late result
  isolation、generation close（1002/1003/1009）、runtime cancel/disconnect、
  inbound dispatch seam + `InboundExecutionToken` fence、capacity
  （resourceLimit / serverBusy）；实现 `SessionConsumer`
  （kind=WebSocketRequestBroker）；
- JSON-RPC 2.0 text profile：分类消费 transport 冻结分类器；numeric id
  lexeme 验证 + canonicalize（`1e0→1`、`-0→0`）；business params/result/error
  保持 lexical opaque slice；control members strict；
- 真实边界 probe：真实 client WS（tokio-tungstenite `connect_async`）→
  本节点 lane → fake dispatcher（不接 run_router）；
- 测试：`router/tests/ws_*` 前缀——23 场景 corpus 经生产 lane 驱动 +
  broker/ledger/index 序列测试（含四向竞态、writer fence、budget、
  tombstone、deadline、capacity）+ JSON-RPC id 词法 22 case 经
  production profile 断言 + 真实 client WS→fake dispatcher 探针。

## 实现决策（冻结契约语义内）

1. `WebSocketLane`（`router/src/ws/lane.rs`）是 WS 链的装配 owner：组合
   `ClientConnectionIndex` + `RuntimeGenerationPinLedger` +
   `WebSocketRequestBroker`，把 finalizer 的第 3/4 步（broker detach、ledger
   release、writer close/drain）接成一条 barrier；三个 owner 各自保持
   reducer 纯同步（`Mutex` 内不跨 `.await`），barrier 的 async 等待在 lane/
   finalizer 任务边界。
2. finalizer barrier：`index.finish(id, terminal, close)` 同步完成标记
   closing、deindex（id/business/runtime）、broker detach + tombstone，
   ledger release 返回 `PendingReleaseHandle`（`watch` 分辨率，dedupe 时新
   receiver 读当前已解析状态），随后 async barrier 等 release 解析 →
   writer close/drain → 删除 old record。四向竞态（replacement/peer
   close/runtime disconnect/shutdown）都进同一 finalizer；`finish` 幂等，
   terminal 恰好一次。
3. `RuntimeGenerationPinLedger` 状态：expected_by_connection_id、
   acquired_by_connection_id、pending release（by connection + by
   request_id）、cached acquire、routerSession↔runtime 双向绑定、
   release ack 计数、failures；`expect_connection` duplicate → fail-stop
   记录（进程退出由 supervisor 负责，本节点只暴露 `fail_stop_reason()`）；
   acquire 顺序：cached → session attachment（sender-mismatch）→
   expectation（not-acquired）→ tuple（tuple-mismatch）→ pending admission
   sender（seam `PendingAdmissionSender`，缺省 true）→ 已 acquired
   （tuple-mismatch）。release：dedupe 复用同一 promise；未 acquired 或
   socket 非 OPEN → 立即 resolve 不写帧；send 失败/timeout/reject →
   failures + `close_session(runtime, 1008)`（seam，health 记录
   runtime_closed），不静默保留 pin；runtime disconnect 清 ack 计数、
   acquired、pending、cached 与绑定并 resolve 全部 pending。
4. `WebSocketRequestBroker` 状态：generations（handle/uid/owner token/
   adapter/captured writer/open/active/sequence）、outbound_by_peer /
   outbound_by_runtime / inbound_by_peer / inbound_by_token、
   outbound/inbound tombstones（FIFO + TTL + capacity，sweep 由 clock
   驱动）。peer id = `<socketGeneration>:<seq>`；peer key = canonical
   `s:<string>` / `n:<int>`。deadline 注册在 entry 上，timer 由 lane 经
   `fire_deadline` 触发（测试显式注入），health 含 timerCount。
5. inbound dispatch：`DispatchInbound` seam 持 `InboundDispatchAction`
   （typed `InboundExecutionToken` + cancellation watch + opaque params）；
   dispatcher terminal 经 `complete_inbound(token, result)` 回调；映射：
   success→result、invalidParams→-32602、internalError/runtimeUnavailable→
   -32603、deadlineExceeded→-32001+abort；detach 先于写 terminal。
6. JSON-RPC profile（`ws/profile.rs`）：分类直接调
   `classify_jsonrpc_20_text_frame`；response terminal（success result /
   remoteError code+message+data）用 lexeme-preserving member-value slice
   提取（error 对象 strict：只允许 code/message/data、duplicate → close
   1002）；平台错误码与消息照 TS：parse -32700、invalidRequest -32600、
   methodNotFound -32601、invalidParams -32602、internal -32603、
   serverBusy -32000、timeout -32001。
7. 健康字段：index（connectionCount、finalizer pending/failures、per-
   connection observed writes、slowClient terminal 计数）、ledger（pins
   acquired/pending release/ack、failures、runtimeClosed、cached/pending
   计数）、broker（generationCount、outbound/inbound active global +
   per-generation、tombstones、timerCount、flush failures）；不含业务
   payload/query/Authorization。
8. 场景 harness（`router/tests/ws_corpus.rs`）：`include_str!` 23 个
   scenarios，按事件顺序驱动生产 lane + fake seams，断言 terminals、
   openConnections、connectionCount、generationCount、outbound/inbound
   pending、tombstones、pinsAcquired、pinsPendingRelease、releaseAcks、
   finalizerCount、runtimeClosed、failStop=false、归零不变量。
9. 真实 probe（`router/tests/ws_real_socket_probe.rs`）：hyper upgrade +
   tungstenite 服务端接入 lane 的 captured writer/reader；client
   `connect_async` 发送 `1e0`/`-0` id 请求断言 canonical peer key 与 terminal
   帧字节；发送非法/超预算帧断言 close 码；fake dispatcher 回 terminal。

## 写入边界

可写：

- `router/src/ws/`（仅本节点：`mod.rs`、`types.rs`、`profile.rs`、
  `ledger.rs`、`broker.rs`、`index.rs`、`lane.rs`、`health.rs`）；
- `router/src/lib.rs`（仅 additive `pub mod ws;` + re-export）；
- `router/tests/`（新增 `ws_*` 前缀测试文件：`ws_corpus.rs`、
  `ws_sequence_tests.rs`、`ws_jsonrpc_id_corpus.rs`、`ws_real_socket_probe.rs`、
  `ws_harness/` 共享 fake seam）；
- `doc/implementation/router-rust-migration-w-websocket-leaf.md`（本文件）。

禁止：

- `router/src/run_router`/`main.rs`/`listener.rs`、`router/src/activation/`、
  `actor/`、`routing/`、`dispatch/`、`session/`、`bootstrap/`、`http/`；
- runtime crate、`runtime/transport/src`、deployment、AGENTS.md、scripts
  README、verify 注册表/selector graph/verify.yml、`skiff-instance.mjs`；
- 修改 contracts-ws 契约/corpus fixtures（已冻结，由 contracts-ws 链拥有）；
- 在 skiff-router 私建 transport codec 副本（分类/帧 codec 只消费不复制；
  profile 的 opaque slice 提取是词法工具，不是 codec 副本）；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 23 场景 corpus 经生产 lane 全绿 | `cargo test -p skiff-router ws_` |
| broker/ledger/index 序列测试（四向竞态、writer fence、budget、tombstone、deadline、capacity、release timeout） | 同上 |
| JSON-RPC id 词法 22 case 经 production profile | `ws_jsonrpc_id_corpus` 全绿 |
| 真实 client WS → fake dispatcher 探针 | `ws_real_socket_probe` 全绿（canonical id 帧字节 + close 码） |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust` |
| 既有 router 测试不回归 | `cargo test --package skiff-router` |
| 格式/clippy | `cargo fmt -p skiff-router -- --check`、`cargo clippy --package skiff-router --all-targets`（exit 0） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b7` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵、seam 清单
（`RuntimeGenerationPeer` / `PendingAdmissionSender` / `RuntimeResponder` /
`DispatchInbound` / `PeerWriter` / `Clock` / `MethodCatalog` /
`NotificationObserver` / `RuntimeViolationSink`）与集成对齐点
（`ConsumerKind::{RuntimeGenerationPinLedger, WebSocketRequestBroker}`
 的 `SessionConsumer` 待集成 Agent 加入 manifest；真实 runtime sender/
dispatcher 接线待 E-ws），并通知 root（父 Agent）。

## 自验收结果（2026-08-02）

| 项 | 结果 |
| --- | --- |
| 23 场景 corpus 经生产 lane | `ws_corpus` 1 test 全绿（含四向竞态 12-18、release timeout 19、writer fence 09、tombstone 11、deadline 10/20、capacity 21、duplicate 22、runtime cancel 23） |
| broker/ledger/index 序列测试 | `ws_sequence_tests` 24 tests 全绿（含 out-of-order、late isolation、FIFO eviction、writer failure、peer disconnect abort、reject 码、dedupe、timeout、send failure、flush 聚合、captured fence、slow-client、1001/1002/1003/1008/1011 close 码） |
| JSON-RPC id 词法 22 case | `ws_jsonrpc_id_corpus` 2 tests 全绿（classifier + lane canonical frame 字节） |
| 真实 client WS → fake dispatcher 探针 | `ws_real_socket_probe` 全绿（`1e0`→`id:1`、`-0`→`id:0`、非法 id error 帧、outbound RPC roundtrip、replacement 1008、depth budget 1009、shutdown 1001、终态归零） |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust` passed |
| 既有 router 测试不回归 | `cargo test -p skiff-router` 46 个 target 全部 ok |
| 格式/clippy | `cargo fmt -p skiff-router -- --check` 通过；`cargo clippy -p skiff-router --all-targets` ws 文件 0 告警（其余 crate 告警为既有基线） |
| 写集 | 仅 `router/src/ws/`、`router/src/lib.rs`（additive 15 行）、`router/tests/ws_*` + `ws_harness/`、本叶子文件；`git status` 无越界文件 |
