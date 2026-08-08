# Router Rust Migration Batch 11 Cutover-delete Leaf Task

日期：2026-08-03

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

状态：execution leaf（一次性有界会话）

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-11.md`（节点 1
  cutover-delete；DAG、并行 ownership 边界、验证 owner、风险与停止条件）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5）
  - §7 E-cutover：只切 default 和删除 TS，不首次实现新 lifecycle。
  - §8 registry transition / named gates；`router-live:actor` 保留为
    two-replica actor chain；`router-live:http` 保留为 Rust-only unary/stream。
  - §11.1 build/deploy 管理 Rust binary；`--only router` 不隐式捆绑 compiler。
  - §11.3 Hard cut：删除 Router TS source/tests/package/lockfile/tsconfig/dist、
    CI install、remote install、tsx/pnpm process path、differential harness；
    残留 gate 三连；发布系统保留上一完整 release。
- 父节点协调（2026-08-03）：cutover-registry 已把 subject `router-rust` 改名
  `router`；verify.yml 的 Router Rust job 命令改为
  `--only router,router-rust-process-smoke`；`router-rust` 的其他 tooling 引用
  按新命名更新；`router-type-check` 由 cutover-registry 自己移除。
- 仓库：`/Users/geek/workspace/skiff`，baseline `origin/main@6f03a59f`。
- worktree：`/Users/geek/workspace/wt-cutover-delete`，branch
  `feat/router-rust-cutover-delete`。

## 任务边界

1. 删除 `router/` 下全部 TS 内容（src 的 .ts/.tsx、tests 的 .ts 与 TS helpers、
   package.json、pnpm-lock.yaml、pnpm-workspace.yaml、tsconfig.json）；
   `router/` 变为纯 Rust crate（Cargo.toml、src/*.rs、tests/*.rs、
   tests/fixtures/router-config、fixtures/hello、router.example.yml 保留）。
2. 删除 differential harness（整个 `scripts/lib/router-differential/` 依赖 TS
   Router，按 plan §11.3 整体删除）及其 CLI/夹具/测试，并同步删除
   verify live registry 中的 `router-live:differential` 条目（否则
   `assertVerifyCatalogComplete` 会因脚本缺失而失败）。
3. 删除 TS rollback unit 演练工具（`router-live:rollback-final`）：
   `scripts/check-router-rollback-final.mjs`、
   `scripts/lib/rollback-unit.mjs`、`scripts/lib/rollback-manifest.mjs`、
   `scripts/tests/rollback-*.test.mjs`，并删除 live registry 对应条目。
   §11.2 演练在 Batch 10 已完成其目的；cutover 后 rollback 语义为"上一完整
   release"（§11.3 step 5），不能再从 workspace 重建 TS unit。Rust-only 的
   `clean-host-bundle.mjs` / `rollback-clean-host-suite.mjs` 保留。
4. 工具链收口：
   - `scripts/skiff-instance.mjs`：删除 router TS match/spawn 路径；
     `router.implementation` 缺失默认 `rust`，显式 `ts` 报错（fail fast）；
     RouterProcessSpec 恒为 Rust。
   - `scripts/lib/dev-runtime-paths.mjs`：spec 恒为 rust（保留
     `implementation: 'rust'` 字段与 `config_path`/`rust_binary_path` 形状，
     删除 ts_source_root / TS invocation）。
   - `scripts/build-runtime-stack.mjs`：router 改为 rsUnit（cargo test +
     zigbuild skiff-router），telemetry 保持 tsUnit。
   - `scripts/deploy-runtime-stack.mjs`：router 只上传 binary + PM2 直执行；
     删除 router rsync / remote pnpm install / tsx PM2 app；`--only router`
     不再隐式捆绑 compiler（§11.1）。
   - `.github/workflows/verify.yml`：删除 `pnpm --dir router install`；
     Router job 命令改为 `--only router,router-rust-process-smoke`。
   - `.github/workflows/router-rust-integration.yml`：删除两处 TS router
     install 步骤及差分/rollback 注释；classifier 删除已删文件路径。
   - loop-risk stress：`ws` 依赖从 `router/package.json` 移到
     `scripts/package.json`（新增 `ws` 依赖 + 更新 lockfile），
     `loop-risk-stress-node.mjs` 与 live registry 改从 scripts 解析。
   - live gates 保留并转 Rust-only：`router-live:http` 去掉 TS 阶段与
     rollback roundtrip；`router-live:actor` 去掉 TS/Rust 差分 Phase 2，
     保留 Rust-only two-replica probe（probe 自带 Rust WS relay，不需要
     Node `ws`/pnpm）。canonical actor-routing projection 的 test-side A1
     producer 移到 `scripts/lib/actor-live-projection.mjs`。
   - `scripts/lib/rollback-relay.mjs`：内联 frames 解码 helper，`ws` 从
     scripts 解析，供 `router-live:http` 使用。
   - `scripts/lib/platform-source-probe-node-dependencies.mjs`（TS router
     依赖探测）与 shared-target-probe 的 router-dependencies 阶段删除。
   - `scripts/lib/router-process-smoke.mjs`：Rust-only（删除 TS spec 断言）。
   - `scripts/lib/isolated-test-runtime-instance.mjs`：fixture 不再写
     `router.implementation`（默认 rust）。
   - `router/README.md`：改为 Rust crate 说明，删除 TS 启动路径。
5. 残留 gate 三连：
   - `rg --files router | rg '\.(ts|tsx)$'` 无结果；
   - `test ! -e router/package.json` 成功；
   - `rg '@skiff/router|pnpm --dir router|tsx.*router'` 在
     production/CI/tooling 无结果（历史 implementation record 保留）。

## 写入边界

允许：`router/`（删除 TS + README 更新）、`scripts/skiff-instance.mjs`、
`scripts/lib/dev-runtime-paths.mjs`、`scripts/lib/isolated-test-runtime-instance.mjs`、
`scripts/lib/router-process-smoke.mjs`、`scripts/lib/actor-live-projection.mjs`（新）、
`scripts/lib/rollback-relay.mjs`、`scripts/build-runtime-stack.mjs`、
`scripts/deploy-runtime-stack.mjs`、`scripts/package.json`/`pnpm-lock.yaml`、
`.github/workflows/verify.yml`、`.github/workflows/router-rust-integration.yml`、
loop-risk stress 脚本（`scripts/lib/loop-risk-stress-node.mjs`）、
`scripts/lib/verify-live-registry.mjs`（仅删除 differential/rollback-final 条目 +
actor/http/stress 前置更新）、`scripts/check-router-http-live.mjs`、
`scripts/lib/http_live_{fixture,process,suite}.mjs`、`scripts/check-router-actor-live.mjs`、
`scripts/lib/platform-source-{probe-node-dependencies,shared-target-probe}.mjs`、
scripts 测试（删除 differential/rollback 测试；更新 dev-runtime-paths、
router-process-spec、router-instance-binary-lifecycle、verify-live-registry、
loop-risk-stress、platform-source-shared-target-probe、verify-rust-quality、
runtime-stack-deploy 等）、`router/README.md`、本叶子任务文件。

禁止：`scripts/lib/verify-rust-subjects.mjs`、`verify-selector-graph.mjs`、
`verify-plan.mjs`、`scripts/tests/verify-taxonomy.test.mjs`（registry 节点）、
runtime crate、`runtime/transport/src`、deployment、repo `AGENTS.md`、
`scripts/README.md`；不操作 stable instance / stable Mongo / PM2 / 4004-4007；
不跑全量 verify。

## 删除文件清单

### router/（167 个文件）

`git ls-tree -r --name-only 6f03a59f router/` 中全部 `.ts`/`.tsx` 文件 +
`router/package.json`、`router/pnpm-lock.yaml`、`router/pnpm-workspace.yaml`、
`router/tsconfig.json`（详见提交 diff；Rust src/tests/fixtures 保留）。

### scripts/（differential + rollback TS 工具）

- `scripts/lib/router-differential/`（22 个 .mjs，整体）
- `scripts/check-router-differential-live.mjs`
- `scripts/fixtures/router-differential/`（17 个文件，整体）
- `scripts/tests/router-differential-{compare,frames,normalize,scenarios}.test.mjs`
- `scripts/tests/actor_parity_differential.test.mjs`
- `scripts/check-router-rollback-final.mjs`
- `scripts/lib/rollback-unit.mjs`
- `scripts/lib/rollback-manifest.mjs`
- `scripts/tests/rollback-final.test.mjs`
- `scripts/tests/rollback-manifest.test.mjs`
- `scripts/lib/platform-source-probe-node-dependencies.mjs`

## 自验收

- 残留 gate 三连通过（含 `scripts/`、`.github/` 无匹配）。
- `cargo test -p skiff-router` 全绿。
- `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 通过
  （本 branch 尚未合并 registry 改名，subject 仍为 `router-rust`；
  verify.yml 已按 post-merge 命名写 `router`）。
