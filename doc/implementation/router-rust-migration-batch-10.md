# Router Rust Migration Batch 10（parity/chat/rollback/编译器集成收尾）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-9.md`（已 push 到
origin/main@edc111f8）。本批次实现权威设计 §7 E-actor-parity/E-chat、§11.2 rollback、
§2.4 A1 compiler 集成、§9 differential 扩展，不修改设计语义。

本地 main 仍在用户并行线；一律以 origin/main 为基线；共享主 worktree 只读。

## 批次目标

- A1-compiler 集成：把 A1 producer 接进 compiler publish 路径（发布时自动生成
  `records/actor-routing/current.json`，A2 已要求 production artifact root 必须含该记录）；
  机械更新 `deployment/tests/fixtures/bootstrap-chain-corpus.json` 的 pendingPresent 标签
  （failClosedPending → recovery 语义）。
- WS-only routing：runtime/host control_plane 的 dispatch_modes 统计扩展为包含 WebSocket
  surface（E-ws 记录的缺口），加 WS-only deployment 路由测试。
- E-actor-parity：ownerLeaseId mint reconciliation（E-actor-rust 记录 seam）+ TS/Rust
  differential full-chain（扩展 W-differential harness 的 actor 场景，router-live:actor
  原子扩展为 differential，A2 已合入）+ parity 证据。
- E-chat：pinned service artifact manifest（Skiff SHA + internals SHA + skiff-packages SHA）
  + `internals/agine` 的 `npm run e2e:chat-smoke` 跑在 isolated Rust Router 实例上；
  本地使用同一 manifest schema 与命令；记录证据（真实 CI 的 private workflow 归 internals）。
- Rollback 终态：immutable TS rollback unit builder 完成（pinned Node runtime + 最后 TS
  source + materialized 依赖/offline store + package/lockfile + process spec + file/source
  identity），全新临时目录离线启动演练（E-http 已有首次 unary roundtrip，本批扩展为
  release-candidate 级）；clean-host 准备（binary + config + artifacts，无 pnpm/tsx PATH）。
- Differential 扩展与记录：HTTP/WS/actor 场景扩展；X-Skiff-Release TS 201 vs Rust 400 差异、
  backpressure macOS 边界等记录进 differential docs（非阻塞）。

退出检查点：节点合入集成分支并 push origin/main（不碰本地 main/共享主 worktree），探针通过，
worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| A1-compiler | §2.4 A1、§7 E-actor-parity 前置 | origin/main@edc111f8 | `feat/router-rust-a1-compiler` / `wt-a1-compiler` |
| WS-only routing | §7 E-ws 残余 | origin/main@edc111f8 | `feat/router-rust-ws-only-routing` / `wt-ws-only-routing` |
| E-actor-parity | §7 E-actor-parity、§8 router-live:actor | origin/main@edc111f8 | `feat/router-rust-e-actor-parity` / `wt-e-actor-parity` |
| E-chat | §7 E-chat、§8 router-live:chat | origin/main@edc111f8 | `feat/router-rust-e-chat` / `wt-e-chat` |
| rollback 终态 | §11.2 | origin/main@edc111f8 | `feat/router-rust-rollback-final` / `wt-rollback-final` |
| differential 扩展 | §9 | origin/main@edc111f8 | `feat/router-rust-differential-ext` / `wt-differential-ext` |

## 并行 ownership 边界

- runtime crate：仅 WS-only routing 节点（control_plane dispatch_modes 统计扩展）；
  E-actor-parity 若需 runtime 改动先停下上报。
- compiler/deployment：仅 A1-compiler 节点。
- scripts differential harness：W-differential 扩展节点与 E-actor-parity 节点都会用
  differential 脚本——按文件前缀划分（`differential_ext_*` / `actor_parity_*`），共享
  inventory 文档由 differential 扩展节点统一维护。
- internals 仓库：仅 E-chat 节点（只读运行 smoke，不改 internals 代码；如需改 internals
  先上报）。
- rollback 终态：scripts/lib/rollback-manifest.mjs 与相关 fixtures 仅 rollback 节点。
- registry/CI：各 gate 节点只 append 自己的条目/job（E-actor-parity 更新 actor 条目描述为
  differential；E-chat 新增 router-live:chat 本地条目与 CI job 占位）。
- AGENTS.md、scripts README、verify selector graph、skiff-instance.mjs：本批禁止触碰。
- 共享主 worktree 只读；基线 origin/main@edc111f8；磁盘纪律同 Batch 9（只清自己的 target）。

## 验证 owner

- A1-compiler：compiler publish 后 artifact root 自动含 actor-routing/current.json（真实
  compiler 产物测试）；A2 侧 loadActorMethods 对真实产物可用；deployment corpus 标签更新
  后测试全绿。
- WS-only routing：真实 WS-only deployment 经真实 Router 路由成功（E-ws harness 去掉
  HTTP 兜底条目后 PASS）。
- E-actor-parity：ownerLeaseId 归零/一致性 + TS/Rust actor differential 无未解释差异 +
  router-live:actor 扩展为 differential 全链。
- E-chat：isolated Rust instance 上 chat smoke PASS，manifest 记录三仓库 SHA。
- rollback 终态：immutable TS unit 在全新临时目录离线启动并完成 unary；clean-host 演练
  （PATH 无 pnpm/tsx）通过。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p
  skiff-router`、`cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p
  runtime`、`check-local-instance.mjs`。

## 风险与停止条件

- E-chat 需要 internals 仓库可用且 agine 服务可构建；若本地 chat smoke 基础设施缺失或
  需要修改 internals 代码，停下上报，不擅自跨仓库改动。
- E-actor-parity 的 differential 若出现未解释差异，先定位 owner，不掩盖。
- rollback 终态若发现 TS unit 无法离线启动（依赖未物化），停下上报。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 origin/main@edc111f8，确认可执行后
创建自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子
任务文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b10 交接并通知主
Agent。集成 Agent 在全部合入、探针通过后 push origin/main（已授权；本地 main/共享主 worktree
一律不碰）。
