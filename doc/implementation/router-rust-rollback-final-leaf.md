# Router Rust Migration Batch 10 — Rollback 终态 Gate Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_rollback_final`
集成目标：`/root/router_rust_integration_b10`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-10.md`
  （rollback 终态节点；baseline `origin/main@edc111f8`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`
  （draft v5），重点 §8 `router-live:clean-host` / `router-clean-host-live`
  （Linux binary/PM2，无 pnpm/tsx/router node_modules）与 §11.2
  Incremental rollback rehearsal（immutable TS unit 必须自包含 pinned Node
  runtime、最后 TS source、materialized Router dependencies 或 offline
  store + frozen install、package/lockfile、process spec、所有 file/source
  identity；最终演练 stop admission → shutdown → verify exit → start
  target → Runtime reconnect exact committed tuple → activation/readiness →
  open admission → HTTP/WS/actor smoke）。
- 兄弟 leaf：`doc/implementation/router-rust-e-http-gate-leaf.md`
  （E-http gate；已完成首次 TS→Rust→TS unary rollback roundtrip，本 leaf
  在其基础上扩展为 release-candidate 级：immutable TS unit + clean-host
  演练）。
- 现有 seam：`scripts/lib/rollback-manifest.mjs`（schema v1，仅冻结 process
  command）、`scripts/lib/dev-runtime-paths.mjs`（RouterProcessSpec /
  routerProcessInvocation）、`scripts/check-router-http-live.mjs` 与
  `scripts/lib/http_live_*`（真实 HTTP → Router → Runtime 编排、
  `runRollbackSuite` 五 case、relay/进程/端口归零断言模式）。

## 零 worktree 只读预检结论（锚定 edc111f8）

1. 基线：`git fetch origin` 后 `origin/main == edc111f8`（
   `edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`）；不存在
   `feat/router-rust-rollback-final` 分支或 `wt-rollback-final` worktree。
2. E-http harness 已在基线：`scripts/check-router-http-live.mjs`、
   `scripts/lib/http_live_fixture/process/suite/client.mjs`、
   `scripts/lib/router-differential/{relay,instance,mongo}.mjs`；
   `router-live:http` 三阶段 roundtrip（TS→Rust→TS）真实通过记录在
   `router-rust-e-http-gate-leaf.md`。
3. TS Router 可启动：`router/package.json` `dev = tsx src/router/server.ts`，
   worktree 内无 `router/node_modules`（被忽略）；本 leaf 通过既有
   `ensureTsRouterDependencies`（frozen lockfile）按需安装。
   实测发现 workspace `router/node_modules` 的 pnpm virtual store 内含
   **绝对**符号链接（指向 workspace 自身），且 `fs.cp`/`cp -RL`
   dereference 都会破坏 pnpm 的 `.pnpm/<pkg>/node_modules` sibling
   解析（tsx 找不到 esbuild）。因此 unit 依赖不在 workspace 安装，而是
   在 unit 内执行 `pnpm --dir <unit>/router install --frozen-lockfile
   --offline`（本地 store，构建期），随后把所有符号链接重写为 unit 内
   相对链接（150 个，绝对链接 0），`fs.cp` 符号链接绝对化问题用自定义
   copyTree（readlink + symlink 逐条重建）规避。
4. 无 ambient pnpm/tsx 的离线启动方式已确认：pinned Node runtime 直接执行
   `node <unit>/router/node_modules/tsx/dist/cli.mjs <unit>/router/src/router/server.ts
   --config <abs>`，cwd 为 `<unit>/router`；tsx CLI 内部用
   `process.execPath` 派生子进程，不需要 PATH 中的 pnpm/tsx。
5. pinned Node runtime 候选：`~/.nvm/versions/node/v22.17.0/bin/node`
   （官方发行版布局，`otool -L` 只依赖系统 framework，单二进制自包含，
   110MB），版本 v22.17.0；通过 `--node-runtime-dir` /
   `SKIFF_ROLLBACK_NODE_RUNTIME_DIR` 显式指定，禁止隐式 Homebrew Cellar
   拷贝（其 dylib 依赖不可 relocatable）。
