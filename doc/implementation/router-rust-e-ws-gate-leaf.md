# Router Rust Migration Batch 9 波 2 — E-ws gate Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_e_ws_gate`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-9.md`
  （波 2 五个 live gate；`router-live:ws` / `router-rust-ws-live`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §7 E-ws（client/business replacement、socket generation、Runtime
  generation leases、broker correlation 各自 owner；single writer、
  frame/byte budget、captured writer fence、late result isolation；
  business params/result/error lexical opaque；numeric id 词法
  `1e0→1`、`-0→0`；parser corpus/fuzz、slow-client saturation、
  disconnect races）、§8 named live tasks。
- 兄弟 leaf：
  - `router-rust-migration-w-websocket-leaf.md`（W-WebSocket lane 交付与
    四向 finalizer / writer fence / tombstone / deadline / 归零语义）；
  - `router-rust-w-composition-leaf.md`（supervisor/components 与延迟
    seam 清单）；
  - `router-rust-e-gates-wiring-leaf.md`（波 1 生产装配；已知
    `ws/ledger.rs router_session_id` 不兼容项，及 WS 生产装配 seam）。
- 冻结契约：C-ws / C-client-lifecycle / C-model-connection / C-net /
  C-session / C-dispatch / C-routing-query / C-process-lifecycle。
