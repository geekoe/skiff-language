# Router Rust Migration Batch 8（production composition + differential + A2）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-7.md`（已 push 到
origin/main@d228b613）。本批次实现权威设计 §5.5 composition、§9 differential、
§2.4 A2、§11.2 rollback 前置，不修改设计语义。

注意：本地 main 在用户并行 actor 工作线（40fac3b6，未 push），本批次及后续所有批次
一律以 origin/main 为基线，禁止参考/合并/回退本地 main。

## 批次目标

- W-composition：生产 composition 装配（RouterSupervisor/RouterComponents + run_router）——
  W-http `HttpDispatchPort` ↔ W-dispatch `RequestDispatcher` 适配（契约形状
  `DispatchRequest { header, payload_bytes, timeout, cancel_signal }` → `DispatchSubmit`）、
  routing query + admission 接入、W-WebSocket 两个 `SessionConsumer` 加入 SessionLayer
  manifest、W-activation coordinator（SessionEnqueuePort/ACK/PublishCommittedEpoch/
  candidate query/repository/blocking loader）、W-actor 端口（ActivationControlPort/
  IdleEvictControlPort/spawn execution sink）与 A3 catalog 装配；公共 composition test
  必须通过（每个 PR 构建并运行）。
- W-differential：implementation-neutral differential harness（TS/Rust 独立端口、artifact
  root、runtime home、Mongo namespace；对比 HTTP/WS/Runtime frames/health/Mongo state/
  terminal counters；normalization 仅 UUID/timestamp/ephemeral port/无语义 log order）；
  场景 inventory + verify 任务注册（fast/hermetic，不进 default）。
- A2：TS Router 硬切只读 canonical actor routing projection（A0 schema records），删除
  File IR 扫描路径（filesystemRuntimeAssemblySnapshotLoader.ts::loadActorMethods）；
  更新 TS 测试与 differential baseline（parity 前置）。

退出检查点：节点合入集成分支并 push origin/main（不碰本地 main/共享主 worktree），
探针通过，worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| W-composition | §5.5、§7 各 E-gate 前置 | origin/main@d228b613 | `feat/router-rust-w-composition` / `wt-w-composition` |
| W-differential | §9、§8 differential | origin/main@d228b613 | `feat/router-rust-w-differential` / `wt-w-differential` |
| A2 | §2.4 A2、§7 E-actor-parity 前置 | origin/main@d228b613 | `feat/router-rust-a2` / `wt-a2` |

## 并行 ownership 边界

- `router/src/supervisor/`（或预检确认的 composition 位置）、run_router/main.rs/listener.rs
  的生产装配：仅 W-composition（本批唯一 wiring owner）。
- `router/src/http`、`router/src/dispatch`、`router/src/ws`、`router/src/activation`、
  `router/src/actor` 既有模块：W-composition 只接线，不改其内部语义；发现需要改内部时
  停下上报。
- router TS（src/tests）：仅 A2。
- scripts/lib/verify-*、新 differential 脚本与 fixtures：仅 W-differential。
- runtime crate、runtime/transport/src、deployment：本批禁止触碰。
- AGENTS.md、scripts README、verify selector graph、skiff-instance.mjs：本批禁止触碰。
- 共享主 worktree 只读（multi-agent-development.md 铁律）；所有 git 操作在各自 worktree；
  基线一律 origin/main@d228b613（各节点在自己 worktree `git fetch origin` 后锚定）。

## 验证 owner

- W-composition：`cargo test -p skiff-router`（含公共 composition test）+ `verify --only
  router-rust,router-rust-process-smoke`；E-gate 前的便宜探针：真实 binary 启动 + session
  roundtrip 不回归（复用 check-router-session-live.mjs）。
- W-differential：differential harness 对既有 TS/Rust 实例跑通至少一个场景（无业务断言
  也可），场景 inventory 落盘、verify --list 含新条目、rg 证明不进 default。
- A2：router TS tests 全绿（catalog 从 canonical projection 读取，File IR 扫描负例 rg 零命中）。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p
  skiff-router`、`cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p
  runtime`、`check-local-instance.mjs`（临时目录）。

## 风险与停止条件

- W-composition 是本批共享检查点：任何 E-gate 需要它；接线发现跨模块语义矛盾时停下上报，
  不自行改模块内部。
- A2 若发现 canonical projection 记录路径与 A1 交付不一致，停止上报，不写兼容 reader。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 origin/main@d228b613（只读
git 对象即可；fetch 只允许在自己 worktree 内做），确认可执行后创建自己的 worktree（位于
/Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务文件（引用本批次文档
与权威设计），完成后直接向 router_rust_integration_b8 交接并通知主 Agent。集成 Agent 在
全部合入、探针通过后 push origin/main（已授权；本地 main/共享主 worktree 一律不碰）。