6. 可观测性：TS Router 的 runtime/control listener
   `/__router/health` 返回 `activeAssembly`（environment/generation/
   assemblyIdentity/configSnapshotId）、`pendingActivation`、`replicas`
   （TS readiness 用）；Rust 侧 `/__router/health` 仍是空 200 占位
   （E-http leaf 已记录的差异边界），Rust 阶段用 relay bootstrap tuple +
   unary 套件作 readiness/reconnect 证明。
7. Runtime 侧 `runtime/host/src/host/lifecycle.rs` 有 250ms→5s 指数退避
   reconnect loop；同一 relay + 同一 Runtime 进程跨 Router 阶段存活时，
   Router 重启后 Runtime 会建立新 downstream 连接，relay 为其建立新
   upstream 到新 Router，可观测新握手。
8. 写入边界已确认（见下）；本任务可闭合，不返回 `TASK_SCOPE_EXPANDED` /
   `TASK_NOT_EXECUTABLE`。

## 任务目标

1. **immutable TS rollback unit builder 完成**：
   - target-platform pinned Node runtime（官方发行版目录，版本 +
     platform/arch + 二进制 SHA-256 入 manifest）；
   - 最后 TS source（`router/` 的 package.json / pnpm-lock.yaml /
     pnpm-workspace.yaml / tsconfig.json / `src/**`，随 unit 逐文件记录
     SHA-256）；
- materialized Router 依赖（unit 内 frozen 安装，全部符号链接重写为
    unit 内相对链接，不引用 workspace store）；
   - package/lockfile（随 source 提供并哈希）；
- process spec（unit 内 pinned node + tsx cli + server.ts + `--config`，
     路径相对 unit root 记录、启动时解析，不依赖 PATH 中的 pnpm/tsx）；
   - 所有 file/source identity（`files` 全量相对路径 → SHA-256 + 聚合
     tree digest + file count）。
2. **release-candidate 级演练**（扩展 E-http roundtrip 为最终形态）：
   stop admission → shutdown TS → verify PID/listener 退出 → 启动
   immutable TS unit（全新临时目录、离线、禁止复用 workspace
   router/node_modules 或网络）→ Runtime reconnect exact committed tuple →
   activation/readiness → open admission → HTTP unary smoke；同时记录
   TS→Rust→TS 双向过程切换命令（rollback manifest 已有基础，本 leaf 增加
   switch plan schema）。
3. **clean-host 准备**：binary + config + artifacts 部署包清单与启动脚本；
   PATH 故意不含 pnpm/tsx 的本地等价演练（真实 Linux/PM2 clean-host 归
   CI，不在本 leaf 实现）。

## 实现决策

### 1. Unit 布局（全新临时目录内构建）

```text
<unitRoot>/
  rollback-unit.json            # skiff-router-rollback-ts-unit-v1
  node-runtime/
    bin/node                    # pinned official Node 二进制
    LICENSE                     # 官方发行版许可（身份一并哈希）
  router/
    package.json
    pnpm-lock.yaml
    pnpm-workspace.yaml
    tsconfig.json
    src/**                      # 最后 TS source（含 src 内既有 .rs 文件）
    node_modules/**             # materialized（frozen 安装 + 相对链接）
```

- `rollback-unit.json` 不在 `files` 自映射内（自引用鸡生蛋）；校验时以
  manifest 中的全量 `files` + `sha256_tree` 重算比对，unit 内任何增删改
  都会失败。
- config 不拷入 unit：`config_path` 为部署侧绝对路径（与 v1 manifest
  一致），unit 只记录身份。
- 依赖策略采用“materialized”模式（任务允许的两种之一）：frozen lockfile
  在 unit 内安装（优先 `--offline` 本地 store，缺失时构建期网络回退），
  安装后移除 `.modules.yaml` 等嵌入构建机 store 路径的 pnpm 运行时元数据，
  并把全部符号链接重写为相对链接；manifest 记录
  `dependencies.install_command` / `install_offline`。

### 2. Manifest / switch plan 扩展

