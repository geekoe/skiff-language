# Router Rust Migration Batch 6 — E-bootstrap Gate Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_e_bootstrap_gate`
集成目标：`/root/router_rust_integration_b6`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-6.md`
  （E-bootstrap gate 节点；baseline `main@8cabf352`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.3（`ActiveRoutingEpochStore` 单一 authority、atomic `Arc` replacement）、
  §3.8（boundedness）、§5.4（C-bootstrap → W-bootstrap → E-bootstrap）、
  §7 E-bootstrap（committed 只读、pending fail closed、missing/malformed/
  identity mismatch、loader saturation、shutdown 全部 fail closed/归零）、
  §8 `router-rust-bootstrap-live` / `router-live:bootstrap`（E-bootstrap slice 起
  required managed CI；compiler artifact、committed reader、initial epoch）、
  §8 CI 条款（所有 `pull_request` + `workflow_dispatch` 触发；cheap change
  classifier；非相关 PR 显式成功；相关 PR 跑 managed bootstrap job，临时 instance
  + 显式 Rust process；禁止 workflow 级 `paths` 导致 required check 缺失）。
- 冻结契约：`doc/implementation/router-rust-migration-c-bootstrap-contract.md`、
  `doc/implementation/router-rust-migration-c-router-activation-state.md`。
- 兄弟交付（已合入 main@8cabf352）：
  - W-activation-state：`router/src/activation/` 的 `ActivationStateRepository`
    read 面 + `MongoActivationStateRepository` / `MemoryActivationStateRepository`
    fake；
  - W-bootstrap：`router/src/bootstrap/` 的
    `CommittedActivationBootstrapReader` / `BootstrapStrictLoader` /
    `BlockingLoader` / `ActiveRoutingEpochStore` / `BootstrapRunner`；
  - W-session：`SessionLayerOptions.epoch_store` seam +
    `SessionLayer::attach_epoch_store` / `current_tuple()`；
  - A3：`ActorRoutingProjectionStore` / `ActorRoutingCatalog` /
    `ActorRoutingProjectionRef`（record path typed seam）。

冲突时以权威设计为准；本叶子只记录 E-bootstrap 装配与 live gate 的实现决策，
不改变冻结契约语义。

## 零 worktree 只读预检结论（锚定 main@8cabf352）

1. 基线：`git rev-parse main` = `8cabf35289e87a610c0940b6aa10af3a0e67d64e`；
   主 worktree 位于 `integration/router-rust-migration-batch-6`
   （`23ddab00`，仅比 main 多批次父文档）；兄弟 worktree
   `wt-w-dispatch` / `wt-w-model-request` 与本节点写集无重叠。
2. `run_router(config)` 目前在 `router/src/listener.rs`：直接
   `start_listeners`（内部新建 `SessionLayer::new`）→ 等 SIGINT/SIGTERM /
   fail-stop → shutdown；没有任何 bootstrap。`router/src/main.rs` 只负责
   config 解析与错误打印，本节点不需要改 main.rs 主流程。
3. W-bootstrap 交付齐全：`BootstrapRunner::run_initial(environment,
   actor_projection)` 完成 read → project → strict load → publish；
   `BootstrapReadOutcome` 五个 durable-state outcome + `FailClosedRepository`；
   `BlockingLoader` 支持 saturated/deadline/shutdown/drain；
   `SessionLayer::attach_epoch_store(Arc<ActiveRoutingEpochStore>)` 是唯一
   epoch source seam。
4. `ActivationStateRepository` read 面：`read(environment)`；缺失 →
   `CasMismatch`，malformed → `InvalidRecord`，基础设施失败 → `Transient` /
   `Closed`；`MongoActivationStateRepository::connect(url, options, clock)`
   需要 `ActivationClock`（`SystemClock`）。state 文档形状
   `{_id: environment, state: <EnvironmentActivationState>}`（canonical
   camelCase DTO）。
5. `ActorRoutingProjectionRef` 的 canonical 记录身份/路径推导仍未冻结
   （A1 producer 输出面未接入 compiler 流水线；A3 D2 seam 由 integration
   合流时替换）。本节点是 batch 6 的 integration 合流点，因此把相对记录路径
   定义为 E-bootstrap 装配常量
   `records/actor-routing/current.json`，并写文档说明这是 A1 合流前的显式
   typed seam（不猜 RuntimeAssembly 字段、不改 config 冻结契约）。
6. 真实 compiler artifact 路径可用：`skiff package build <root>
   --artifact-root <dir> --environment <env>` 经真实 `skiff-compiler` 产出
   PackageArtifact；`skiff assembly build --artifact-root <dir> --environment
   <env>`（无 root deployment）产出 RuntimeAssembly 记录；
   `runConfigSnapshotAuthoring`（`config-snapshot-tooling`）可产出真实
   RuntimeConfigSnapshot 记录。三者均有既有 Node helper 可直接调用。
7. live harness 基建：`scripts/lib/activation-state-live-harness.mjs` 提供临时
   mongod replica set（45000-45999 租约端口 + mktemp dbPath + rs.initiate +
   清理/端口回收断言）；`scripts/lib/local-port-lease.mjs` 提供端口租约；
   `scripts/lib/command-execution.mjs` 提供 checked command；
   `scripts/lib/cargo-target-dir.mjs` 提供 workspace target。
8. `verify-live-registry.mjs` 的 `FIXED_COMMAND` script source 形态可直接注册
   新 managed 条目（`router-rust-bootstrap-live` /
   `router-live:bootstrap`）；`verify-live-plan.mjs` 无需改动。
9. `router/tests/process_listener.rs` 现有二进制探针 config 无 `environment`，
   E-bootstrap 装配后 `run_router` 将 fail closed——本节点把它更新为
   fail-closed 负例（无 environment → 不绑定 listener、非零退出），真实
   二进制成功路径由 live probe 覆盖。
10. 任务可闭合；无需改公共契约（config schema、activation DTO、wire shape、
    A0/A3 schema 均不动）。不返回 `TASK_SCOPE_EXPANDED` /
    `TASK_NOT_EXECUTABLE`。

## 实现决策（在冻结契约语义内）

1. **生产装配 `RouterBootstrapAssembly`**（`router/src/bootstrap/assembly.rs`）：
   config 必须显式携带 `environment`（缺失 fail closed）；按顺序
   `MongoActivationStateRepository::connect(config.serviceDb.mongoUrl)`
   → `CanonicalCommittedRefValidator::open(artifacts_path)` →
   `BlockingLoader`（默认选项）→ `CommittedActivationBootstrapReader` →
   `BootstrapStrictLoader::open(artifacts_path, artifacts_path)` →
   `ActiveRoutingEpochStore` → `BootstrapRunner::run_initial`。committed
   epoch 发布后才返回装配成功；pending/missing/malformed/identity mismatch/
   loader 失败一律 fail closed 且不启动 listener。装配失败时关闭 repository。
   装配体持有 `epoch_store` / `loader` / `repository`，`shutdown()` 先 drain
   loader 再 close repository。
2. **actor projection typed seam**：装配使用
   `ActorRoutingProjectionRef::new(ArtifactRelativePath::new(
   ACTOR_ROUTING_PROJECTION_RECORD_PATH, ...))`，
   `ACTOR_ROUTING_PROJECTION_RECORD_PATH =
   "records/actor-routing/current.json"`；strict loader 校验链不变，记录
   不存在/非 canonical 同样 fail closed。该路径是 A1 producer 合流前的
   显式 seam（记录由 live harness/probe 物化）。
3. **listener wiring**（`router/src/listener.rs`，仅 E-bootstrap wiring）：
   `run_router` 先 `RouterBootstrapAssembly::assemble`，成功后构造
   `SessionLayer` 并 `attach_epoch_store`，再
   `start_listeners_with_session`；shutdown 顺序 = listener/session shutdown
   → assembly shutdown（loader drain + repository close）。`start_listeners`
   / `start_listeners_with_session` 的既有测试 seam 不变（无 store 时保持
   W-session 回退语义）。
4. **readiness/admission**：committed epoch 发布是 listener 启动前置条件；
   admission（请求/会话 admission）按计划在 E-session 后才开放，本节点只接
   epoch source，不实现 admission gate。
5. **live probe**（`router/tests/bootstrap_live_probe.rs`，`#[ignore]`，由
   harness 注入 env）：真实 compiler assembly 记录 + 真实 snapshot store +
   真实 actor projection 记录 + 临时 Mongo repository；成功链
   `run_initial` 发布 epoch 并启动真实 `skiff-router` 二进制，WS 客户端收到
   `router.bootstrap` 帧并校验 activation 字段；负例矩阵（missing /
   malformed / pending / identity mismatch / snapshot missing）进程级与
   runner 级均 fail closed、零发布；loader saturation（并发 1 持 permit +
   第二次 load → `Saturated`）与 shutdown（拒绝 + drain 归零）fail closed。
