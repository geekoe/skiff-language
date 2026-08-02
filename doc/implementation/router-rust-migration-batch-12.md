# Router Rust Migration Batch 12（health projection + release workflow）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01，已置
complete 但本批为计划 §3.2/§10/§8 遗留项的最终落地）。
直接父批次：`doc/implementation/router-rust-migration-batch-11.md`（已 push 到
origin/main@ea8616bc）。

本地 main 仍在用户并行线；一律以 origin/main 为基线；共享主 worktree 只读；不操作
stable instance。

## 批次目标

- Rust `/__router/health` 生产投影：`HealthAggregator` 聚合各 owner 快照，输出与 TS 兼容的
  health JSON（ok/activeAssembly/pendingActivation/replicas 等现有消费者所需字段）+ §10
  计数面（active routing epoch、sessions/capabilities/health/barrier、admission permits/
  cursor、request pending、client generations、generation leases、broker pending/tombstone、
  actor ownership/claim/invocation/control/lease timers、activation durable/live/recovery、
  per-owner mailbox、writer queues、spawned tasks/timers/shutdown residue）+ 
  `?detail=loop-risk` 投影 parity；loop-risk evaluator/live harness 与 external
  `runtime-live` 消费同一 Rust health。
- scheduled release workflow：`.github/workflows/router-rust-release.yml`
  （scheduled + workflow_dispatch）运行 clean-host（Linux binary/PM2，无 pnpm/tsx）、
  loop-risk、完整 rollback 演练。

退出检查点：节点合入集成分支并 push origin/main（不碰本地 main/共享主 worktree），探针通过，
worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| health projection | §3.2 HealthAggregator、§10 health、§8 runtime-live/loop-risk | origin/main@ea8616bc | `feat/router-rust-health` / `wt-health` |
| release workflow | §8 CI release workflow | origin/main@ea8616bc | `feat/router-rust-release-ci` / `wt-release-ci` |

## 并行 ownership 边界

- `router/src/health/`（或预检确认位置）+ listener 的 /__router/health 路由 + loop-risk
  evaluator/脚本：仅 health 节点。
- `.github/workflows/router-rust-release.yml`（新）：仅 release-ci 节点；它不得改
  router-rust-integration.yml（除非 append 自己的 job，先上报）。
- runtime crate、runtime/transport/src、deployment、router TS（已删除）、AGENTS.md、
  scripts README、verify selector graph：本批禁止触碰。
- 共享主 worktree 只读；基线 origin/main@ea8616bc；不操作 stable instance。

## 验证 owner

- health：`/__router/health` 真实响应与 TS 兼容 shape（test-runner/外部 live gate 可消费）、
  `?detail=loop-risk` 字段齐全、owner 计数归零断言、loop-risk hermetic self-test 与
  live harness（隔离实例）通过、`cargo test -p skiff-router` 全绿。
- release-ci：workflow YAML 解析、jobs 引用真实存在的 scripts/check 命令、`--list` 展开
  对应 live selector。
- 集成探针：`verify --only router,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p runtime`、
  `check-local-instance.mjs`、残留 gate 三连（保持 Batch 11 状态）。

## 风险与停止条件

- health shape 必须兼容既有消费者（test-runner runtime_execution/wire.rs、runtime-live、
  loop-risk）；若需改消费者契约先更新 corpus/测试再实现。
- release-ci 的 clean-host job 只在 GitHub Linux runner 上真实执行，本机只做 dry-run/解析；
  不能把本地 macOS 结果冒充 Linux gate。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 origin/main@ea8616bc，确认可执行后
创建自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子
任务文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b12 交接并通知主
Agent。集成 Agent 在全部合入、探针通过后 push origin/main（已授权；本地 main/共享主 worktree
一律不碰；不操作 stable instance）。
