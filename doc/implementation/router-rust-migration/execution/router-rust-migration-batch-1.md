# Router Rust Migration Batch 1（PR 0a + C0）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，2026-08-01）。
本批次是把权威设计 §5.1 / §5.2 / §6.2 / §7 / §8 / §11.2 转成可执行节点的实现拆分，不修改设计语义。
叶子任务引用本文件；本文件继续引用权威设计。

## 批次目标

落地迁移的第一波，全部在 `skiff/` 仓库内（本地 main，不 push）：

- PR 0a：instance `router.implementation: ts|rust` 迁移期字段 + 唯一 `RouterProcessSpec`、
  空 `skiff-router` Cargo binary、`routerBinary` dev path / build-install placeholder / process match /
  binary SHA-256 identity、`router-rust` Rust subject（leaf `router-rust-contracts`）、
  `router-rust:process-smoke`、manual `router` selector 迁移期展开、verify/CI 快速入口、
  rollback manifest/schema/builder 与 TS/Rust process commands。
- C0-control：`?detail=loop-risk` 投影移入 production `AssemblyControlPlane` 并更新 evaluator/
  self-test/live baseline；canonical control 契约统一到 `/__skiff/activate-assembly`；
  router production 与 tests 删除 stale `/__skiff/reload-artifacts`。
- C-config：冻结唯一 Router process config schema/defaults/relative-path resolution/secret redaction/
  unknown-key policy 与 golden invalid corpus；renderer 删除未声明/未消费字段
  （含 `ecosystemStoreCliPath`）；更新 repo/workspace `AGENTS.md`、`scripts/README.md`、
  local instance checks 与 config surface。

退出检查点：三个节点全部合入本地 main（不 push origin），focused 证据通过，
一级 worktree 与已合并临时分支清理完毕，批次报告落盘。

## DAG 节点

| 节点 | 对应设计条款 | 基线 | 分支 / worktree | 集成目标 |
| --- | --- | --- | --- | --- |
| C0-control | §2.5、§5.2 C0、§8 loop-risk | main@9e492fa7 | `feat/router-rust-c0-control` / `/Users/geek/workspace/wt-c0-control` | router_rust_integration_b1 |
| C-config | §2.5 C-config | main@9e492fa7 | `feat/router-rust-c-config` / `/Users/geek/workspace/wt-c-config` | router_rust_integration_b1 |
| PR 0a | §5.1、§6.2(1)、§7 PR0a、§8、§11.2 | main@9e492fa7 | `feat/router-rust-pr0a` / `/Users/geek/workspace/wt-pr0a` | router_rust_integration_b1 |

三节点互不依赖，可并行；集成 Agent 串行合入 `integration/router-rust-migration-batch-1`。

## 并行 ownership 边界（写文件声明）

- `scripts/skiff-instance.mjs`：C-config 只改 `routerConfigText` / `urls.routerReload` 相关行；
  PR 0a 只改 process spawn/match 与 `router.implementation` / `RouterProcessSpec` 解析；
  C0-control 不写该文件。
- `AGENTS.md`（repo + workspace）、`scripts/README.md`、`scripts/lib/local-instance-config.mjs`、
  `scripts/check-local-instance.mjs`：仅 C-config。
- `router/src/router/controlPlane.ts`、stale reload handler 所在 gateway、loop-risk evaluator
  （`scripts/lib/loop-risk-*.mjs`、`scripts/check-loop-risk-*.mjs`）、`verify-live-registry.mjs`
  loop-risk 条目、router control/reload 相关 tests：仅 C0-control。
- `Cargo.toml` workspace members、`scripts/lib/verify-rust-subjects.mjs`、
  `scripts/lib/verify-selector-graph.mjs`、`scripts/lib/verify-plan.mjs`（router selectors）、
  `.github/workflows/verify.yml`、rollback manifest builder、`router/` 下新增 Cargo package：仅 PR 0a。
- `verify-live-registry.mjs` 中 local-instance 条目：仅 C-config。
- workspace `AGENTS.md` 位于 git 仓库外，只做本地修改，不提交、不 push。
- 任何节点不得修改 `.skiff-instance/` 的稳定配置语义、不得重启/操作本机 stable instance、
  不得动 MongoDB / PM2 / 4004-4007 端口进程。

## 验证 owner

聚焦验证，不跑全量 `pnpm verify`，不把全量 gate 放关键路径：

- C0-control：router TS tests + loop-risk hermetic self-test + `rg` 负例（reload-artifacts 不在
  router production；health handler 不再内联 `?detail=loop-risk` 投影）。
- C-config：`node scripts/check-local-instance.mjs` + router type-check/tests + scripts tests +
  golden invalid corpus 负例。
- PR 0a：`cargo test --package skiff-router` + `node scripts/verify.mjs --only router-rust,router-rust-process-smoke`
  + `verify --list` 展开断言 + subject registry integrity + rollback builder 自测。
- 集成探针（integration agent 唯一 owner）：合流后 `verify --only router-rust,router-rust-process-smoke`、
  `verify --list` 展开断言、`node scripts/check-local-instance.mjs`。

## 风险与停止条件

- `skiff-instance.mjs` 有两个写入者，边界按函数划分；集成 Agent 只处理机械冲突，语义冲突停下上报主 Agent。
- 发现设计空洞、公共契约变化、需要新顶层配置/manifest/schema/集中式 owner 时，叶子返回
  `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE` 并附精确证据，不原地猜测设计。
- 每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@9e492fa7，确认可执行后
  创建自己的 worktree，在第一次 production 修改前形成完整叶子任务文件（引用本批次文档），
  完成后直接向 router_rust_integration_b1 交接并通知主 Agent。
