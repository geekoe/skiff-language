# Router Rust Migration Batch 9 波 1 — E-gates wiring Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话；主 Agent 已裁决扩展后继续）
Agent：`/root/dev_e_gates_wiring`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-9.md`
  （E-gates wiring；baseline `origin/main@acd47cfc`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（owner 表）、§5.5（demux 与 sink bundle）、§7（E-ws /
  E-actor-rust）。
- 兄弟 leaf：`router-rust-w-composition-leaf.md`（supervisor/components 与
  延迟 seam 清单）、`router-rust-migration-w-websocket-leaf.md`、
  `router-rust-migration-w-actor-leaf.md`、`router-rust-migration-w-session-leaf.md`。
- 冻结契约：C-ws / C-client-lifecycle / C-model-connection / C-actor /
  C-model-actor / C-spawn / C-model-spawn / C-dispatch / C-routing-query /
  C-session / C-process-lifecycle。

## 主 Agent 裁决（2026-08-03）

1. **H1**：允许在 `router/src/http/server.rs` 增加 additive 公共 seam——
   `HttpGatewayServerOptions` 增加可选 upgrade 路径 + WS handler 回调；设置时
   accept loop 使用 `.with_upgrades()` 并把匹配 upgrade 交给回调，未设置时现有
   行为完全不变。禁止复制 HTTP 管线、禁止独立 WS 端口。
2. **H2**：在 supervisor/listener.rs 按 TS parity 与 contracts-ws 构建
   WS connect-admission 生产流（URL/header → service/entry/business identity
   → reserve/admit → runtime 选择 → websocketConnect 出站 + receipt 关联 →
   expect_connection → attach → peer task/finalizer → MethodCatalog /
   PendingAdmissionSender / DispatchInbound 装配）。沿用现有 config
   （`websocket.path`）与 deployment records，不新增顶层配置键，不改 transport
   wire；发现需要新契约或新配置时停下返回 TASK_SCOPE_EXPANDED 附证据。
3. **H3**：spawn 方向矛盾不属本节点（root 单独派发 M-spawn-repair 共享模型修复
   节点）。本节点只把 spawn seam 缺口如实记录，不实现 spawn 接线。
4. 其余任务照常：actor inbound sink（非 spawn 部分）、HTTP surface 复核。

## 零 worktree 只读预检结论（锚定 origin/main@acd47cfc）

1. `git rev-parse origin/main` =
   `acd47cfc8509d66c526ae105782546cc4f382c22`；worktree HEAD 即该 commit。
2. W-composition 交付：`RouterComponents` 静态 manifest、`RouterSupervisor`
   唯一 lifecycle owner、`SupervisorListeners`（public_http + runtime_control +
   session）、session inbound sink bundle（request/connection/activation 已装；
   actor/spawn 为 None）、HTTP surface 已从 deployment records 只读构造。
3. WS lane 现状：`WebSocketLane`（index/ledger/broker）完整；生产装配全部为
   占位——`AllowAnyPendingAdmission`、`EmptyMethodCatalog`、
   `NoopNotificationObserver`、`UnsupportedDispatchInbound`；
   `listener.rs::ListenerKind::Public` 仍是空响应，client WS accept 未接。
4. actor lane：六 owner + `SpawnSubmitRouter` 完整；`ActorLaneSpawnControl`
   已接 dispatcher；`RecordingActorMethodSpawnExecutionSink` /
   `UnavailableSpawnParentLookup` / `send_spawn_submit Err` 为延迟 seam。
5. transport 已具备全部所需 codec：websocketConnect / websocketJsonRpc
   request.start 与 response.end（专用 decode/encode）、actor.method /
   actor.owner / actor control frames、generation lifecycle。
6. 设计空洞（已裁决）：H1 同端口 HTTP+WS 装配；H2 WS connect-admission 生产流
   无 composition 层冻结文档；H3 spawn family 方向矛盾（transport registry
   `Spawn => RouterToRuntime`，而 Runtime driver 实际出站
   `spawn.submit.request` 到 Router；session demux 会终止 exact session；
   `SpawnSubmitAcceptance` 丢失原始 wire header/payload）。

## 实现决策

### H1 — http/server.rs additive upgrade seam

- `GatewayUpgradeHandler` trait（async，经 `async_trait`）：
  `handle(request: Request<Incoming>) -> Response<BoxBody<Bytes, hyper::Error>>`；
  回调自己完成 101 握手与升级后 WS 任务 spawn（超时/错误返回普通 HTTP 响应）。
- `GatewayUpgradeOptions { path: String, handler: Arc<dyn GatewayUpgradeHandler> }`；
  `HttpGatewayServerOptions` 增加 `websocket_upgrade: Option<...>`，`new()` 默认
  `None`。设置时 accept loop 对匹配 upgrade 的请求调用回调并使用
  `.with_upgrades()`；未设置时逐字节保持现有路径（真实 socket HTTP 测试零回归）。
- 升级后 WS 任务由回调实现者（supervisor/listener）独立跟踪，drain deadline 后
  abort（C-net §5 冻结语义）。

### H2 — WS connect-admission 生产装配（supervisor/ws.rs + listener.rs）

1. `WsGatewaySurfaceView`：从 `epoch.deployment_projection()` 经
   `CanonicalArtifactStore` 只读读取 deployment records（与
   `load_http_surface_view` 同形态）。按 `(service_id, path)` 建
   `WsBinding { service_id, deployment, gateway_entry_identity,
   websocket_entry_id（`skiff_artifact_identity::websocket_entry_id` 派生）,
   path, connect_handler: bool, methods: BTreeMap<method, WsMethodBinding> }`。
   ingress 条目 `protocol == WebSocket`：method None → connect；method Some →
   JSON-RPC method 表。重复 key / 非 websocketConnect 协议面 fail closed。
2. `WsDispatchStore`（composition 自有 pending/correlation 关联，仿
   `PendingHttpRouter` 形态；不触碰 ws/dispatch 模块内部）：
   - `connections: connection_id -> { runtime, binding, business_identity,
     path, assembly identity/generation }`；
   - `connect: connect_request_id -> { connection_id, response: watch }`；
   - `inbound: request_id -> { token: InboundExecutionToken, cancel }`；
   - per-runtime in-flight WS dispatch 计数（上限
     `runtime.maxConcurrency`；WS 与 ordinary pending 独立容量，记录为
     owner-split 的组合层决策）。
3. 生产 ports（trait 注入，测试用 fake）：
   - `WsSessionWriter`：向 exact `RuntimeSessionEpoch` 写帧（生产实现走
     `SessionHandle` + `SessionLayer::write_session_frame`）；
   - `WsConnectSelector`：候选选择（生产实现走 `RuntimeCandidateQuery`
     （unary）+ composition 自有 `RuntimeAdmissionPool`）；
   - `WsClock`：复用 `ws::Clock`。
4. connect-admission 序列（TS `webSocketGateway.prepareUpgrade` parity）：
   selector headers（`http::selector` 公共 API）+ Host + origin-form path →
   surface 解析 binding → `lane.reserve(connection_id)` →
   `lane.ledger.expect_connection(tuple)`（tuple.router_session_id 为
   composition 生成的 router session token）→ `WsConnectSelector` 选 runtime →
   编码 `RuntimeAssemblyWebSocketConnectRequestStartFrameHeader`（mode unary、
   caller gateway、routing runtimeAssembly + ingress webSocket method=null、
   websocket_connect 元数据：connection_id/url/query/headers/cookies/
   websocket_entry_id/gateway_entry_identity）→ 写 exact runtime → 等待
   response.end（`decode_runtime_assembly_websocket_connect_response_end_frame`）
   → Accept{ business_identity/admission_rank/connection_policy } 或 Reject。
   - Accept：`business_key = service\0entry\0business_identity`；policy 缺省
     `max_connections = u32::MAX, overflow = RejectNew`（TS 无 policy 时无
     per-key 上限的语义等价）；`lane.admit(...)` → store 注册 connection →
     返回 101。
   - Reject / 无 runtime / 超时：101 后 close frame（admit 拒绝）或普通 HTTP
     错误（pre-upgrade 失败）；reservation 经 `lane.finish(..., PolicyRejected/
     TransportError, close)` 归零。
   - binding 无 connect handler 且无 method（TS `requiresRuntimePin == false`）：
     Rust `WebSocketLane::attach` 必须绑定 runtime，本节点 fail closed 拒绝
     （记录为组合层决策，E-ws gate 使用带 handler 的 artifact）。
5. 升级后（101 后 spawned task）：`SocketPeerWriter`（bounded channel + 字节
   budget，仿 ws_real_socket_probe 生产化）+ `lane.attach(connection_id,
   generation=1, display=connection_id, runtime, writer, AttachMeta)` →
   reader loop（text/binary/disconnect）→ finalizer。
6. `MethodCatalog` 生产实现：epoch surface 全部 method 的 union（W-WebSocket
   seam 是全局 catalog，无 connection 上下文；per-connection 精确表在
   `WsDispatchStore::dispatch` 二次校验，不匹配 fail closed 为 InternalError，
   记录偏差）。
7. `PendingAdmissionSender` 生产实现：`WsDispatchStore` 的
   connection_id → runtime 映射，`is_pending_acquire_sender` 精确比对 tuple
   runtime。
8. `DispatchInbound` 生产实现：`action.method` 在 connection 自己的 method
   表解析 binding → 编码 `RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader`
   （unary、gateway、routing ingress webSocket method/path、deadline =
   inbound timeout、websocket_json_rpc {profile, connection_id,
   websocket_entry_id, gateway_entry_identity, business_identity}）→
   `WsSessionWriter` 写 exact runtime → 注册 inbound pending（request_id →
   token）→ Ok。响应帧经 Request-family sink 的 WS 分支映射 outcome →
   `WebSocketLane::complete_inbound(token, result)`（success/invalidParams/
   internalError/deadlineExceeded + abort）。
9. Request-family sink（supervisor/http.rs，additive）：decode 时先尝试
   websocketConnect/websocketJsonRpc response.end 专用解码器，命中
   `WsDispatchStore` 的 pending 则走 WS 分支；否则走既有 dispatcher →
   `PendingHttpRouter`。response.error 对 WS inbound 映射
   RuntimeUnavailable/InternalError。
10. listener.rs：`start_http_gateway` 的 upgrade 回调工厂
    （`handle_client_websocket_upgrade`：pre-upgrade 解析/admission + 101 +
    spawned peer task）；WS 任务 AbortHandle registry 归 supervisor，
    shutdown 时 abort。

### Actor inbound sink（supervisor/actor_sink.rs，非 spawn 部分）

- `ActorFrameSink`（`InboundFrameSink`, family=Actor）：按 C-model-actor §3
  方向表把 Runtime→Router / Either 帧分发给六 owner，并把 Router→Runtime
  响应/转发经 `WsSessionWriter` 写回 exact session：
  - `actor.getOrCreate.request` → `ActorActivationRequestBroker::get_or_create`
    → `actor.getOrCreate.response`（含 actorRef）/ `actor.getOrCreate.error`
    （`ActorSpawnRuntimeErrorFrameHeader`）；
  - `actor.replace.request` → registry `advance_incarnation` + activation
    broker 路径 → `actor.replace.response/error`；
  - `actor.find.request` → registry `current_owner` →
    `actor.find.response/error`；
  - `actor.remove.request` → registry `release`/`advance_incarnation` →
    `actor.remove.response/error`；
  - `actor.method.invoke`（caller）→ registry `current_owner` 取 fence →
    `ActorInvocationRelay::invoke` → `actor.owner.invoke` 转 owner runtime；
    no-owner/拒绝 → `actor.method.error`（TS parity 的
    ActorVersionRejected/IncarnationReplaced/Upgrading 映射，未覆盖边缘返回
    fail closed 并记录，E-actor-rust gate 小修）；
  - `actor.method.return/error`（owner）→ `relay.on_owner_settle` →
    `actor.method.return/error` 转 caller runtime；
  - `actor.method.cancel`（caller）→ `relay.on_caller_cancel` →
    `actor.method.cancel` 转 owner runtime；
  - `actor.owner.control.ack` → `ActorOwnerControlBroker::on_ack`；
  - `actor.owner.failure` → control broker / registry 对应收敛；
  - `actor.owner.invoke` / `actor.owner.control`（Router→Runtime）入站视为
    方向违规（MalformedFrame）。
- sink 安装：`InboundSinkSet { actor: Some(...), spawn: None }`。

### H3 — spawn seam 缺口记录（本节点不实现）

- transport family registry `RuntimeFrameFamily::Spawn.direction()` =
  `RouterToRuntime`，与 Runtime driver 出站 `spawn.submit.request`
  （`runtime/host/src/host/router_session/spawn_submit.rs`）矛盾；session demux
  按 direction 拒绝入站 spawn 帧并终止 exact session。修复归 M-spawn-repair
  节点（改 runtime/transport registry/corpus、session/demux、runtime-host 及
  消费者）。
- `SpawnSubmitAcceptance` 不携带原始 wire header/payload，真实执行 owner 无法
  仅凭公共 API 重建出站 `spawn.submit.request`；M-spawn-repair 需同时补
  acceptance 数据面或等价 seam。
- 本节点保留：`RecordingActorMethodSpawnExecutionSink`、
  `UnavailableSpawnParentLookup`、`send_spawn_submit Err`（均按原状）；
  `InboundSinkSet.spawn = None` 维持 fail closed。

## 写入边界

可写：

- `router/src/http/server.rs`（仅 H1 additive seam，已获授权）；
- `router/src/supervisor/`（`ws.rs` 新、`actor_sink.rs` 新、`mod.rs`、
  `http.rs` 的 RequestFrameSink additive WS 分支）；
- `router/src/listener.rs`（client WS accept 路径 + WS 任务 registry）；
- `router/src/main.rs`（如 run_router 装配需要）；
- `router/src/lib.rs`（仅 additive 模块声明/re-export 需要时）；
- `router/tests/gates_wiring_*`（新测试）；
- `doc/implementation/router-rust-migration/execution/router-rust-e-gates-wiring-leaf.md`（本文件）。

禁止（M-spawn-repair 并行面 + 本批冻结面）：

- `runtime/transport/src`、`runtime` crate、`session/demux.rs`（以及
  session 其余内部）、`ws/`、`actor/` 模块内部语义（只调用公共 API）、
  `dispatch/`、`activation/` 内部、deployment、router TS、AGENTS.md、
  scripts README、verify 文件、`skiff-instance.mjs`、config schema。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| http seam 未设置旧行为零回归 | `cargo test -p skiff-router http` 全绿 + `gates_wiring_http_seam` 断言未设置时 upgrade 不接管 |
| WS connect-admission 单元/集成 | `cargo test -p skiff-router gates_wiring`（fake runtime 驱动 surface/connect/acquire/inbound 全链 + 终态归零） |
| actor sink | `cargo test -p skiff-router gates_wiring_actor`（帧→owner→出站帧，含 error 映射） |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` 全绿 |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| session live 不回归 | `node scripts/check-router-session-live.mjs` PASS |
| 格式 / clippy | `cargo fmt -p skiff-router -- --check`；`cargo clippy -p skiff-router --all-targets` 无新增 error |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff origin/main...HEAD` 聚焦 |

## 交接

完成后提交到 `feat/router-rust-e-gates-wiring`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit/tree、实际
写集、自验收矩阵与已知延迟 seam（spawn 接线归 M-spawn-repair/E-actor-rust；
actor 边缘错误路径；WS per-connection method 精确性；no-pin 连接 fail
closed），并通知 root。

## 执行结果（2026-08-03 提交前填写）

### 交付

- H1 http seam：`HttpGatewayServerOptions.websocket_upgrade`（
  `GatewayUpgradeOptions` + `GatewayUpgradeHandler`）；设置时 accept loop 用
  `.with_upgrades()` 并把匹配 upgrade 交给回调；未设置时逐字节保持旧路径。
  真实 socket 测试：未设置不 hijack（非 101）、设置后 101 + 回调、非匹配
  path 仍走 HTTP。
- H2 WS 生产装配：
  - `supervisor/ws.rs`：`WsGatewaySurfaceView`（deployment records 只读投影，
    `(service_id, path)` → connect binding + JSON-RPC method 表）、
    `WsDispatchStore`（connect/inbound correlation、pinned connection 表、
    per-runtime dispatch capacity）、`WsMethodCatalog`（epoch method union）、
    `WsPendingAdmissionSender`、`WsInboundDispatch`、`WsLaneSessionConsumer`
    （runtime disconnect → store fail + lane finalizer）、
    `ProductionWsConnectSelector`（`RuntimeCandidateQuery` unary +
    composition 自有 `RuntimeAdmissionPool`）、`WsSessionWriter`/
    `LayerWsSessionWriter`、`WsLaneHandle`。
  - `listener.rs`：`ClientWsContext`（`GatewayUpgradeHandler`：selector/
    Host/path → binding → reserve → expect → selector → websocketConnect
    出站 → 等待 response.end → admit → store register → 101 → peer task）、
    `SocketPeerWriter`/writer/reader loop 生产化、`WsTaskRegistry`。
  - `supervisor/http.rs`：`RequestFrameSink::new_with_ws` 增加
    websocketConnect/websocketJsonRpc response.end 专用解码分支 + WS
    inbound response.error/cancel 分支；`new` 保持 2 参不变。
  - `supervisor/mod.rs`：全部生产 ports 接线；`SupervisorListeners` 增加
    ws_tasks abort；`start_listeners` 通过 http seam 挂 client WS。
- Actor inbound sink：`supervisor/actor_sink.rs`（getOrCreate 全流含 waiter
  ACK 解析、find/remove/replace、method invoke/return/error/cancel 转发、
  owner.control.ack / owner.failure 映射、方向违规 fail closed）；安装进
  `InboundSinkSet.actor`。
- 测试：`gates_wiring_http_seam.rs`（3）、`gates_wiring_ws.rs`（6）、
  `gates_wiring_actor.rs`（4）；更新 `composition_supervisor.rs` 的 actor
  sink 断言（占位 → 已安装，spawn 仍 None）。

### 自验收

| 项 | 结果 |
| --- | --- |
| http seam 未设置旧行为零回归 | `cargo test -p skiff-router http` 全绿；`gates_wiring_http_seam` 3 passed |
| WS connect-admission 单元/集成 | `gates_wiring_ws` 6 passed（fake runtime：surface/method catalog、connect 帧 byte-decode、admit/attach、inbound 帧、complete_inbound 到 peer writer、runtime disconnect 归零） |
| actor sink | `gates_wiring_actor` 4 passed |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` 65 个 target 全绿（0 failed） |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 2/2 passed |
| session live 不回归 | `node scripts/check-router-session-live.mjs` PASS（真实 Router + Runtime + 临时 Mongo） |
| 格式 / clippy | `cargo fmt -p skiff-router -- --check` 通过；`cargo clippy -p skiff-router --all-targets` 本节点文件零 warning/error（剩余为 baseline crate warning） |

