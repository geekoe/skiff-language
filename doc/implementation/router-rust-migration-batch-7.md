# Router Rust Migration Batch 7（E-session gate + W-activation + W-WebSocket + W-actor）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-6.md`（已合入并 push 到
origin/main@7d8779c4）。本批次实现权威设计 §4、§5.4 W-activation/W-WebSocket/W-actor、
§7 E-session、§8 router-live:session，不修改设计语义。

## 批次目标

- E-session gate：`router-live:session` managed harness——真实 Rust Runtime 进程对真实
  Rust Router binary 完成 bootstrap/register/health/reconnect/shutdown roundtrip；
  session barrier、pre-auth limit/timeout、saturation、fail-stop 归零；不声称 unary；
  verify live registry 注册 + CI workflow（router-rust-integration.yml）增加 managed
  session job。
- W-activation：`ActivationCoordinator`——live transaction（§4.1 步骤 1-10：current
  revision/active epoch 读取、blocking loader、candidate query 冻结、durable prepare CAS
  前 revalidate、non-blocking enqueue、ACK 校验、durable commit CAS 后由 durable
  authoritative、atomic Arc swap、commit/abort enqueue 失败 abort exact session）+ cold
  recovery（§4.2：committed 先发布、pending recovery transaction、expected replica 注册时
  rebind）；消费 W-activation-state repository、W-routing-query、W-bootstrap publish port、
  session ports。
- W-WebSocket：`ClientConnectionIndex`、`ClientSocketGeneration`、`RuntimeGenerationPinLedger`、
  `WebSocketRequestBroker`（peer correlation/deadline/tombstone/captured writer fence）、
  JSON-RPC 2.0 text profile（id lexeme 校验 1e0→1、-0→0）、single writer、frame/byte 预算、
  slow-client saturation、四向竞态（replacement/peer close/runtime disconnect/shutdown）；
  首个真实边界为真实 client WS → fake dispatcher。
- W-actor：`ActorMethodCatalogView`（只读 A0/A3 projection，不读 File IR）、
  `ActorOwnershipRegistry`（ActorClaimToken reserve/commit/abort 唯一 claim truth）、
  `ActorActivationRequestBroker`、`ActorInvocationRelay`、`ActorOwnerControlBroker`、
  `ActorLeaseExpiryScheduler`、spawn consumer（SpawnSubmitRouter 按 callerKind 精确选择）；
  消费 C-actor/C-spawn corpus。

退出检查点：节点合入本地 main，探针通过，push origin/main，worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| E-session gate | §7 E-session、§8 router-live:session | main@7d8779c4 | `feat/router-rust-e-session-gate` / `wt-e-session-gate` |
| W-activation | §4、§5.4 W-activation | main@7d8779c4 | `feat/router-rust-w-activation` / `wt-w-activation` |
| W-WebSocket | §5.4 W-WebSocket、§7 E-ws 前置 | main@7d8779c4 | `feat/router-rust-w-websocket` / `wt-w-websocket` |
| W-actor | §5.4 W-actor、§7 E-actor-rust 前置 | main@7d8779c4 | `feat/router-rust-w-actor` / `wt-w-actor` |

## 并行 ownership 边界（写文件声明）

- `router/src/activation/`：W-activation 只加 coordinator/recovery 模块（repository 已有，
  不改）；W-activation 可消费但不能改 W-session/W-routing-query/W-bootstrap 交付。
- `router/src/ws/`（新）：仅 W-WebSocket；`router/src/lib.rs` additive。
- `router/src/actor/`（新）：仅 W-actor；`router/src/lib.rs` additive。
- `scripts/lib/verify-live-registry.mjs`、CI workflow、live harness 脚本：仅 E-session gate。
- run_router/main.rs/listener.rs：本批次禁止触碰（除非 E-session gate 发现必须的生产接线，
  此时停下上报 root，不自行修改）。
- runtime crate、runtime/transport/src、deployment、router TS、AGENTS.md、scripts README、
  verify selector graph、skiff-instance.mjs：本批次禁止触碰。
- 任何节点不得操作 stable instance/Mongo/PM2/4004-4007；live harness 用隔离 instance/
  临时 Mongo（45000-45999 租约），用后清理；不跑全量 `pnpm verify`。

## 验证 owner

- E-session gate：`router-live:session` 真实 roundtrip 通过（bootstrap/register/health/
  reconnect/shutdown、barrier、pre-auth、saturation 归零）、`verify --only router-rust,
  router-rust-process-smoke` + `--list` 含新 live 条目、CI workflow YAML 解析。
- W-activation：coordinator 单元/sequence 测试（live 16 + coldRecovery 6 corpus）+ 临时
  Mongo 全链探针（prepare→commit→swap→commit→re-register→new request 的准备路径，
  full-chain 归 E-activation）、`verify --only router-rust`。
- W-WebSocket：broker/ledger/index 序列测试 + 真实 client WS → fake dispatcher 探针 +
  JSON-RPC id 词法 corpus（22 case）、`verify --only router-rust`。
- W-actor：六 owner 各自 sequence 测试 + C-actor/C-spawn corpus（20 帧 + 10 parent 场景）、
  `verify --only router-rust`。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p runtime`、
  `check-local-instance.mjs`。

## 风险与停止条件

- E-session gate 若暴露生产接线缺口（真实 Runtime 无法完成 roundtrip），停下报告 root
  并附精确失败证据，不顺手改生产代码。
- W-activation 的 coordinator 与其他 owner 共享面：只通过契约端口消费，不跨模块改状态。
- W-actor 的 claim truth 唯一 owner 在 ActorOwnershipRegistry；broker 不得另存 claim truth。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@7d8779c4，确认可执行后创建
自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务
文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b7 交接并通知主 Agent。
集成 Agent 在全部合入、探针通过后把 main push 到 origin/main（已授权）。
