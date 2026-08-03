# Router Rust Migration Batch 9（E-gates wiring + 五个 live gate）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-8.md`（已 push 到
origin/main@acd47cfc）。本批次实现权威设计 §7 E-dispatch/E-http/E-ws/E-activation/
E-actor-rust、§8 named live tasks、§11.2 rollback 扩展，不修改设计语义。

本地 main 仍在用户并行线，本批次一律以 origin/main 为基线；共享主 worktree 只读。

## 批次目标

### 波 1：E-gates wiring（唯一生产装配节点）

按 W-composition/W-session/W-WebSocket/W-actor leaf 记录的剩余 seam 完成生产接线：
- WS client listener accept 路径（listener.rs 的 websocket_path）+ WS inbound dispatch
  （PendingAdmissionSender/MethodCatalog/InboundExecutionToken 生产装配）；
- actor inbound sink（actor 帧分发）与 spawn execution sink（真实执行 owner）；
- 其余 E-gate 依赖的装配缺口，保持各 lane 模块内部语义不变。

### 波 2：五个 live gate（并行）

| Gate | harness / 任务 ID | 证据 |
| --- | --- | --- |
| E-dispatch | `router-rust-dispatch-live` / `router-live:dispatch` | fake ingress → admission/pending → real Runtime；selector/deployment/entry 负例、timeout、disconnect、竞态不双计容量、pending 归零 |
| E-http | `router-rust-http-live` / `router-live:http` | real HTTP → Router → Runtime unary/stream；CORS/ceiling/backpressure/error；首次 unary rollback roundtrip（TS→Rust→TS，§11.2） |
| E-ws | `router-rust-ws-live` / `router-live:ws` | real client WS → real Runtime；generation/replacement/slow-client/JSON-RPC id 词法 |
| E-activation | `router-activation-full-chain-live` / `router-live:activation-full-chain` | real Router+Mongo+compiler artifact+Runtime：prepare→commit→swap→commit→re-register→new-generation request；cold recovery |
| E-actor-rust | `router-rust-actor-live` / `router-live:actor` | two real Runtime replicas：get-or-create/spawn/invocation/control/lease full-chain；disconnect/replacement/竞态 fail closed |

每个 gate 交付：harness 脚本（仿 check-router-session-live.mjs 模式）、verify live
registry 条目（各自 key 块）、CI workflow 增加各自 managed job（append 模式）、叶子文档、
真实运行证据。gate 若发现生产缺口：小修仅限本 lane 模块内，需动 supervisor/main 时停下上报。

## 并行 ownership 边界

- 波 1 是 run_router/main.rs/listener.rs/supervisor 的唯一写入者；波 2 各 gate 禁止写这些
  共享装配面（除非 root 明确授权小修）。
- `router/src/ws`：E-ws gate 可在 wiring 后做本 lane 小修；`router/src/actor`：E-actor-rust
  gate 同；其余 gate 禁止。
- `scripts/lib/verify-live-registry.mjs`、`.github/workflows/router-rust-integration.yml`：
  五个 gate 各自只 append 自己的条目/job（不同行段）；registry 测试同步 +1 断言由各自节点
  维护自己的行。集成 Agent 做机械合并，冲突停下上报。
- runtime crate、runtime/transport/src、deployment、router TS、AGENTS.md、scripts README、
  verify selector graph、skiff-instance.mjs：本批禁止触碰。
- 共享主 worktree 只读；基线 origin/main@acd47cfc。

## 验证 owner

- 波 1：`cargo test -p skiff-router` 全绿 + `verify --only router-rust,router-rust-process-smoke`
  + check-router-session-live.mjs 不回归。
- 波 2：各 gate harness 真实运行通过（含负例与归零断言）、`verify --only router-live:<lane>
  --list` 含新条目、workflow YAML 解析；E-http 的 rollback roundtrip 记录在案。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p runtime`、
  `check-local-instance.mjs`。

## 风险与停止条件

- E-http 的 rollback roundtrip 需要 TS Router 仍可启动：TS 侧 A2 已合入，若 rollback 演练
  暴露 TS 启动问题，停下上报（rollback unit 完整化归 Batch 10）。
- E-activation full-chain 需要真实 Mongo replica set + compiler artifact：按既有 harness
  约定，45000-45999 租约，用后清理。
- E-actor-rust 的 two-replica 需要两个真实 Runtime 实例：独立 runtime home，不共享。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 origin/main@acd47cfc，确认可执行后
创建自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子
任务文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b9 交接并通知主
Agent。集成 Agent 在全部合入、探针通过后 push origin/main（已授权；本地 main/共享主 worktree
一律不碰）。
