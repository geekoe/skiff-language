# Router Rust Migration Batch 4（实现波：bootstrap/session 基础 + 剩余 contract packs）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-3.md`（已合入本地 main@7683b7c8）。
本批次实现权威设计 §2.4 A1/A3、§5.3 W-model/M-pack、§5.4 W-session/W-activation-state
与剩余 C-model/C-* contract packs，不修改设计语义。

## 批次目标

- A1：compiler/deployment 侧 actor routing projection producer（消费 A0 冻结 schema，
  见 `router-rust-migration-a0-contract.md`）。
- A3：Rust strict reader/consumer + artifact loader 模块（skiff-router），不读
  PackageArtifact/File IR；交付 M-artifact consumer 证据。
- W-activation-state-repository：Router-owned Mongo adapter（read/CAS/audit/retry/index），
  durable DTO/reducer 若缺失则补 canonical 类型；交付 P-activation-state（reducer/CAS/retry/
  audit failure + 临时 Mongo replica set）。
- W-session：connection/session task、handshake、`RuntimeRegistrationDirectory`、health、
  cancellation/barrier、demux，消费 contracts-session corpus；真实 socket + fake Runtime peer。
- W-model：W-model-registration + W-model-bootstrap-wire 的 DTO/codec/corpus 与
  M-registration/M-bootstrap-wire Rust consumer gate（skiff-router + runtime 消费同一 corpus）。
- contracts-request：冻结 C-model-request + C-routing-query + C-dispatch。
- contracts-ws：冻结 C-model-connection + C-client-lifecycle + C-ws。
- contracts-actor：冻结 C-model-actor + C-model-spawn + C-actor + C-spawn
  （含 `callerKind = request | actorInvocation` 决策与 H-spawn-parent-cut 前置）。

退出检查点：节点合入本地 main（不 push），focused 证据通过，worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| A1 | §2.4 A1 | main@7683b7c8 | `feat/router-rust-a1` / `wt-a1` |
| A3 | §2.4 A3 | main@7683b7c8 | `feat/router-rust-a3` / `wt-a3` |
| W-activation-state | §5.3 C-router-activation-state、§8 P-activation-state | main@7683b7c8 | `feat/router-rust-w-activation-state` / `wt-w-activation-state` |
| W-session | §5.4 C-session、§8 E-session 前置 | main@7683b7c8 | `feat/router-rust-w-session` / `wt-w-session` |
| W-model | §5.3 W-model-registration/W-model-bootstrap-wire/M-pack | main@7683b7c8 | `feat/router-rust-w-model` / `wt-w-model` |
| contracts-request | §5.4 C-model-request/C-routing-query/C-dispatch | main@7683b7c8 | `feat/router-rust-contracts-request` / `wt-contracts-request` |
| contracts-ws | §5.4 C-model-connection/C-client-lifecycle/C-ws | main@7683b7c8 | `feat/router-rust-contracts-ws` / `wt-contracts-ws` |
| contracts-actor | §5.4 C-model-actor/C-model-spawn/C-actor/C-spawn | main@7683b7c8 | `feat/router-rust-contracts-actor` / `wt-contracts-actor` |

所有节点并行；集成 Agent 串行合入 `integration/router-rust-migration-batch-4`。

## 并行 ownership 边界（写文件声明）

- `skiff-router`（`router/`）模块划分：
  - `src/session/`、`src/listener.rs`/`src/main.rs` 的 session 装配：仅 W-session；
  - `src/artifact/`（strict reader + loader）：仅 A3；
  - `src/activation/`（repository adapter）：仅 W-activation-state；
  - `src/lib.rs`：各节点只加自己的 `mod` 声明（additive，机械合并）；
  - `tests/`：各节点用自己前缀（`session_*`、`artifact_*`、`activation_*`、`contract_*`、`w_model_*`）。
- `deployment` crate：`src/projection/` 仅 A1（A0 已建 actor_routing，A1 加 producer）；
  `src/activation-state/`（若需 DTO/reducer）仅 W-activation-state；`tests/` 按文件前缀划分。
- `runtime/transport/src`：仅 W-model（contracts-* 只写 tests/testdata，Batch 3 已合入）；
  `runtime` crate consumer tests：仅 W-model。
- `verify-rust-subjects.mjs`：若任何节点新增 workspace crate 必须先上报主 Agent，不得自行注册。
- AGENTS.md、scripts README、verify selector graph、verify.yml、`scripts/skiff-instance.mjs`：
  本批次禁止触碰。
- 任何节点不得操作 stable instance/Mongo/PM2/4004-4007；临时 Mongo 只允许 W-activation-state
  节点按仓库既有 harness 约定自建（独立 dbPath/port），用后清理。

## 验证 owner

- A1：deployment cargo test（producer + corpus）、rg 反向搜索（producer 输出严格按 A0 schema）。
- A3：skiff-router artifact tests + deployment 相关测试；rg 证明新 reader 不读 File IR/source/payload。
- W-activation-state：repository 单元/sequence 测试 + 临时 Mongo replica set 的
  CAS/retry/audit-failure 探针（P-activation-state 证据）。
- W-session：skiff-router session tests + 真实 socket handshake probe（fake Runtime peer）；
  `verify --only router-rust,router-rust-process-smoke`。
- W-model：transport/runtime/router consumer corpus tests；golden bytes 不变。
- contracts-*：corpus 测试 + 契约文档覆盖 §5.4 必填项 + rg 反向搜索。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、contract corpus 命令、
  `check-local-instance.mjs`。

## 风险与停止条件

- W-session 与 W-model 存在 codec 依赖：W-session 先消费 transport 现有 codec；若目标
  `assembly.activation:Register` 序列缺失，向集成 Agent/主 Agent 报告并等待 W-model 合入，
  不得在 skiff-router 私建 codec 副本。
- skiff-router 共享面（lib.rs/main.rs）只允许 additive/机械合并；语义冲突由集成 Agent 停下上报。
- 任何节点发现设计空洞/公共契约变化返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@7683b7c8，确认可执行后创建
自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务
文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b4 交接并通知主 Agent。
