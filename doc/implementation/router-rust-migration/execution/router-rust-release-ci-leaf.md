# Router Rust Migration Batch 12 — Release CI Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_release_ci`
集成目标：`/root/router_rust_integration_b12`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-12.md`
  （release workflow 节点；baseline `origin/main@ea8616bc`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，
  已 complete）：§8 `router-clean-host-live` / `router-live:clean-host`
  （scheduled + RC；Linux binary/PM2，无 pnpm/tsx）、§11.1 binary lifecycle
  （clean Linux/PM2 gate 只提供 binary、config、artifacts，PATH 不含
  pnpm/tsx，完成 start/health/Runtime reconnect/unary/shutdown）、§11.2
  Incremental rollback rehearsal、§11.3 Hard cut（rollback = 上一完整
  release，不能从 workspace 重建 TS unit）。
- cutover 事实：`doc/implementation/router-rust-migration/execution/router-rust-migration-b11-cutover-delete-leaf.md`
  （`router-live:rollback-final` 已退役；`clean-host-bundle.mjs` /
  `rollback-clean-host-suite.mjs` 保留，未来 `router-live:clean-host`
  注册归 tooling/release——即本 leaf）。
- 父节点裁决（2026-08-03）：接受预检上报的
  `check-router-rollback-final.mjs` 缺失/退役事实，按选项 2 继续：
  "完整 rollback" 定义为 Rust-only rollback rehearsal，新增
  `scripts/check-router-clean-host-live.mjs` 并在
  `scripts/lib/verify-live-registry.mjs` 注册 `router-live:clean-host`。

## 零 worktree 只读预检结论（锚定 ea8616bc）

1. `git fetch origin` 后 `origin/main == ea8616bc`；不存在
   `feat/router-rust-release-ci` 分支或 `wt-release-ci` worktree。
2. `scripts/check-router-rollback-final.mjs` 在基线**不存在**：由
   cutover-delete `b9714d7f`（已合入）删除，属 §11.3 退役，不能重建；
   `scripts/check-loop-risk-health-live.mjs` 也不存在，canonical live
   selector `loop-risk-health-live` 的 source path 是
   `scripts/check-loop-risk-health.mjs`（存在）；
   `scripts/check-loop-risk-stress-live.mjs` 存在。
3. Rust-only clean-host 工具保留在基线：
   `scripts/lib/clean-host-bundle.mjs`（bundle builder/verifier、clean env、
   PATH 断言、sh start 脚本）、`scripts/lib/rollback-clean-host-suite.mjs`
   （unary 就绪轮询 + HTTP 五 case）、`scripts/lib/rollback-relay.mjs`
   （`router-live:http` 使用）。
4. 可复用 fixture/harness 均在基线：
   `http_live_fixture.mjs`（真实 HTTP service source + compiler
   artifact + committed state）、`http_live_process.mjs`（binary install、
   logged spawn、listener wait、SIGTERM/SIGINT exit、port-closed）、
   `activation-state-live-harness.mjs`（临时 Mongo replica set）、
   `local-port-lease.mjs`（45000-45999）、`command-execution.mjs`
   （captureCheckedCommand，已注册 child-process owner）。
5. loop-risk canonical config schema 已确认：顶层仅
   `healthUrl`/`runtimeIds`/可选 `stress`；`stress` 要求
   `wsUrl`/`runtimePids`/绝对 `runtimeLogs`；health URL 必须精确指向
   `/__router/health?detail=loop-risk`。`router-live:clean-host` 目标实例
   的 stress `wsUrl` 用 router control 端口 `/runtime`（匿名 WS upgrade
   101，无 selector 依赖），`runtimePids`/`runtimeLogs` 取自 held Runtime
   子进程。

## 任务目标（父节点裁决后范围）

1. 新增 `scripts/check-router-clean-host-live.mjs`（Rust-only clean-host
   编排器，复用上述保留工具）：
   - 默认模式：临时 Mongo + 真实 compiler artifact + cargo 构建
     `skiff-router`/`runtime` debug binary → `buildCleanHostBundle`
     （bin/config/artifacts/start-*.sh/bundle-manifest）→
     `cleanHostEnv`（PATH=/usr/bin:/bin:/usr/sbin:/sbin，删
     PNPM_HOME/NVM_DIR/代理等）→ `assertNoPnpmOrTsxOnPath` →
     `/bin/sh start-router.sh` 启动 → control `/__router/health` 200 →
     `/bin/sh start-runtime.sh` → unary 就绪轮询证明 Runtime reconnect →
     HTTP 五 case → Router SIGTERM/Runtime SIGINT exit 0 → 端口关闭 →
     `assertCleanHostBundle`（运行前后哈希不变）。
   - `--loop-risk-config <path>` + `--loop-risk-stop-file <path>` hold
     模式：先跑完整 rehearsal，再以同一 bundle 启动 held target，写
     canonical loop-risk JSON，轮询 stop 文件后按同语义 teardown；供
     release workflow 的 loop-risk job 在实例存活期间显式调用两个
     `check-loop-risk-*.mjs`。
   - `--preflight`：断言脚本/库存在 + `node`/`cargo`/`mongod`/`mongosh`
     版本探测（CI 环境预检）。
   - 平台语义：Linux CI 是唯一真实 gate；macOS 本地运行只作 dry-run
     证据并在输出/文档标注 `platform`，不冒充 Linux gate。
2. `scripts/lib/verify-live-registry.mjs` 注册
   `router-rust-clean-host-live` → selector `router-live:clean-host`
   （FIXED_COMMAND，managed，live/manual，requiredInputs []，
   requiredExecutables node/cargo/mongod/mongosh，forbidSkips true）。
3. `scripts/tests/verify-live-registry.test.mjs` 对应行：LIVE_SELECTORS
   列表加 `router-live:clean-host`；clean-host invocation 断言；catalog
   checker path 循环加 `scripts/check-router-clean-host-live.mjs`。
4. 新增 `.github/workflows/router-rust-release.yml`
   （schedule + workflow_dispatch）：
   - clean-host job（ubuntu-latest；checkout/setup-node 24/rustup
     stable/apt MongoDB；`test -f` 预检 + `--preflight` + 全量执行；
     不安装 pnpm）。
   - loop-risk job（同基础 + pnpm 安装 scripts 依赖供 `ws`；后台 hold
     target 生成 canonical config；显式
     `check-loop-risk-health.mjs --config` 与
     `check-loop-risk-stress-live.mjs --config`；单 shell 内 trap
     保证 teardown 与退出码）。

## 写入边界

可写：

- `scripts/check-router-clean-host-live.mjs`（新）
- `scripts/lib/verify-live-registry.mjs`（仅 clean-host 条目）
- `scripts/tests/verify-live-registry.test.mjs`（对应行）
- `.github/workflows/router-rust-release.yml`（新）
- `doc/implementation/router-rust-migration/execution/router-rust-release-ci-leaf.md`（本文件）

禁止：

- `router/src`、runtime crate、`runtime/transport/src`、deployment、router
  TS、AGENTS.md、scripts README、其余 verify 文件（catalog/plan/selector
  graph/verify.mjs）、`skiff-instance.mjs`、
  `router-rust-integration.yml`（除非确需 append 并先上报）；
- 不操作 stable instance / stable Mongo / PM2 / 4004-4007；不跑全量
  verify；不为本脚本新增 child_process ledger owner（脚本通过
  `captureCheckedCommand` / `spawnLoggedProcess` / `ActivationStateMongoHarness`
  / `assertNoPnpmOrTsxOnPath` 执行子进程，均已有 owner）。

## 设计决策

1. Rust-only rollback rehearsal 即 clean-host rehearsal（§11.1 语义），
   不再重建 immutable TS unit（§11.3/b11 退役）。
2. 脚本自身不 import `node:child_process`：clean-host 子进程经
   `spawnLoggedProcess`（http_live_process，ledger 已注册）启动；其
   env 固定取 `process.env`，故脚本在 spawn 前把 `process.env` 收敛为
   `cleanHostEnv(...)` 的结果、teardown 后恢复，实现 PATH 净化。
3. loop-risk held target 复用同一 bundle 与同一 fixture，避免 workflow
   复制编排逻辑；stress `wsUrl` 指向 control 端口 `/runtime`（匿名
   101 upgrade，正是 loop-risk close-storm 的语义目标）。
4. `--preflight` 通过 `captureCheckedCommand` 探测工具版本，不新增
   child_process 使用面。
5. workflow 两个 job 并行；loop-risk job 在单一 `run` 块内用
   `trap cleanup EXIT` 保证 hold 进程一定被 stop 并回收退出码。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 脚本语法 | `node --check scripts/check-router-clean-host-live.mjs` |
| 环境预检 | `node scripts/check-router-clean-host-live.mjs --preflight`（macOS dry-run） |
| macOS dry-run rehearsal | `CARGO_PROFILE_DEV_DEBUG=0 node scripts/check-router-clean-host-live.mjs` 全流程 PASS（注明 platform=darwin，非 Linux gate） |
| hold 模式 | `--loop-risk-config <abs> --loop-risk-stop-file <abs>` 写出 canonical JSON → touch stop → teardown PASS |
| registry 测试 | `node --test scripts/tests/verify-live-registry.test.mjs` 全绿 |
| `--list` 展开 | `node scripts/verify.mjs --only router-live:clean-host --list` 含 selector；loop-risk 两个 selector `--list` 展开 |
| YAML 解析 | Python yaml.safe_load(`.github/workflows/router-rust-release.yml`) |
| 写集干净 | `git status` 仅本 leaf 声明文件 |

## 交接

完成后提交到 `feat/router-rust-release-ci`（不 push），直接向
`/root/router_rust_integration_b12` 报告 branch、worktree、commit、实际
写集、自验收矩阵与已知 seam；同步通知 root。

## 执行结果（提交前填写）

### 状态：完成

基线：`origin/main@ea8616bc`；worktree
`/Users/geek/workspace/wt-release-ci`，分支
`feat/router-rust-release-ci`。

#### 自验收证据（2026-08-03 本地 macOS）

- `node --check scripts/check-router-clean-host-live.mjs` → passed。
- `node scripts/check-router-clean-host-live.mjs --preflight` → ok：
  node v22.17.0 / cargo 1.88.0 / mongod v8.2.5 / mongosh 2.7.0，
  platform darwin。
- macOS dry-run rehearsal（`CARGO_PROFILE_DEV_DEBUG=0`，
  CARGO_TARGET_DIR=worktree/build/cargo-target）→ **PASS**：
  bundle 69 文件，pathProbe ABSENT（clean-host PATH 无 pnpm/tsx），
  HTTP 五 case 201/200/400/404/206，Router SIGTERM exit 0，
  Runtime SIGINT exit 0，端口关闭，`assertCleanHostBundle` 前后哈希
  不变。platform=darwin，明确为 dry-run，不代表 Linux gate。
- hold 模式（`--loop-risk-config` + `--loop-risk-stop-file`）→
  **PASS**：rehearsal 后二次启动同一 bundle，unary 就绪轮询证明
  Runtime reconnect，写出 canonical loop-risk JSON
  （healthUrl `?detail=loop-risk`、runtimeIds、stress.wsUrl=`/runtime`、
  runtimePids、绝对 runtimeLogs），stop 文件触发后 Router SIGTERM /
  Runtime SIGINT 均 exit 0，端口关闭，bundle 哈希不变。
  - 附加边界记录：held target 上显式调用
    `check-loop-risk-health.mjs --config <canonical>`，消耗同一
    config 并如实返回 baseline 边界失败
    （`Unexpected end of JSON input`：Rust `/__router/health` 当前
    仍是空 200 占位，无 loopRisk detail）；该投影由并行 health 节点
    合入后提供，本节点不改 consumer 契约。
- registry 测试：`node --test scripts/tests/verify-live-registry.test.mjs`
  → 20/20 全绿（先 `pnpm --dir scripts install --frozen-lockfile`
  安装 `ws`；未安装时基线同样有 2 个 pre-existing 失败，
  与本次改动无关，已在临时 baseline worktree 复现）。
- `node scripts/verify.mjs --only router-live:clean-host --list` →
  `live:router-rust-clean-host | live/manual | node
  scripts/check-router-clean-host-live.mjs`。
- `node scripts/verify.mjs --only loop-risk-health-live,
  loop-risk-stress-live --loop-risk-config <canonical> --list` → 两个
  selector 均展开为真实命令 + 显式 `--config`。
- YAML：`yaml`（1.2）parse `.github/workflows/router-rust-release.yml`
  ok，jobs=clean-host,loop-risk，cron=`0 4 * * 1`。
- `node scripts/check-command-execution-policy.mjs` → ok（新脚本未新增
  child_process import，无需 ledger 变更）。

#### 写集

- `.github/workflows/router-rust-release.yml`（新）
- `scripts/check-router-clean-host-live.mjs`（新）
- `scripts/lib/verify-live-registry.mjs`（仅 clean-host 条目）
- `scripts/tests/verify-live-registry.test.mjs`（对应行）
- `doc/implementation/router-rust-migration/execution/router-rust-release-ci-leaf.md`（本文件）

未触碰：router/src、runtime、deployment、router TS、AGENTS.md、scripts
README、其余 verify 文件、skiff-instance.mjs、router-rust-integration.yml；
stable instance / stable Mongo / PM2 / 4004-4007 未操作；未跑全量 verify。

#### 已知 seam / 交接

- `router-live:clean-host` 已注册为 managed live/manual selector；默认
  verify 不展开（与其余 live selector 一致）。release workflow 直接调用
  脚本，不依赖 selector 展开。
- loop-risk job 的 held target 由同一 clean-host 编排器托管；health
  gate 依赖并行 health 节点合入后的 `?detail=loop-risk` 投影，合入前
  workflow 若手工触发会在 health/stress 处如实失败。
- 平台契约：Linux GitHub runner 是唯一真实 clean-host gate；本地 macOS
  结果仅作 dry-run 证据。
