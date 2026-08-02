# Router Rust Migration C0-control Leaf Task

日期：2026-08-02
节点：C0-control（一次性有界会话）
基线：`main@9e492fa77bb5129a5d872f964959449e929c2051`
分支 / worktree：`feat/router-rust-c0-control` / `/Users/geek/workspace/wt-c0-control`
集成目标：`/root/router_rust_integration_b1`

## 引用

- 批次文档：`doc/implementation/router-rust-migration-batch-1.md`（C0-control 行、并行 ownership 边界、验证 owner）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md` §2.5（TS baseline cleanup 前置项）、§5.2（C0 为 C-net 硬前置）、§8（loop-risk live gates、TS test 删除 ledger 约定）、§10（loop-risk evaluator 与 health snapshot 语义）。
- 冲突时以权威设计为准。

## 只读预检结论（零 worktree，锚定 9e492fa7）

1. 生产调用链：`router/src/router/server.ts` 只装配 `AssemblyControlPlane`（
   `new AssemblyControlPlane({...})` 后传入 `runtimeEndpoint.listen({ controlPlane, ... })`）。
   `RouterControlPlane`（`router/src/router/controlPlane.ts`）仅被 router tests 使用
   （`router/tests/helpers/routerHarness.ts`、`test-dispatch*.test.ts`、`artifact-reload.test.ts`、`loop-risk-health.test.ts`）。
2. `?detail=loop-risk` 投影当前内联于 `RouterControlPlane` 的 `/__router/health` handler
   （`controlPlane.ts` line ~125），数据源是 `RuntimeRegistry.loopRiskRuntimeHealthSnapshot()`。
   但 production（`RuntimeEndpoint` line ~760-774）在 assembly 模式下把 `runtime.health` 记入
   `AssemblyRuntimeRegistry.recordHealth()`；因此 canonical owner 的 loop-risk `runtimes`
   必须改由 `AssemblyRuntimeRegistry.snapshot()`（replicaId/connected/lastHealthAt/healthCounters）投影，
   保持 wire shape 不变：`loopRisk.observedAt / router.dispatcher / router.httpStream / runtimes[]`。
3. `router.httpStream` counters 的生产来源是 `AssemblyHttpGateway.streamLifecycleCounters()`；
   `AssemblyControlPlane` 目前没有该来源，需要新增可选 counter source + setter，并在 `server.ts` 装配。
4. stale `/__skiff/reload-artifacts` 存在三处：`controlPlane.ts` 的完整 handler（含 overrides 解析、
   in-flight dedup、snapshot replace/broadcast）、`httpGateway.ts` 的 public 404 stub、`artifact-reload.test.ts`。
   canonical control 契约 `/__skiff/activate-assembly` 已由 `AssemblyControlPlane` 提供（
   `protocol/assemblyActivationProtocol.ts` `POST /__skiff/activate-assembly`），无需新端点。
5. 测试 inventory/ledger：仓库当前没有 router test ledger 文件；按权威设计 §9“每删除一个 TS test，
   ledger 标记 retired/shared owner/…”，本节点创建批次 ledger
   `doc/implementation/router-rust-migration-batch-1-test-ledger.md`。
6. 兄弟节点 ownership：C-config 拥有 `verify-live-registry.mjs` local-instance 条目、
   `local-instance-config.mjs`、`check-local-instance.mjs`、`scripts/README.md`、AGENTS.md；
   C0 只可能触碰 `verify-live-registry.mjs` 的 loop-risk 条目（预检结论：无需改动，wire 契约不变）。
   PR 0a 拥有 Cargo/process seam；与 C0 无文件重叠。
7. 预检时主 worktree 已被集成 Agent 切到 `integration/router-rust-migration-batch-1`（其上加了一个 docs
   commit 7abba712，仅新增批次文档）；`main` ref 仍为 9e492fa7，baseline 未实质改变。本分支严格从
   `main@9e492fa7` 创建，不包含集成分支的 docs commit。

## 写入范围（严格）

- `router/src/router/controlPlane.ts`：删除 reload handler/options/methods/overrides 解析与
  loop-risk 内联投影/计数器源（保留 plain `/__router/health`、`/__router/prune-runtimes`、
  `/__skiff/test-dispatch`）。
- `router/src/router/assemblyControlPlane.ts`：新增 `?detail=loop-risk` 投影（canonical owner）、
  可选 `httpStreamCounters` 来源 + setter、runtimes 由 `AssemblyRuntimeRegistry` 投影。
- `router/src/router/httpGateway.ts`：删除 `/__skiff/reload-artifacts` 404 stub（reload handler 所在 gateway）。
- `router/src/router/server.ts`：production 装配 httpStream counter source（任务要求移入 production
  canonical owner 的必要 wiring；批次 ownership 表未列 server.ts，但这是唯一生产装配点，列入本叶子写集）。
- `router/tests/artifact-reload.test.ts`：删除并迁移保留用例到 `router/tests/router-control-plane.test.ts`
  （reload 用例 retired；runtime-control broadcast / prune-runtimes 用例 re-owned）。
- `router/tests/loop-risk-health.test.ts`：改为以 `AssemblyControlPlane` fixture 验证 canonical 投影。
- `router/tests/helpers/routerHarness.ts`：`createControlPlane` 移除 reloadArtifacts 参数（如仍被引用）。
- `scripts/check-loop-risk-health.mjs`：self-test 增加 canonical 投影的 disconnected-session 基线用例。
- `scripts/lib/loop-risk-*.mjs`：预检结论 wire shape 不变，无需逻辑改动；仅在有真实需要时同步。
- `scripts/lib/verify-live-registry.mjs`：仅 loop-risk 条目；预检结论无需改动，若改动仅限描述文本。
- 测试 inventory/ledger：新建 `doc/implementation/router-rust-migration-batch-1-test-ledger.md`。
- 本叶子文件。

## 禁止写 / 非目标

- 不写 `scripts/skiff-instance.mjs`、`scripts/lib/local-instance-config.mjs`、
  `scripts/check-local-instance.mjs`、AGENTS.md（repo/workspace）、`scripts/README.md`、
  `Cargo.toml`、`scripts/lib/verify-rust-subjects.mjs`、`scripts/lib/verify-selector-graph.mjs`、
  `scripts/lib/verify-plan.mjs`、`.github/workflows/verify.yml`、router/ 下任何 Cargo package、
  rollback manifest builder（分别归 C-config / PR 0a）。
- 不新增端点、不新增配置、不新增集中式 owner；不重启/修改 stable instance、Mongo、PM2、4004-4007 端口进程；
  不跑全量 `pnpm verify`。
- 不改变路由/HTTP/WS/actor 外部语义；control listener 上 `/__router/health`、`/__skiff/test-dispatch`、
  `/__skiff/activate-assembly` 行为保持不变。
- `scripts/tests/verify-live-registry.test.mjs` 等 scripts tests 不在写集；若实现要求改动它们
  （目前预期不需要），视为 TASK_SCOPE_EXPANDED 上报。

## 完成标准

1. production `AssemblyControlPlane` 独占 `?detail=loop-risk` 投影；`controlPlane.ts` 的 health handler
   不再内联该投影。
2. router production 无 `/__skiff/reload-artifacts` handler；public gateway 与 control listener 对该路径
   均回落 404；canonical control 契约只有 `/__skiff/activate-assembly`。
3. loop-risk evaluator/self-test/live baseline 与 canonical 投影同步；wire shape 字段不变
   （observedAt/router.dispatcher/router.httpStream/runtimes）。
4. 被删除的 legacy tests 在 batch-1 test ledger 标记 retired / re-owned。
5. 聚焦验证通过：`pnpm --dir router test`、`node scripts/check-loop-risk-health.mjs --self-test`、
   `rg` 负例（`reload-artifacts` 不在 router production；`loop-risk` 不在 controlPlane.ts）。

## 聚焦验证命令

```bash
pnpm --dir router test
node scripts/check-loop-risk-health.mjs --self-test
rg -n "reload-artifacts" router/src router/tests   # 期望无结果（router 内）
rg -n "loop-risk|loopRisk" router/src/router/controlPlane.ts   # 期望无结果
node scripts/verify.mjs --only checks-default --list   # 可选：确认 checks:loop-risk-health:self-test 在默认计划
```

## 停止条件

- 需要改变公共契约/架构职责/设计语义 → 返回 `TASK_SCOPE_EXPANDED`（附精确代码路径与证据）。
- 发现需要独立 owner 的能力节点、一次预检后仍无单一明确实现路径、需要未授权外部状态 →
  返回 `TASK_NOT_EXECUTABLE`。
- 兄弟 ownership 冲突或基线被实质改变 → 通知 root 协调，不直接写入。
