# Router Rust Migration Batch 11 Cutover-registry Leaf Task

日期：2026-08-03

状态：execution leaf（一次性有界会话）

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-11.md`（节点 2
  cutover-registry；DAG、并行 ownership 边界、验证 owner、风险与停止条件）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）
  - §8 registry transition：迁移期手工 `router` selector 展开
    `router-ts-tests` + Rust subject + process smoke；cutover 时一次原子
    registry transition，删除 TS builder/manual graph，Rust subject 从
    `router-rust` 改为 `router` 唯一 owner；`router-rust-contracts` 是 Rust
    subject 自动生成的唯一 Cargo test leaf，manual `router` graph 展开
    subject selector 而不是重复 leaf；graph transition test 覆盖
    `implementation-tests`、manual `router` 和 Rust subject 展开后的 task
    去重。
  - §13 completion：TypeScript Router source/tests/package/lockfile/dist/CI/
    remote install 全部删除；长期 owner/contract 并入 `doc/architecture/`；
    本文改为 `complete`，不充当第二份架构规范。
- 仓库：`/Users/geek/workspace/skiff`，baseline `origin/main@6f03a59f`
  （`git rev-parse origin/main` 已验证）。
- worktree：`/Users/geek/workspace/wt-cutover-registry`，branch
  `feat/router-rust-cutover-registry`。

## 任务边界

1. verify registry 过渡：
   - Rust subject `router-rust` → `router` 唯一 owner（`skiff-router` /
     `router` workspace member），leaf `router-rust-contracts` →
     `router-contracts`，task ID `router-rust:contracts` → `router:contracts`；
   - manual `router` selector 只展开 Rust leaves（subject leaf +
     `router-rust-process-smoke`），删除 `router-ts-tests` expansion；
   - 删除 `router-ts-tests` builder（`implementation:router` pnpm task）；
   - registry transition test：`implementation-tests`、manual `router`、Rust
     subject 展开后的 task id/execution 去重；防止同名双 owner（subject
     selector/leaf/task id 唯一、manual graph 不重复注册 subject leaf）与
     workspace member 漏 owner（`assertRustWorkspaceOwnership` 全量覆盖）。
2. 更新 repo `AGENTS.md`：测试入口/组件说明移除 TS router（router 现为 Rust
   workspace crate `skiff-router`），保留 `skiff-tests` /
   `implementation-tests` 描述一致性。
3. 新增 `doc/architecture/router-rust.md`：长期 owner/contract 汇总（进程
   拓扑、state owner 表、wire/artifact/durable model 归属、named gates 与
   验证入口、rollback 策略），作为计划置 complete 后的唯一架构参考。
4. `doc/implementation/router-rust-migration-plan.md` 状态由 draft v5 改为
   complete，并在头部注明“实施完成，长期架构见
   `doc/architecture/router-rust.md`”，不充当第二份架构规范。

## 写入边界

- 允许：`scripts/lib/verify-rust-subjects.mjs`、
  `scripts/lib/verify-selector-graph.mjs`、`scripts/lib/verify-plan.mjs`、
  相关 verify 测试（`scripts/tests/verify-taxonomy.test.mjs`）、repo
  `AGENTS.md`、`doc/architecture/router-rust.md`（新）、
  `doc/implementation/router-rust-migration-plan.md`（仅状态/头部注释）、
  本叶子任务文件。
- 禁止：`router/`（TS 删除归 cutover-delete）、`scripts/skiff-instance.mjs`、
  `scripts/build-runtime-stack.mjs`、deploy 相关、`.github/workflows/verify.yml`
  （cutover-delete）、runtime crate、`runtime/transport/src`、deployment。

## 预检结论（origin/main@6f03a59f）

- `scripts/lib/verify-rust-subjects.mjs`：subject `router-rust`
  （leaf `router-rust-contracts`，task `router-rust:contracts`，package
  `router`/`skiff-router`）；`assertRustWorkspaceOwnership` 已防止 workspace
  member 漏 owner，`assertRustSubjectRegistryIntegrity` 已防止 selector/leaf/
  task id/workspace member 重复。
- `scripts/lib/verify-selector-graph.mjs`：
  `router: ['router-ts-tests', 'router-rust', 'router-rust-process-smoke']`；
  `implementation-tests` 展开含 Rust subject selectors + `router`；
  `publicSelectors` 含 `router-rust`。
- `scripts/lib/verify-plan.mjs`：`router-ts-tests` builder（pnpm
  `implementation:router`）、`router-rust-process-smoke` builder（node
  `router-rust:process-smoke`）、Rust subject 自动生成 Cargo leaf；
  `assertOrdinaryTaskBuilderCoverage` 强制 builder 与 leaf 一一对应。
- `scripts/tests/verify-taxonomy.test.mjs`：断言当前迁移期命名，需更新为
  过渡后命名并新增/保留去重与漏 owner 防护。
- `scripts/lib/verify-live-registry.mjs` 的 `router-rust-*-live` 是 live
  selector 命名，按设计 §8 保留，不属于本节点 subject 重命名范围。
- `.github/workflows/verify.yml` 与 `scripts/tests/verify-rust-quality.test.mjs`
  仍引用 `--only router-rust,router-rust-process-smoke`；workflow 文件归
  cutover-delete，本节点不改，交接时显式上报集成协调点。

## 自验收

- `node scripts/verify.mjs --only implementation-tests,router --list`：
  无 `router-ts-tests`、无 `router-rust` selector；`router` 只含 Rust leaves
  （`router-contracts` + `router-rust-process-smoke`）。
- `node --test scripts/tests/verify-taxonomy.test.mjs` 全绿（transition
  去重、同名双 owner、workspace member 漏 owner）。
- `doc/architecture/router-rust.md` 与计划状态 complete 落盘。

## 交接

完成自验收后提交到 `feat/router-rust-cutover-registry`，直接交接
`/root/router_rust_integration_b11` 并通知 root。发现 registry 双 owner 或漏
owner 时停下上报。
