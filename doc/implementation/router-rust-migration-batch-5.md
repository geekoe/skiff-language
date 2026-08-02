# Router Rust Migration Batch 5（W-bootstrap + H-registration-cut + 剩余 W-model 包）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-4.md`（已合入本地 main@85596193）。
本批次实现权威设计 §2.5 H-registration-cut、§4 activation wire、§5.3 W-model/M-pack、
§5.4 W-bootstrap、§7 E-bootstrap/E-session 前置，不修改设计语义。

## 批次目标

- W-bootstrap：`CommittedActivationBootstrapReader`（只读 repository port）、strict assembly/
  config reader 构造完整 `RoutingEpoch`（environment、assembly_generation、assembly_identity、
  config_snapshot_id、immutable ingress/deployment/actor routing projection）并原子发布到
  `ActiveRoutingEpochStore`；blocking loader 有界池（semaphore/timeout/shutdown/health）；
  把 epoch source 接入 W-session 的 `SessionLayer` seam；missing/malformed/identity mismatch/
  pending 全部 fail closed（完整 recovery 归 E-activation）。
- H-registration-cut：current TS Router 与 Rust Runtime 同时硬切到新 handshake
  （bootstrap → capabilities → bind → `assembly.activation:Register` → registered ACK → health），
  删除 inbound legacy `runtime.register`，`runtime.registered` 只作成功 ACK；wrong order/
  identity change/duplicate/stale/ACK loss 有严格 terminal；TS 与 Rust consumer 先过共享
  corpus（contracts-session + W-model 交付物）再切。
- W-model-activation + W-model-connection：activation transaction wire
  （prepared/reject/prepare/commit/abort）与 connection/generation lifecycle wire 的
  DTO/codec/corpus + M-activation/M-connection Rust consumer gate。
- W-model-actor + W-model-spawn：actor wire codec/corpus + spawn `callerKind` canonical codec
  （新 wire generation，无兼容 reader；consumer 硬切归 H-spawn-parent-cut，本节点不切生产
  TS/Runtime consumer）。

退出检查点：节点合入本地 main，focused 证据通过，main push 到 origin/main（本批次起启用
push，使 CI gate 可真实建立），worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| W-bootstrap | §5.4 W-bootstrap、§7 E-bootstrap | main@85596193 | `feat/router-rust-w-bootstrap` / `wt-w-bootstrap` |
| H-registration-cut | §2.5、§3.5、§5.3 M-registration 后 | main@85596193 | `feat/router-rust-h-registration-cut` / `wt-h-registration-cut` |
| W-model-activation-connection | §5.3 W-model-activation/W-model-connection | main@85596193 | `feat/router-rust-w-model-activation-connection` / `wt-w-model-activation-connection` |
| W-model-actor-spawn | §5.3 W-model-actor/W-model-spawn | main@85596193 | `feat/router-rust-w-model-actor-spawn` / `wt-w-model-actor-spawn` |

## 并行 ownership 边界（写文件声明）

- `router/src/bootstrap/`：仅 W-bootstrap；W-bootstrap 可改 `router/src/session/` 中
  `SessionLayer` 的 epoch source 装配（seam 已由 W-session 声明），不得改 session 内部逻辑。
- router TS（`router/src/`、`router/tests/` TS 文件）：仅 H-registration-cut。
- `runtime` crate（src/tests，consumer 与 driver）：H-registration-cut 与
  W-model-activation-connection / W-model-actor-spawn 都会写 consumer tests——
  按 `tests/` 文件名前缀划分（`h_registration_cut_*` / `w_model_activation_*` /
  `w_model_actor_*`）；runtime `src` 仅 H-registration-cut。
- `runtime/transport/src`：activation/connection 模块仅 W-model-activation-connection；
  actor/spawn 模块仅 W-model-actor-spawn；`lib.rs`/`protocol.rs` 只加 additive 声明。
- `deployment`、`scripts/skiff-instance.mjs`、AGENTS.md、scripts README、verify selector
  graph、verify.yml：本批次禁止触碰。
- 任何节点不得操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 验证 owner

- W-bootstrap：router cargo test（bootstrap/epoch/loader 序列测试 + fail-closed 负例）、
  SessionLayer 接入后 session 测试不回归、`verify --only router-rust,router-rust-process-smoke`。
- H-registration-cut：router TS tests + runtime tests + 共享 corpus consumer 测试全绿；
  rg 负例：legacy `runtime.register` 在 production 零命中；TS/Rust handshake wire 与
  contracts-session corpus 一致。
- W-model-*：transport/runtime/router consumer corpus 测试全绿；golden bytes 按契约 corpus
  逐字节一致（spawn 新 generation 以 C-model-spawn 为准）。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p runtime`
  （聚焦 consumer binary）、`check-local-instance.mjs`。

## 风险与停止条件

- H-registration-cut 是生产 wire 硬切：必须先让 TS/Rust consumer 过共享 corpus 再改 production；
  若现状与契约 corpus 偏差无法收敛，停止并附精确差异，不写兼容 reader/fallback。
- W-model-actor-spawn 的 `callerKind` 是 wire generation 升级：按现有 schema generation 规则
  实施，不做旧 shape 兼容；consumer 硬切（H-spawn-parent-cut）不属本节点。
- W-bootstrap 与 W-session 的 seam 装配若发现需要改 session 内部语义，停下上报。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@85596193，确认可执行后创建
自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务
文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b5 交接并通知主 Agent。
集成 Agent 在全部合入、探针通过后把 main push 到 origin/main（本批次明确授权）。
