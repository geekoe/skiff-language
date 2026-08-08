# Router Rust Migration Batch 8 — W-composition Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话；主 Agent 裁决扩展后继续）
Agent：`/root/dev_w_composition`
集成目标：`/root/router_rust_integration_b8`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-8.md`
  （W-composition 节点；baseline `origin/main@d228b613`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（owner 表、`RouterSupervisor` 唯一 lifecycle owner）、§3.6
  （consumer manifest / 静态注册 / barrier）、§3.8（boundedness）、§5.5
  （demux 与 sink bundle、composition 不成为 merge hotspot）、§7
  （各 E-gate 前置）。
- 冻结契约：
  - `router-rust-migration-c-process-lifecycle-contract.md`
    （`RouterSupervisor` 唯一 lifecycle owner、composition 扩展规则、
    shutdown step / residue）。
  - `router-rust-migration-c-session-contract.md`（§5.1 manifest、
    §5.3 per-session outbound queue、§6 demux / sink）。
  - `router-rust-migration-c-dispatch-contract.md`（§7.2
    `DispatchRequest` → `DispatchSubmit`、pending/terminal）。
  - `router-rust-migration-c-routing-query-contract.md`（candidate
    projection、`CandidateDirectoryView`）。
  - `router-rust-migration-c-activation-coordinator.md`（§8
    `ActivationCoordinatorPorts`：repository / loader / candidates /
    sessions / publish / health）。
  - `router-rust-migration-c-ws-contract.md`（ledger / broker ports、
    fake seam；production peer/responder 接线）。
  - `router-rust-migration-c-actor-contract.md`（§10 W-actor 交付义务；
    `ActivationControlPort` / `IdleEvictControlPort` /
    `ActorMethodSpawnExecutionSink`）。
- 兄弟交付（已在 `origin/main@d228b613`）：W-http、W-dispatch、W-routing-query、
  W-bootstrap（含 `RouterBootstrapAssembly`）、W-session、W-WebSocket、
  W-activation（`ActivationCoordinator` + ports）、W-actor。

## 主 Agent 裁决（2026-08-03，最小扩展授权）

原任务上报的三处阻塞（per-session outbound writer 缺失、inbound sink
安装点缺失、HTTP surface 不在 RoutingEpoch）按最小扩展授权：

1. `router/src/session/layer.rs` + `task.rs`（必要时 `demux.rs`）：
   additive seam——
   (a) per-session outbound writer registry：SessionLayer 增加向 exact
   `RuntimeSessionEpoch` 写任意帧的公共注册/调用 API（`OutboundQueue` 仍归
   session task 所有，只暴露 bounded non-blocking enqueue）；
   (b) inbound sink 分发 hook：`RuntimeFrameDemux` 保持 closed family
   registry 与方向规则，把未实现家族改为可注入 sink bundle（§5.5
   `RuntimeFrameSinks` 形态），composition 负责装配各 lane sink。
   严禁改变既有 session 语义：既有 session 测试必须全绿；新增 API 全部
   additive；未注入 sink 时保留 `Unimplemented` 行为。
2. HTTP surface：supervisor/composition 可从 deployment records 读取
   gateway entries 并构造 `HttpGatewaySurfaceView`（deployment crate API
   只读可消费，deployment crate 文件禁止写）；若必须把 surface 挂进
   `RoutingEpoch`，做 additive 字段并同步 bootstrap assembly 构造。
3. 完成原任务全部 6 项接线 + 公共 composition test。

其余写入边界不变。禁止：改既有 session 状态机语义、改 demux 中央 match
语义（只允许注入点）、写 deployment/runtime/transport/TS、碰共享主
worktree 与本地 main。

## 零 worktree 只读预检结论（锚定 origin/main@d228b613）

1. 基线：`git rev-parse origin/main` =
   `d228b613eafeba5e2275bf830f5770f21b931e81`；`router/src/` 已含
   activation/actor/artifact/bootstrap/config/dispatch/http/listener/
   routing/session/ws 全部 W-* 交付；`router/src/supervisor/` 不存在，
   本节点按批次文档在 `router/src/supervisor/` 建立 composition 位置。
2. `lib.rs` 已 re-export 全部 seam 类型：`RouterBootstrapAssembly`、
   `RoutingEpoch`/`ActiveRoutingEpochStore`、`RequestDispatcher`/
   `RuntimeAdmissionPool`/`DispatchSubmit`、`RuntimeCandidateQuery`/
   `CandidateDirectoryView`、`HttpDispatchPort`/`DispatchRequest`/
   `HttpGatewaySurfaceView`、`WebSocketLane`/ledger/broker、
   `ActivationCoordinator`/ports、actor 六 owner + spawn consumer。
3. 缺口（主 Agent 已裁决扩展）：
   - `SessionLayer` 无向 exact session 写任意帧的公共 API；
     `OutboundQueue` 在 `run_session_task` 内私有创建
     （`session/task.rs`）。
   - `RuntimeFrameDemux` 对 Request/Connection/Actor/Spawn 家族与非
     Register 的 Activation 变体一律 `Unimplemented` → 终止 exact
     session；无 sink 安装点（`session/demux.rs`）。
   - `RoutingEpoch` 无 HTTP surface（mode/adapterKind）；surface 数据在
     deployment records（`DeploymentGatewayEntry`），deployment crate 只读。
   - listener 公共 HTTP 目前为空响应；client WS path 未接（E-ws 范围）。
4. 预检确认：`SessionLayer::with_options` 公共、manifest/consumers 可注入；
   `CandidateViewSource`/`RoutingEpochSource`/`LeaseRevalidate`/
   `SessionAbortControl` 可基于 session 公共 API 实现；
   `EpochStorePublishPort`/`BlockingLoaderCandidatePort`/
   `RoutingCandidateQueryPortAdapter` 已存在；A3 catalog 已在
   `RoutingEpoch::actor_catalog()`。

## 实现决策（在冻结契约语义内）

### Session additive seam

1. `OutboundFrameId` 增加 `Business` 变体（additive；writer error 按
   disconnect 处理，不改变 Bootstrap/RegisteredAck/Close 语义）。
2. `SessionLayer` 增加 `SessionFrameWriter` trait 与 per-session registry：
   - `register_frame_writer(session, writer)`：session task 在
     capabilities bind 后注册；`unregister_frame_writer(session)` 在
     close 前注销。
   - `write_session_frame(session, bytes) -> Result<(), String>`：
     bounded non-blocking enqueue；队列满 / 未注册 → Err（lane 按各自
     契约 fail closed：dispatch callback_error、activation queue full、
     ws release failure、actor control failure）。
3. `RuntimeFrameDemux` 增加可注入 `InboundSinkSet`（§5.5 sink bundle
   形态）：request / connection / activation-transaction / actor /
   spawn 五个安装槽；`classify` 保持 framing/direction/payload 校验，
   未安装 sink 的家族维持原 `Unimplemented` → 终止 exact session。
   `SessionLayer::install_inbound_sinks` 为 additive setter（在 listener
   启动前调用；不改变 `SessionLayerOptions` 结构，避免既有 struct
   literal 回归）。
4. `session/task.rs`：bind 后注册 writer、close 前注销；`DemuxEvent::Sink`
   分发到已装 sink；sink 返回 `Terminal` 时 exact session 终止。

### Supervisor composition

5. `router/src/supervisor/`：
   - `RouterComponents`：静态组件 manifest + 全部生产 adapter
     （consumer wrapper、peer/responder/session port、coordinator ports、
     actor ports、http adapter、sink bundle）。
   - `RouterSupervisor`：唯一 lifecycle owner（config → bootstrap
     assembly → components → listeners → shutdown）。
   - HTTP surface：从 deployment records（`CanonicalArtifactStore`
     只读 API）构造 `HttpGatewaySurfaceView`。
   - 公共 HTTP：`start_http_gateway` 经 `HttpDispatchPort` adapter 接
     `RequestDispatcher`；listener.rs 增加 additive 装配函数，run_router
     使用 supervisor 装配。
6. Consumer manifest：HealthLedger + RequestDispatcher（supervisor
   wrapper）+ RuntimeGenerationPinLedger + WebSocketRequestBroker +
   ActivationCoordinator；`SessionLayer::with_options` 校验消费者集合与
   manifest 一致。
7. 未注入 / 延迟 seam（文档化，E-gate 补齐）：actor inbound sink
   （E-actor-rust）、WS client listener 与 `DispatchInbound`/`MethodCatalog`
   生产实现（E-ws）、actor spawn 执行 owner（E-actor-rust）、
   `PendingAdmissionSender` 真实 pending 池（E-ws）。未装 sink 家族维持
   `Unimplemented` fail closed，不伪造 capability。

## 写入边界（裁决后）

可写：

- `router/src/supervisor/`（新，composition 位置）；
- `router/src/session/layer.rs`、`task.rs`、必要时 `demux.rs`、
  `budget.rs`（仅上述 additive seam）；
- `router/src/bootstrap/assembly.rs`（仅 additive accessor /
  surface seam 需要时）；
- `router/src/listener.rs`、`router/src/main.rs`（run_router 装配）；
- `router/src/lib.rs`（仅 additive 模块声明与 re-export）；
- `router/tests/composition_*.rs`（公共 composition test + seam 单测）；
- `doc/implementation/router-rust-migration/execution/router-rust-w-composition-leaf.md`（本文件）。

禁止：

- `router/src/http`、`router/src/dispatch`、`router/src/ws`、
  `router/src/activation`、`router/src/actor` 既有模块内部；
- 改既有 session 状态机语义、demux 中央 match 语义；
- deployment、runtime crate、`runtime/transport/src`、router TS、
  AGENTS.md、scripts README、verify 文件、`skiff-instance.mjs`；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| seam 单测（outbound registry、sink 注入、Unimplemented 保留） | `cargo test -p skiff-router composition` |
| 公共 composition test | 同上：manifest/consumers 一致、adapter 契约转换、coordinator ports、actor ports、run_router 装配 |
| 既有 session 测试零回归 | `cargo test -p skiff-router session` 全绿 |
| 全 crate 回归 | `cargo test -p skiff-router`（`--no-fail-fast`） |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| 格式 / clippy | `cargo fmt -p skiff-router -- --check`；`cargo clippy -p skiff-router --all-targets` 无新增 error |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff origin/main...HEAD` 聚焦 |