6. **live harness**（`scripts/check-router-bootstrap-live.mjs`，`check-*` 前缀是
   verify live catalog 对已注册 script source 的既有发现约定）：复用
   `ActivationStateMongoHarness` 起临时 replica set；真实 compiler
   （`skiff package build` + `skiff assembly build`）产出 artifact；
   `runConfigSnapshotAuthoring` 产出 snapshot；显式
   `cargo build -p skiff-router --bin skiff-router` 构建 Rust 二进制；运行
   live probe；finally 清理 mongod/临时目录/端口租约并断言端口关闭。
   不触碰 stable instance / Mongo（27017）/ PM2 / 4004-4007。
7. **verify live registry**：`scripts/lib/verify-live-registry.mjs` 增加
   `router-rust-bootstrap-live` 条目（script source + `FIXED_COMMAND`，
   `router-live:bootstrap`，`MANAGED` / `live/manual`，requiredExecutables
   `node/cargo/mongod/mongosh`，`forbidUnchecked: true`）。
8. **CI workflow**（`.github/workflows/router-rust-integration.yml`，新文件）：
   `on: pull_request + workflow_dispatch`（无 workflow 级 `paths`）；第一个
   job 是 cheap change classifier（`gh api` 列出 PR changed files，非相关
   PR 输出 `related=false`；`workflow_dispatch` 恒 related）；managed job
   名固定 `Router Rust Bootstrap (managed)`，`if: always()`，非相关时显式
   skip-success，相关时安装临时 mongod/mongosh + Rust + Node，运行
   `node scripts/check-router-bootstrap-live.mjs`（临时 instance + 显式 Rust
   process）；gate 成熟后由集成 Agent 把该稳定 job 名加入 required checks。

