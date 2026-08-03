# Router Rust Migration Batch 11（E-cutover：默认切 Rust + 删除 TS Router + 收尾）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-10.md`（已 push 到
origin/main@6f03a59f）。本批次实现权威设计 §7 E-cutover、§8 registry transition、
§11.3 hard cut、§13 completion，不修改设计语义。

本地 main 仍在用户并行线；一律以 origin/main 为基线；共享主 worktree 只读。
本批次不重启/不切换 stable instance（用户共享基础设施，实际切换由用户触发）；
isolated fixture 与 release 工具链在本批验证 Rust binary 管理。

## 批次目标

### 节点 1：TS Router 删除 + 工具链收口（cutover-delete）

- 删除 `router/` 下全部 TS source/tests/package/lockfile/tsconfig/dist 与 differential
  harness（TS 侧引用）；`router/` 变为纯 Rust crate；
- `scripts/skiff-instance.mjs`：删除 TS process match/spawn 路径，`router.implementation`
  默认切 `rust`，`RouterProcessSpec` 恒为 Rust；
- `scripts/build-runtime-stack.mjs`：删除 tsUnit('router') 与 TS 依赖；deploy/remote
  install 删除 pnpm/tsx router 路径；
- `.github/workflows/verify.yml`：删除 `pnpm --dir router install` 与 router TS 相关 scope；
  `loop-risk-stress-node.mjs` 的 `ws` 依赖归 `scripts/package.json`；
- 残留 gate 通过：`rg --files router | rg '\.(ts|tsx)$'` 无结果、
  `test ! -e router/package.json`、`rg '@skiff/router|pnpm --dir router|tsx.*router'`
  在 production/CI/tooling 无结果（历史 implementation record 可保留）。

### 节点 2：verify registry 过渡 + 文档收尾（cutover-registry）

- Rust subject `router-rust` → `router` 唯一 owner；manual `router` selector 只展开 Rust
  leaves；删除 `router-ts-tests`；registry transition test 防止同名双 owner 或 workspace
  member 漏 owner（覆盖 implementation-tests、manual router、Rust subject 展开去重）；
- 更新 repo `AGENTS.md`（测试入口/组件说明去除 TS router）；
- 新增 `doc/architecture/router-rust.md`（长期 owner/contract 汇总：进程拓扑、state owner、
  wire/artifact/durable model 归属、named gates）；`doc/implementation/router-rust-migration-plan.md`
  状态改为 `complete`（不充当第二份架构规范）。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| cutover-delete | §11.3、§8 残留 gate | origin/main@6f03a59f | `feat/router-rust-cutover-delete` / `wt-cutover-delete` |
| cutover-registry | §8 registry transition、§13 docs | origin/main@6f03a59f | `feat/router-rust-cutover-registry` / `wt-cutover-registry` |

## 并行 ownership 边界

- `router/` 删除（TS 文件、package、tests、differential TS 侧）：仅 cutover-delete；
  `router/Cargo.toml/src`（Rust）cutover-delete 可做必要清理（如 Cargo.lock），不改语义。
- `scripts/skiff-instance.mjs`、`scripts/build-runtime-stack.mjs`、`deploy-runtime-stack.mjs`、
  `scripts/package.json`、`.github/workflows/verify.yml`、loop-risk stress 脚本：
  仅 cutover-delete。
- `scripts/lib/verify-rust-subjects.mjs`、`verify-selector-graph.mjs`、`verify-plan.mjs`、
  registry transition 测试：仅 cutover-registry。
- repo `AGENTS.md`、`doc/architecture/router-rust.md`、migration plan 状态：仅
  cutover-registry。workspace `AGENTS.md`（git 外）由主 Agent 本地更新。
- 两个节点共享 `router/Cargo.lock`/根 `Cargo.lock`（若有变化）：允许机械并集。
- runtime crate、runtime/transport/src、deployment：本批禁止触碰。
- 共享主 worktree 只读；基线 origin/main@6f03a59f；不操作 stable instance。

## 验证 owner

- cutover-delete：残留 gate 三连通过；`cargo test -p skiff-router` 全绿；
  `verify --only router-rust,router-rust-process-smoke` 通过；isolated fixture 的
  instance build/up 用 Rust binary 通过；`pnpm install` 面不再引用 router。
- cutover-registry：`verify --list` 无 router-ts-tests、`router` selector 只含 Rust
  leaves、transition test 通过；AGENTS.md/架构文档/计划状态落盘。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p
  skiff-router`、`cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p
  runtime`、`check-local-instance.mjs`、残留 gate 三连。

## 风险与停止条件

- 删除 TS 前先确认 rollback unit 已可重建（Batch 10 已验证）；删除动作独立 commit，
  保留历史可恢复。
- registry 过渡若发现 workspace member 漏 owner 或同名双 owner，停下上报。
- 若删除后发现 tooling 仍有 TS router 引用（CI/scripts/production），停下补齐后再合入。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 origin/main@6f03a59f，确认可执行后
创建自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子
任务文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b11 交接并通知主
Agent。集成 Agent 在全部合入、探针通过后 push origin/main（已授权；本地 main/共享主 worktree
一律不碰；不操作 stable instance）。