## 交接

完成后提交到 `feat/router-rust-w-composition`（不 push），直接向
`/root/router_rust_integration_b8` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知延迟 seam（actor inbound sink、WS client
listener、spawn execution owner、`PendingAdmissionSender` 真实 pending
池），并通知 root。

## 执行结果（2026-08-03 提交前填写）

### 交付

- session additive seam：`OutboundFrameId::Business`；`SessionFrameWriter`
  registry（register/unregister/write_session_frame/has_frame_writer）；
  `InboundSinkSet` / `InboundFrameSink`（含 `accepts_frame_type` 注入点，
  覆盖 closed family registry 无法分类的 `response.*` 与
  `websocket.generation.lifecycle` 帧；无 sink 时保持原 MalformedFrame /
  Unimplemented）；`DemuxEvent::Sink`；session task 的 writer 注册/注销、
  capabilities 保留、sink 分发、Business writer error 按 disconnect。
- supervisor：`RouterComponents`（dispatcher/admission、WS lane、
  coordinator、actor lane、HTTP adapter、sink bundle、consumer manifest）、
  `RouterSupervisor`（唯一 lifecycle owner）、`SupervisorListeners`；
  `DispatcherHttpPort`（DispatchRequest → DispatchSubmit、timeout/cancel、
  reject/terminal 映射、unary/stream round-trip）、`PendingHttpRouter`、
  `RequestFrameSink`、`ActivationTransactionSink`、`ConnectionFrameSink`、
  `SessionCandidateViewSource`/`DirectoryLeaseRevalidate`/
  `SessionRuntimePeer`/`ActivationSessionEnqueuePort`/WS peer/responder/
  close/violation、actor ActivationControlPort/IdleEvictControlPort/
  spawn execution sink；`HttpGatewaySurfaceView` 从 deployment records
  只读构造；listener.rs `start_runtime_control_listener` /
  `resolve_listener_addr` / `ListenerHandle::{begin_shutdown,join_shutdown}`；
  run_router 经 supervisor 装配。