- `scripts/lib/rollback-manifest.mjs` 保留 v1 builder/validator 字节不变
  （既有 `scripts/tests/rollback-manifest.test.mjs` 必须仍绿），新增：
  - `ROUTER_ROLLBACK_UNIT_SCHEMA = 'skiff-router-rollback-ts-unit-v1'`：
    `buildTsRollbackUnitManifest` / `assertTsRollbackUnitManifest`。
  - `ROUTER_ROLLBACK_SWITCH_SCHEMA = 'skiff-router-rollback-switch-v1'`：
    `buildRouterRollbackSwitchPlan`（TS→Rust / Rust→TS 双向：stop
    SIGTERM + expect exit 0 + listeners closed；start 为对端 process
    command；Rust→TS 支持 unit 直接命令覆盖）/
    `assertRouterRollbackSwitchPlan`。
- unit manifest 内嵌 `switch_commands`（用 unit 直接命令做 Rust→TS）。
- `files` 全量身份同时覆盖普通文件（内容 SHA-256）与符号链接（目标串
  SHA-256）；`symlinks` map 记录每个链接的目标，校验器逐项断言目标为
  相对路径且不逃逸 unit。

### 3. Rehearsal 阶段（真实进程，单一 relay + 单一 Runtime 存活跨阶段）

```text
ts-workspace  →  ts-unit  →  rust  →  ts-unit-relocated
```

- 每阶段：wait listeners → relay 观测新握手（`router.bootstrap` … →
  `runtime.health`）→ 断言 `latestBootstrapTupleAfter` 与 committed tuple
  完全一致 → TS 阶段额外断言 `/__router/health`（activeAssembly tuple 一致、
  `pendingActivation: null`、replicas 含 runtime id）→ 打开 admission →
  `runRollbackSuite`（unary-happy / typed-unary / missing-selector /
  wrong-path / stream-roundtrip）。
- stop admission 模型：harness 侧 admission gate（无 in-flight、不再发新
  请求）并在 evidence 记录；无生产 admission toggle seam（与 E-http
  记录一致），不新增控制端点。
- 切换：SIGTERM 当前 Router → exit 0 → `httpPort`/`runtimePort` 关闭 →
  下阶段启动；relay 与 Runtime 进程保持存活，Runtime 经 relay 重连。
- 复用 `router-differential/relay.mjs` 会静默保留 Runtime 侧 downstream
  socket（Router 侧断开后只 detach 不 close），Runtime 永远不会重连；
  因此本 leaf 提供 `scripts/lib/rollback-relay.mjs`（rollback 边界内）：
  与生产一致，Router 侧 upstream 关闭时同时关闭 downstream，驱动
  Runtime 的 250ms→5s reconnect loop，使每个阶段产生新的可观测握手。
- `ts-unit-relocated` 阶段把整个 unit `cp -R` 到第二个全新临时目录并重新
  校验身份后启动，证明 unit 可迁移、不可变、自包含。
- 结束时 Runtime SIGINT exit 0、relay 关闭、全部端口关闭。
- clean-host 阶段不用 relay（部署真实拓扑直连）：以“真实 unary 就绪轮询
  （POST /unary 201）”证明 Runtime reconnect，再跑与 rollback 套件同语义
  的 HTTP-only 五 case（无 relay 帧断言），bundle 运行前后哈希不变。

### 4. Clean-host 演练（真实 binary + config + artifacts 包）

- `scripts/lib/clean-host-bundle.mjs`：组装
  `<bundle>/bin/{skiff-router,skiff-runtime}`、
  `<bundle>/config/{router.yml,runtime.yml}`、`<bundle>/artifacts/**`、
  `<bundle>/scripts/{start-router.sh,start-runtime.sh}`、
  `bundle-manifest.json`（schema v1，全量文件 SHA-256 + process command）；
  runtime-home 是部署侧 stateful 路径，放 bundle 外（与
  deploy-runtime-stack 拓扑一致）。
- 启动脚本：`#!/bin/sh` + `exec "$BUNDLE_ROOT/bin/skiff-router" ...`，
  不含 node/pnpm/tsx 引用。