## 写集

生产 / 装配（仅本叶子）：

- `router/src/bootstrap/assembly.rs`（新，E-bootstrap 生产装配）；
- `router/src/bootstrap/mod.rs`（仅 additive `mod assembly;` + re-export）；
- `router/src/listener.rs`（仅 `run_router` E-bootstrap wiring + assembly
  shutdown）；
- `router/tests/process_listener.rs`（更新为无 environment fail-closed 负例；
  listener 机制测试仍由 `start_listeners*` seam 与 live probe 覆盖）；
- `router/tests/bootstrap_live_probe.rs`（新，`#[ignore]` live probe）；
- `scripts/lib/verify-live-registry.mjs`（仅 bootstrap 条目）；
- `scripts/tests/verify-live-registry.test.mjs`（仅 `LIVE_SELECTORS` 期望列表
  与注册条目原子配套的最小更新，避免 tooling gate 回归）；
- `scripts/check-router-bootstrap-live.mjs`（新 live harness，`check-*` 前缀）；
- `.github/workflows/router-rust-integration.yml`（新）；
- `doc/implementation/router-rust-e-bootstrap-gate-leaf.md`（本文件）。

禁止写：`router/src/http`、`router/src/dispatch`、`router/src/routing`、
router TS、`runtime` crate、`runtime/transport/src`、deployment、AGENTS.md、
scripts README、verify selector graph、`skiff-instance.mjs`；不操作 stable
instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 装配单测 | `cargo test -p skiff-router bootstrap_production_wiring`（注入 memory repository + 真实 artifact root：成功发布 + 各 fail-closed 负例 + shutdown） |
| live 成功链 | `node scripts/check-router-bootstrap-live.mjs`：真实 compiler artifact → committed reader → initial epoch，真实二进制 WS `router.bootstrap` 帧字段校验 |
| live fail-closed 负例 | 同 harness：missing/malformed/pending/identity mismatch/snapshot missing 进程与 runner 级零发布；loader saturated/shutdown 归零 |
| verify 注册表 | `node scripts/verify.mjs --list` 含 `router-rust-bootstrap-live` / `router-live:bootstrap` |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 通过 |
| workflow | `.github/workflows/router-rust-integration.yml` YAML 可解析；job 名稳定；无 workflow 级 `paths` |
| 格式 / clippy | 触碰 Rust 文件 `cargo fmt --check`；`cargo clippy -p skiff-router --all-targets` 无新增 error |
| Batch 5 Verify 观测 | `gh run list` / `gh run view 30749913464 --repo geekoe/skiff-language` 报告结论；失败为存量环境问题则标注不阻塞 |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后提交到 `feat/router-rust-e-bootstrap-gate`（不 push），直接向
`/root/router_rust_integration_b6` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知 seam（actor projection record path 为 A1 合流前
显式 seam；`process_listener` 语义更新），并通知 root。