- corpus / 参考模型（test-only，生产 lane 消费同一 fixtures）：
  `runtime/transport/testdata/client-ws/`（frames.json、jsonrpc-ids.json、
  scenarios/*.json）、`runtime/transport/tests/client_ws_corpus.rs`、
  `ws_generation_ledger_contract.rs`、`ws_broker_contract.rs`。

## 基线

- 集成 head：`a9c8715bb6829c31c9fa75a88e38dde8ccaee7f3`（含波 1 E-gates
  production wiring）。
- 共享主 worktree 只读；本节点 worktree：
  `/Users/geek/workspace/wt-router-rust-e-ws-gate`，分支
  `feat/router-rust-e-ws-gate`。

## 主 Agent 授权（2026-08-03，两次小修 + 原 gate 交付）

1. **Gap 1（ws lane 小修）**：`router/src/ws/ledger.rs` 的 expectation
   比较改为 TS parity 的 5 字段 helper（serviceId / assemblyIdentity /
   assemblyGeneration / websocketEntryId / connectionId；cached/acquired
   全 tuple 校验保持不动），加测试覆盖。
2. **Gap 2（本授权新增，仅 supervisor WS 面）**：
   `router/src/supervisor/ws.rs` 只改 `WsDispatchStore`
   （`connect_begin` 登记 pending admission：connection_id → runtime +
   匹配字段；settle/unavailable 移除）与 `WsPendingAdmissionSender`
   查询（TS parity：acquire 在 connect pending 期间必须命中），加时序
   测试（acquire 先于 connect 注册到达的真实顺序）。不动 ws lane 内部
   语义、不改其他 supervisor 面。
3. 随后完成 gate 全部交付：`scripts/check-router-ws-live.mjs` harness
   （真实 client WS → real Runtime：generation/replacement/slow-client/
   id 词法/parser corpus/disconnect 竞态/四向 finalizer）+
   `router/tests/ws_live_probe.rs` + registry 条目（自己的 key 块）+
   CI job append + 叶子文档 + 真实运行证据。

## 零 worktree 只读预检结论（锚定 a9c8715b）

1. 生产装配完整：`ClientWsContext`（listener.rs）→ surface/binding →
   `lane.reserve` → `ledger.expect_connection`（Router 自铸
   `router_session_id`）→ `WsConnectSelector` → `store.connect_begin`
   （websocketConnect request.start）→ response.end → `lane.admit` →
   `store.register_connection` → 101 → peer task attach/reader loop。
2. 已知 Gap 1（波 1 leaf 记录）：`ledger.rs acquire_response` 全 tuple
   比较；TS `matchesExpectation` 只比 5 字段；真实 Runtime acquire tuple
   使用 Runtime 自铸 `skiff-router-session-v1:opaque:<uuid>`。
3. 预检发现 Gap 2：`WsPendingAdmissionSender` 只查
   `WsDispatchStore::connection_runtime`（pinned connections 表，仅在
   connect response 到达后注册）；真实 Runtime 在 connect 执行期间先发
   acquire 并阻塞 `receipt.wait()` 后才发 response.end。TS parity 的
   `isPendingWebSocketAcquireSender` 扫描 in-flight pending unary
   websocketConnect dispatch，因此真实链路会在 acquire 处
   SenderMismatch → 503。已由主 Agent 授权 supervisor WS 面小修。
4. Runtime 侧完整：`runtime/host` 已实现 websocketConnect 执行 +
   generation acquire/release lifecycle + websocketJsonRpc 执行与
   response.end 映射（只读确认，不写 runtime）。

## 执行中发现的生产缺口（已停止上报）

### Gap 3（阻塞真实链路，需动 supervisor/http 面，未授权）

- 现象：`router-live:ws` harness 已跑通 std 发布、package/assembly/config
  snapshot authoring、真实 Router+Runtime 构建与 Mongo 启动；但真实 Router
  composition 启动失败：
  `HTTP surface load failed: gateway entry big has no HTTP protocol surface
  in the HTTP surface view`。
- 位置：`router/src/supervisor/http.rs::load_http_surface_view` 把 deployment
  record 的**全部** gateway entries 交给
  `router/src/http/ingress.rs::HttpGatewaySurfaceView::from_deployment_gateway_entries`；
  `http_surface` 对非 HTTP protocol surface（WebSocketConnect /
  WebSocketJsonRpc）直接报错。因此任何含 websocket 条目的真实 deployment
  （E-ws 的 WS-only service 必然如此）都会让 Router fail closed 启动失败。
- TS parity：TS Router 的 HTTP 面只消费 HTTP ingress；WS 条目由
  `runtimeAssemblyWebSocketSnapshot` 独立投影，互不阻塞。
- 最小修复建议（需 root 授权）：`load_http_surface_view` 只投影
  `GatewayProtocolSurface::Http` 的条目（跳过 WebSocketConnect/
  WebSocketJsonRpc），与 `load_ws_surface_view` 已按
  `IngressProtocol::WebSocket` 过滤的形态对称；或等价地在
  `http/ingress.rs::from_deployment_gateway_entries` 跳过非 HTTP 条目。
  仅需 supervisor/http.rs（和/或 http/ingress.rs）的小改，不影响 HTTP lane
  语义；可加一个 deployment-record 级测试（HTTP+WS 混合条目 HTTP 面只含
  HTTP 条目）。

### 记录：slow-client budget 的 observed-write 结算语义（非阻塞，仅记录）

- `router/src/ws/index.rs` 的 `observed_write_bytes` 只在
  `transport.write_text` 报错时 `complete_write` 结算；成功路径不递减，因此
  预算实际按累计写入量（而非未 flush 量）计算，与 TS
  `webSocketConnectionLifecycle.ts` 的 `settleObservedWrite`（发送回调后
  递减）语义有偏差。属 ws lane 内部，超出本节点授权；live gate 的
  slow-client 场景在两种语义下都能触发 1011，不阻塞 gate 断言，仅记录。

### Gap 4（阻塞真实链路，需动 runtime，未授权）

- 现象：Gap 3 修复后 Router 正常启动、真实 Runtime 完成连接与注册
  （`runtime.registered` ACK），但 client WS connect 持续 503。Router 侧无
  日志；`ProductionWsConnectSelector` 的候选查询返回空。
- 根因：真实 Runtime 的 `runtime.capabilities` 帧
  （`runtime/host/src/host/control_plane.rs::queue_runtime_capabilities`）
  只设 `package_test_dispatch: false, request_cancel: true`，
  `dispatch_modes` 为空；Router 的 `DispatchCapabilities` 按
  `router/src/session/task.rs` 原样投影（empty → unary=false），
  `RuntimeCandidateQuery::project_session` 按冻结的 C-routing-query §3 rule 5
  （unary 要求 capabilities.unary）把该 session 排除 → 无候选 → 503。
  全 runtime crate 无任何 `dispatch_modes` 发射点（已 grep 确认）。
- TS parity：TS Router 从不消费 `dispatchModes` 做候选筛选（仅 protocol
  schema），因此同一真实 Runtime 在 TS 下可路由。
- 影响面：E-ws 之外同样阻塞 E-dispatch / E-http 的真实 unary 路由。
- 最小修复建议（需 root 授权，属 runtime 侧）：在
  `runtime/host/src/host/control_plane.rs::queue_runtime_capabilities` 声明
  `dispatch_modes: vec![Unary, ServerStream]`（runtime 实际已实现 unary 与
  stream 执行）；或等价地放宽/对齐候选筛选（需改 C-routing-query 契约语义，
  改动更大）。

## 执行状态（提交前，截至 Gap 4）

- Gap 1/2/3 修复 + 测试全绿（`ws_live_ledger_admission` 8、
  `ws_live_surface` 2、`gates_wiring_ws` 6、全 crate 368）。
- registry +1、CI job append、workflow 解析通过。
- harness 真实链路推进到：std/package/assembly/config authoring → 临时
  Mongo → 真实 Router+Runtime → Runtime 注册 → client WS 连接；被 Gap 4
  阻塞（503）。
- probe 内留有临时诊断（deployment entry 枚举、connect 失败原因、
  process log dump），最终提交前精简。

## 执行状态（提交前）

- Gap 1 修复 + 8 个新测试全绿（`ws_live_ledger_admission`，含真实顺序
  acquire-before-settle 用例）；`cargo test -p skiff-router` 368 passed。
- Gap 2 修复 + 时序测试全绿；`gates_wiring_ws` 6 passed 无回归。
- registry 条目、`verify-live-registry.test.mjs` +1、CI job append 完成；
  `node --test scripts/tests/verify-live-registry.test.mjs` 20/20 passed；
  workflow YAML 解析通过。
- harness 已实现且真实链路推进到 Router 启动；被 Gap 3 阻塞，未完成最终
  真实运行 PASS。

## 任务目标

`router-live:ws` managed harness：real client WS → real Router → real
Runtime 全链，覆盖：

- client connection/business replacement、socket generation、Runtime
  generation leases、broker correlation 各自 owner；
- single writer、frame/byte budget、captured writer fence、late result
  isolation；
- business params/result/error lexical opaque；
- numeric id 词法（`1e0→1`、`-0→0`）；
- parser corpus/fuzz、slow-client saturation、disconnect races、四向
  finalizer；
- 归零不变量（连接/引脚/pending/tombstone/finalizer）。

## 实现决策

### Gap 1（ledger.rs，已授权）

新增 `matches_expectation(expected, tuple)` helper：只比较
service_id / assembly_identity / assembly_generation /
websocket_entry_id / connection_id，忽略 `router_session_id`。
`acquire_response` 的 expectation 检查改用 helper；cached acquire 与
acquired pin 的 `tuple ==` 全等校验保持不变（TS `tuplesEqual` parity）。
测试：ledger 序列测试增加“Runtime 自铸 routerSessionId 的 acquire 命中
expectation”、“其他字段不匹配仍 TupleMismatch”、“cached/acquired
全 tuple 校验不回归”。

### Gap 2（supervisor/ws.rs，仅授权面）

`WsDispatchStore` 增加 pending-admission 状态：

- `connect_begin` 登记 `pending_admission_by_connection_id`：
  connection_id → `{ runtime, service_id, assembly_identity,
  assembly_generation, websocket_entry_id, connection_id }`；
- `connect_response` / `connect_unavailable`（及 `on_session_closed`
  清理该 runtime 的 connect pending）移除对应登记；
- `pending_admission_runtime(connection_id)` 只读访问；
- `WsPendingAdmissionSender::is_pending_acquire_sender` 先查 pending
  admission（TS parity：acquire 在 connect pending 期间命中），
  pinned connections 表保持作为已 admit 后的后备查询。

不动 ws lane 内部、不改其他 supervisor 面。

### Harness

`scripts/check-router-ws-live.mjs` 仿
`scripts/check-router-session-live.mjs`：

- 编译真实 package（websocket.yml：connect handler + JSON-RPC method）与
  assembly（rootDeployments 指向 serviceDeploymentReceipt.deployment）；
- 生成 runtime config snapshot；启动临时 Mongo replica set；租用
  45000-45999 端口；构建显式 Rust router + runtime binary；
- 运行 `router/tests/ws_live_probe.rs`（ignored test，env 驱动）：
  seed committed activation state → spawn real Router → spawn real
  Runtime → 等待注册 → real client WS（tokio-tungstenite connect_async）
  经 selector headers + upgrade → 101 → JSON-RPC 帧（含 `1e0`/`-0`）→
  断言 terminal 帧/结果 → replacement / slow-client / disconnect 竞态 /
  shutdown 归零。

### Registry / CI / tests

- `scripts/lib/verify-live-registry.mjs`：append 自己的
  `router-rust-ws-live` entry（selector `router-live:ws`，managed，
  fixed command，id `live:router-rust-ws`）；
- `scripts/tests/verify-live-registry.test.mjs`：对应行 +1 断言；
- `.github/workflows/router-rust-integration.yml`：append
  `router-rust-ws-managed` job（仿 session job，含 change-classifier
  自己的脚本路径段）；
- 叶子文档：本文件。

## 写入边界

可写：

- `router/src/ws/ledger.rs`（仅 Gap 1 helper + 检查点）；
- `router/src/supervisor/ws.rs`（仅 Gap 2 pending-admission 面）；
- `scripts/check-router-ws-live.mjs`（新）；
- `scripts/lib/`（仅 `ws_live_*` 前缀，如需要）；
- `scripts/lib/verify-live-registry.mjs`（仅自己的 entry append）；
- `scripts/tests/verify-live-registry.test.mjs`（仅对应行）；
- `.github/workflows/router-rust-integration.yml`（仅 append 自己的
  job + classifier 自己的脚本路径段）；
- `router/tests/`（仅 `ws_live_*` 前缀测试/共享 harness 文件）；
- `doc/implementation/router-rust-e-ws-gate-leaf.md`（本文件）。

禁止：

- 其他 `router/src`、`runtime/`、`deployment/`、router TS、AGENTS.md、
  scripts README、verify selector graph、`skiff-instance.mjs`；
- 修改 contracts-ws 契约/corpus fixtures；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 verify。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| ledger 5 字段 parity 测试 | `cargo test -p skiff-router ws_`（含新期望比较用例） |
| supervisor pending-admission 时序测试 | `cargo test -p skiff-router gates_wiring_ws` 新增用例 |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` |
| harness 真实运行 | `node scripts/check-router-ws-live.mjs` PASS |
| registry --list | `node scripts/verify.mjs --only router-live:ws --list` 含新条目 |
| workflow YAML 解析 | workflow 语法/结构校验通过 |
| 格式 / clippy | `cargo fmt -p skiff-router -- --check`；`cargo clippy -p skiff-router --all-targets` 无新增 error |
| 写集干净 | `git status` 仅本叶子声明文件 |

## 交接

完成后提交到 `feat/router-rust-e-ws-gate`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知残留（如有），并通知 root。