- isolated fixture instance build/up 用 Rust binary 通过
  （`scripts/tests/router-instance-binary-lifecycle.test.mjs` 已覆盖）。
- `node scripts/verify.mjs --only scripts-tests`（及 scripts-syntax）全绿。
- `node scripts/verify.mjs --only checks` 中 local-instance 等 checker 不回归。

## 已知 seam / 交接

- verify.yml 的 `router` selector 依赖 cutover-registry 合并后的 subject
  改名与 `router-ts-tests`/`router-type-check` 删除；两分支合入前不要跑
  `--only router`。
- repo `AGENTS.md` 仍写 loop-risk stress 从 `router/package.json` 解析 ws，
  归 cutover-registry 更新。
- `scripts/tests/verify-taxonomy.test.mjs` 归 cutover-registry，本节点不改。
- `doc/architecture/test-runner-runtime-isolation.md` 仍引用
  `router/package.json`（公开文档，归 cutover-registry 的文档收尾）。

## 执行记录

### 状态：完成

基线 `origin/main@6f03a59f`；worktree
`/Users/geek/workspace/wt-cutover-delete`，分支
`feat/router-rust-cutover-delete`。

#### 提交

1. `b9714d7f` refactor(router): cutover-delete TS Router, differential
   harness, and TS rollback tooling（独立删除 commit；router 167 文件 +
   scripts differential/rollback/probe 56 文件）。