- 测试：`composition_session_seams.rs`（3）、`composition_components.rs`
  （10）、`composition_supervisor.rs`（2）。

### 自验收

| 项 | 结果 |
| --- | --- |
| seam 单测（outbound registry、sink 注入、Unimplemented/MalformedFrame 保留） | `cargo test -p skiff-router --test composition_session_seams` 3 passed |
| 公共 composition test | `composition_components` 10 passed + `composition_supervisor` 2 passed |
| 既有 session 测试零回归 | `cargo test -p skiff-router session` 全绿；全 crate 全绿 |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` 全绿（0 failed；live probe 为 ignored） |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 2/2 passed |
| 真实 binary session roundtrip | `node scripts/check-router-session-live.mjs` PASS（真实 Router + Runtime + 临时 Mongo，握手/reconnect/替换/shutdown 无回归） |
| 格式 | 触碰文件 `rustfmt --edition 2021` 通过 |
| clippy | `cargo clippy -p skiff-router --all-targets` 本节点文件零 warning/error（剩余均为 baseline 依赖 crate warning） |
| 写集 | 仅叶子声明文件 + `Cargo.lock`（base64 additive） |

### 已知延迟 seam（E-gate 补齐）

- actor inbound sink（`InboundSinkSet.actor = None`；E-actor-rust）；
- `SpawnSubmitRouter` 的 function-parent lookup 与 `RuntimePeer::send_spawn_submit`
  wire 映射（E-actor-rust）；actor spawn 执行 owner
  （`RecordingActorMethodSpawnExecutionSink` 为 composition 占位）；
- `ActivationControlPort` 的 ownerLeaseId mint 与 registry commit mint 的
  reconciliation（E-actor-parity）；
- WS client listener（`/ws`）与 `DispatchInbound`/`MethodCatalog`/
  `PendingAdmissionSender` 真实 pending 池（E-ws）；
- HTTP surface 仅消费 committed epoch 的 deployment records；A1 合流前
  deployment records 必须存在，否则 supervisor fail closed（显式 seam）。