### 已知延迟 seam / 阻断项（交接给集成与对应 gate）

1. **ledger `router_session_id` 不兼容（ws/ledger.rs 内部，本节点禁写）**：
   `RuntimeGenerationPinLedger::acquire_response` 对 expectation 做全 tuple
   比较（`expected != tuple`，含 `routerSessionId`），而 Runtime 自铸
   `skiff-router-session-v1:opaque:<uuid>`（`runtime/host/.../router_session.rs`）
   且从不发给 Router。TS parity 的 `matchesExpectation` 只比
   service/assembly/entry/connectionId。E-ws live gate 的真实 Runtime acquire
   会 TupleMismatch。修复归 E-ws gate 的 ws lane 小修（或 root 授权本节点
   补丁）；本节点测试用 self-consistent tuple 覆盖接线。
2. spawn 链（H3）：方向矛盾 + `SpawnSubmitAcceptance` 丢 wire 数据 → 归
   M-spawn-repair / E-actor-rust；`InboundSinkSet.spawn = None` 维持 fail
   closed；`send_spawn_submit` 仍返回 Err；spawn 执行 owner 仍为占位。
3. WS no-pin（handler-less + method-less connect binding）fail closed；
   per-connection method 精确性由 `WsDispatchStore` 二次校验（catalog 是
   union，broker 前置 accepts 为全局 seam）。
4. actor replace 目前 fail closed（`ActorReplaceUnavailable`）；invoke
   duplicate/saturated 不写错误帧（TS parity：无 errorFrame）；owner.failure
   转发原帧。E-actor-rust gate 细化。
5. `client_ws` 的 close-after-upgrade（ranked high-water 拒绝）路径已接但无
   专门测试（E-ws gate harness 覆盖）。

### 写集

- `router/src/http/server.rs`、`router/src/http/mod.rs`（re-export）；
- `router/src/supervisor/ws.rs`（新）、`actor_sink.rs`（新）、`mod.rs`、
  `http.rs`；
- `router/src/listener.rs`、`router/src/lib.rs`（additive re-export）；
- `router/tests/gates_wiring_http_seam.rs`、`gates_wiring_ws.rs`、
  `gates_wiring_actor.rs`（新）、`composition_supervisor.rs`（断言更新）；
- `doc/implementation/router-rust-migration/execution/router-rust-e-gates-wiring-leaf.md`（本文件）。

未触碰：runtime/transport、session 内部、ws/actor 内部、dispatch/activation
内部、deployment、router TS、verify/scripts、AGENTS.md。