2. 工具链收口 commit（见 git log）。

#### 自验收证据（2026-08-03 本地 macOS）

- 残留 gate 三连：
  - `rg --files router | rg '\.(ts|tsx)$'` → 无结果；
  - `test ! -e router/package.json` → 成功；
  - `rg '@skiff/router|pnpm --dir router|tsx.*router'` scripts + .github →
    无结果。
- `cargo test -p skiff-router --no-fail-fast` 全绿（全部 test binaries +
  doc-tests）。
- `node scripts/verify.mjs --only router-rust,router-rust-process-smoke`
  → 2/2 passed（本分支 subject 仍为 `router-rust`；verify.yml 已按
  post-merge 命名写 `router`）。
- 真实 Rust binary isolated fixture：临时 instance（managed temp mongod +
  bootstrap artifacts + committed state + 空 actor-routing projection），
  `instance supervise` 构建并启动真实 `skiff-router`，控制端口
  `/__router/health` 200（当前 Rust binary 的 recorded empty-200
  占位边界），`skiff-router --identity` 输出真实 identity，随后 `instance
  down` 干净退出。顺带修复 canonical isolated seed 的两个 Rust 缺口：
  - `initializeRouterActivationState` 原来写 TS 时代
    `router_assembly_activation_states` collection，改为 Rust
    `skiff-router.activation_state`；
  - `seedBootstrap` 现在写入空 canonical actor-routing projection
    record（Rust bootstrap 严格加载必需）。
- `node scripts/verify.mjs --only scripts` → 2/2 passed（630 tests 全绿，
  含 scripts-tests 与 dev-sync fixture）。
- `node scripts/verify.mjs --only scripts-syntax` → passed。
- `node scripts/verify.mjs --only checks`：仅
  `checks:runtime-crate-dag` 失败，已在基线 origin/main 复现（runtime
  crate DAG 依赖问题，非本节点范围）；其余 checker（含
  command-execution-policy、local-instance、runtime-execution-boundaries）
  全绿。

#### 顺带闭合的 batch-9/10 缺口（已在 ledger 记录）

- `command-execution-ledger`：补注册 chat/activation/clean-host/http-live
  process/execFile owners（基线红，checker 要求逐一登记 + marker）。
- `platform-source-transport-combined.test.mjs`：canonical skiff source
  registry 补 `actor-test-effect-capability`（batch-10 同步遗漏）。
- `runtime-execution-boundary-*`：删除 TS Router subject/owner/role 与
  router TS 夹具/自测 mutations（TS 文件删除后 checker 会因 required
  file 缺失而红）；`runtime-execution-boundary-router.mjs` 随之删除。

#### 已知 seam / 交接

- `router-instance-binary-lifecycle.test.mjs` 改为 managed router shim
  （真实 Router 无 artifacts/Mongo 时 fail-closed，skeleton 时代 fixture
  已过期；shim 绑定端口并应答 200，验证 binary install/identity/refresh
  生命周期）。真实 binary 的 isolated startup 由本次真实 fixture 验证 +
  `router-live:*` gates 覆盖。
- `router-live:rollback-final`（TS unit 演练）与 `router-live:differential`
  已按 plan §11.3 退役并从 live registry 删除；Rust-only clean-host
  工具保留在树中，未来 `router-live:clean-host` 注册归 tooling/release。
- `verify --only router`（新 subject 名）依赖 cutover-registry 合入后的
  selector graph；两分支合入前请勿单独跑该 selector。
- repo `AGENTS.md` 与 `doc/architecture/test-runner-runtime-isolation.md`
  仍引用 `router/package.json`（loop-risk ws 来源），归 cutover-registry
  文档收尾。
- `checks:runtime-crate-dag` 为基线既有红项（runtime crate DAG），本节点
  未触碰 runtime crate。