- 本地演练：PATH 显式置为 `/usr/bin:/bin:/usr/sbin:/sbin`，先断言
  `command -v pnpm` / `command -v tsx` 均失败，再经脚本启动 Router +
  Runtime（真实 Rust binary），health 200、unary 套件 PASS、SIGTERM
  exit 0、端口关闭；bundle 在运行前后哈希不变（runtime-home 在外）。

## 写入边界

可写：

- `scripts/lib/rollback-manifest.mjs`（扩展，v1 不变）；
- `scripts/lib/rollback-unit.mjs`（新：unit builder / verifier / copy）；
- `scripts/lib/rollback-relay.mjs`（新：生产语义 relay，驱动持久 Runtime
  跨阶段重连）；
- `scripts/lib/clean-host-bundle.mjs`（新：bundle builder / verifier /
  clean-host env）；
- `scripts/check-router-rollback-final.mjs`（新：编排 + 证据）；
- `scripts/tests/rollback-final.test.mjs`（新：纯函数单测，不跑 live）；
- `doc/implementation/router-rust-rollback-final-leaf.md`（本文件）。

禁止：

- `router/src`、runtime crate、`runtime/transport/src`、deployment、router
  TS（unit 只消费既有 TS 产物，不写 TS 代码）、AGENTS.md、scripts README、
  verify 文件（`verify-live-registry*` / `verify.mjs` / selector graph /
  workflow YAML）、`skiff-instance.mjs`；
- 操作 stable instance / stable Mongo / PM2 / 4004-4007；不跑全量
  `pnpm verify`。

若发现 TS unit 无法离线启动（依赖未物化、pinned runtime 不自包含、
tsx 直接命令不可行），停下上报，不绕过验证。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 既有 v1 回归 | `node --test scripts/tests/rollback-manifest.test.mjs` 全绿 |
| 新 lib 单测 | `node --test scripts/tests/rollback-final.test.mjs` 全绿 |
| immutable unit | builder 产物在全新临时目录；`verifyImmutableTsRollbackUnit` 全量哈希一致、符号链接全部相对且不逃逸 unit；relocated 副本重新校验一致 |
| release-candidate 演练 | `node scripts/check-router-rollback-final.mjs` PASS：四阶段 bootstrap tuple 与 committed 完全一致；每阶段 unary suite 通过；TS 阶段 health readiness 断言通过；SIGTERM/SIGINT exit 0；端口关闭 |
| clean-host 演练 | 同一脚本 PASS：PATH 无 pnpm/tsx（`command -v` 失败断言）；经 `start-*.sh` 启动；unary PASS；bundle 哈希运行前后不变 |
| 写集干净 | `git status` 仅本 leaf 声明文件；未触碰禁止目录 |

## 交接

完成后提交到 `feat/router-rust-rollback-final`（不 push），直接向
`/root/router_rust_integration_b10` 报告 branch、worktree、commit、实际写集、
自验收矩阵与已知 seam；同步通知 root。

## 执行结果（提交前填写）

### 状态：完成（release-candidate 演练 + clean-host 演练全绿，真实证据在案）

基线：`origin/main@edc111f8`；worktree
`/Users/geek/workspace/wt-rollback-final`，分支
`feat/router-rust-rollback-final`。

#### 自验收证据（2026-08-03 本地 macOS；磁盘争用期间用
`CARGO_PROFILE_DEV_DEBUG=0` 控制 target 体积，脚本本身不硬编码）

`node scripts/check-router-rollback-final.mjs` → exit 0，输出完整 evidence：

- **immutable TS unit**：全新临时目录构建（pinned Node v22.17.0
  darwin/arm64，`node-runtime/bin/node` SHA-256 入 manifest）；2130 个
  payload 文件、150 个符号链接全部为 unit 内相对链接（绝对链接 0）；
  `router_source` 171 文件、`dependencies` 1955 文件（frozen install +
  `install_offline: true`）；`verifyImmutableTsRollbackUnit` 全量哈希一致；
  `cp` 到第二个全新临时目录后重新校验通过（relocatable）。