## 执行结果（提交前自验收填写）

（2026-08-02 提交前填写，全部通过）

1. 生产装配 + wiring 单测：
   `cargo test -p skiff-router --test bootstrap_production_wiring` 7 passed
   （成功发布 + SessionLayer epoch source + missing/malformed/pending/identity
   mismatch/无 environment fail closed + 路径 seam）。
2. 全 crate 回归：`cargo test -p skiff-router --no-fail-fast` 154 passed /
   0 failed（1 ignored = `activation_mongo_probe` live，`bootstrap_live_probe`
   ignored 由 harness 驱动）。
3. `router-live:bootstrap` live harness：
   `node scripts/check-router-bootstrap-live.mjs` 通过：真实 compiler
   （`skiff package build` + `skiff assembly build`）产出 RuntimeAssembly +
   真实 config-snapshot-tooling 产出 snapshot（`<root>/runtime-config`）；
   临时 mongod replica set（45000-45999 租约端口）；显式
   `cargo build -p skiff-router --bin skiff-router`；真实二进制
   `router.bootstrap` WS 帧字段校验（environment/generation/assembly/
   configSnapshot/mongoUrl/artifactsPath）；missing/malformed/pending/identity
   mismatch/snapshot missing 在 runner 与进程两级全部 fail closed 零发布；
   loader saturation（并发 1 持 permit + 第二次 load → `Saturated`）与
   shutdown（拒绝 + occupancy/queued 归零）通过；mongod/临时目录/端口租约
   全部清理。
4. verify 注册表：`node scripts/verify.mjs --only router-live:bootstrap --list`
   展开 `live:router-rust-bootstrap`；
   `scripts/tests/verify-live-registry.test.mjs` 的 registry contract 两项
   （single declaration / help）通过；`--help` 含新 selector。该测试文件仅做
   与注册条目原子配套的最小 `LIVE_SELECTORS` 更新（其余 2 项失败为 worktree
   未装 `ws` module 的存量依赖条件，与本次条目无关）。
5. 聚焦 verify：`node scripts/verify.mjs --only router-rust,router-rust-process-smoke`
   2/2 passed（`router-rust:contracts` + `router-rust:process-smoke`）。
6. workflow：`.github/workflows/router-rust-integration.yml` 经 `yaml` 包解析
   通过；`pull_request` + `workflow_dispatch` 双触发；无 workflow 级 `paths`；
   cheap change classifier 先跑；required job 名稳定为
   `Router Rust Bootstrap (managed)`，`if: always()` 对非相关 PR 显式成功。
7. rustfmt/clippy：`cargo fmt -p skiff-router -- --check` 通过；
   `cargo clippy -p skiff-router --all-targets` 无 skiff-router 新 warning/
   error（其余 crate 为既有 baseline warning）。
8. Batch 5 Verify 观测（`gh run list` / `gh run view 30749913464
   --repo geekoe/skiff-language`）：run 已结束，结论 FAILED；
   `Router Rust Contracts and Process Smoke` job PASS（2m23s）——本批次相关
   lane 绿。Skiff Source Tests 失败为 isolated Mongo spawn 120s timeout
   （存量环境问题）；Quality and Checks 失败 4 项：rustfmt diff 为 batch 5
   既有代码格式差异（非本节点），file-lines/command-execution-policy/
   runtime-crate-dag 均为 `spawnSync rg ENOENT`（runner 无 ripgrep，存量
   环境问题）；Implementation Tests 失败为 runtime/eval lib 既有编译目标
   + test-runner isolated Mongo exit 127 + tooling rg/JSON 环境问题。
   全部标注存量/批次 5 既有，不阻塞本节点。
9. 写集：`git status` 仅含叶子写集 + `scripts/tests/verify-live-registry.test.mjs`
   最小配套更新；未触碰任何禁止目录；`git diff main...HEAD` 聚焦。