- **release-candidate 演练（四阶段，单一 relay + 单一 Runtime 跨阶段存活）**：
  ts-workspace → ts-unit → rust → ts-unit-relocated，每阶段：
  stop admission → SIGTERM Router exit 0 → http/runtime 端口关闭 →
  启动目标 → Runtime 经 relay 重连 → bootstrap tuple 与 committed
  （environment http-live / generation 1 / assembly identity / config
  snapshot id）完全一致 → TS 阶段 `/__router/health` readiness（tuple +
  `pendingActivation: null` + replicas 含 runtime id）→ open admission →
  unary suite 5/5（unary-happy 201、typed-unary 200、missing-selector
  400、wrong-path 404、stream-roundtrip 206）。四阶段 tuple 逐字段一致；
  Runtime SIGINT exit 0；relay 关闭、端口全部关闭。
- **clean-host 演练**：bundle 68 文件（bin/skiff-router、
  bin/skiff-runtime、config/router.yml、config/runtime.yml、
  artifacts/**、scripts/start-*.sh、bundle-manifest.json）；
  PATH=`/usr/bin:/bin:/usr/sbin:/sbin`，`command -v pnpm|tsx` 均 ABSENT；
  经 `sh scripts/start-*.sh` 启动；真实 unary 就绪轮询后 HTTP-only 五 case
  全过；Router SIGTERM / Runtime SIGINT 均 exit 0；端口关闭；bundle
  运行前后哈希不变（runtime-home 在 bundle 外）。
- 既有 v1 回归：`node --test scripts/tests/rollback-manifest.test.mjs
  scripts/tests/rollback-final.test.mjs` 9/9 PASS。

#### 已解决的阻塞与记录边界

1. pnpm virtual store 内含**绝对**符号链接（workspace 安装）且
   `fs.cp` 在 macOS 会把相对链接复制成绝对链接；unit 改为 unit 内
   frozen 安装（优先 `--offline`）+ 全部链接重写为相对 + 自定义
   copyTree（readlink+symlink 逐条重建），验证后绝对链接为 0。
2. `router-differential/relay.mjs` 在 Router 侧断开后保留 Runtime 侧
   socket，持久 Runtime 不会重连；本 leaf 新增
   `scripts/lib/rollback-relay.mjs`（生产语义：upstream 关闭时同时关闭
   downstream），驱动 250ms→5s reconnect loop。
3. Rust `/__router/health` 仍为空 200 占位（E-http leaf 已记录差异）；
   Rust 阶段 readiness 以 relay bootstrap tuple + unary suite 证明，
   TS 阶段额外断言 health JSON。
4. 生产无 admission toggle seam：stop/open admission 为 harness 侧 gate
   （无 in-flight、记录时间序），未新增控制端点。
5. clean-host 阶段不用 relay（真实直连拓扑）：以 POST /unary 201 轮询
   证明 Runtime reconnect，再跑 HTTP-only 五 case。
6. 共享磁盘在演练期间被并行节点占满（cargo target 峰值 122GB），多次
   `No space left on device`；清理本 worktree target +
   `CARGO_PROFILE_DEV_DEBUG=0` 后完成。stable instance / Mongo / PM2 /
   4004-4007 未触碰；未跑全量 verify。

#### 写集

- `scripts/lib/rollback-manifest.mjs`（扩展：unit schema v1 +
  switch plan schema v1；既有 v1 函数不变）
- `scripts/lib/rollback-unit.mjs`（新）
- `scripts/lib/rollback-relay.mjs`（新）
- `scripts/lib/rollback-clean-host-suite.mjs`（新：clean-host HTTP-only
  五 case 与 unary 就绪轮询）
- `scripts/lib/clean-host-bundle.mjs`（新）
- `scripts/check-router-rollback-final.mjs`（新）
- `scripts/tests/rollback-final.test.mjs`（新）
- `doc/implementation/router-rust-rollback-final-leaf.md`（本文件）

未触碰：router/src、runtime、deployment、router TS、AGENTS.md、scripts
README、verify 文件、skiff-instance.mjs、`.github/workflows`。

### 交接

见文首「交接」节：提交至 `feat/router-rust-rollback-final`（不 push），
直接向 `/root/router_rust_integration_b10` 报告并同步通知 root。
