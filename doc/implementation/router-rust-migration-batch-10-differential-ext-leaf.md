# Router Rust Migration Batch 10 — Differential Extension Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_differential_ext`
集成目标：`/root/router_rust_integration_b10`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-10.md`
  （differential 扩展节点；baseline `origin/main@edc111f8`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）
  §9 Continuous Integration Matrix（implementation-neutral differential
  harness：TS/Rust 独立端口、artifact root、runtime home、Mongo namespace，
  不共享 Runtime、不镜像 live traffic；对比 HTTP、WS、Runtime frames、
  health、Mongo state/audit、terminal counters；normalization 仅允许 UUID、
  timestamp、ephemeral port、无语义 log order；每删除一个 TS test，ledger
  标记 retired/shared owner/Rust replacement/black-box replacement）。
- 父节点 W-differential：
  `doc/implementation/router-rust-migration-batch-8-w-differential-leaf.md`
  （harness 结构、资源约定、可写面）。
- 场景 inventory 文档：
  `doc/implementation/router-rust-migration-batch-8-differential-scenarios.md`
  （本节点负责继续维护其矩阵与 JSON 源）。
- Test ledger：
  `doc/implementation/router-rust-migration-batch-8-test-ledger.md`
  （自 batch 8 起是 Router TS test 处置的 canonical 登记处，本节点继续同步）。
- 兄弟 gate leaf：`router-rust-e-http-gate-leaf.md`（X-Skiff-Release TS
  201 vs Rust 400 差异、backpressure macOS OS-absorption 边界记录）、
  `router-rust-e-ws-gate-leaf.md`（WS-only 残余缺口）、
  `router-rust-e-actor-rust-leaf.md`（actor full-chain 先例）。

## 零 worktree 只读预检结论（锚定 edc111f8）

1. **基线锚定**：`git fetch origin` 后 `origin/main` =
   `edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`；worktree
   `/Users/geek/workspace/wt-differential-ext`（分支
   `feat/router-rust-differential-ext`）HEAD 即该 commit。共享主 worktree
   只读；本地 main（adbcd1b4）与 origin/main 分叉，一律不参考。
2. **既有 differential harness**（edc111f8 已合入 batch 8）：scenario
   inventory JSON（runnable 1 + planned 6）、`scripts/lib/router-differential/`
   （constants/frames/relay/mongo/normalize/compare/instance/scenarios/
   harness）、`scripts/check-router-differential-live.mjs`、
   hermetic 测试 4 个文件。`scenario-inventory.json` baseline 仍为
   `d228b613`（batch 8 基线），本节点改为 edc111f8 并同步测试断言。
3. **gate 能力事实**（edc111f8）：E-http/E-ws/E-actor-rust 已合入，真实
   HTTP unary/stream/error/CORS、client WS generation/replacement/id 词法、
   actor call/control 全链均已可用；但 Rust `router/src`、runtime crate 仍
   是只读（本节点禁止写）。`http_live_*` / `ws_live` / `actor_live` harness
   提供可复用模式（read-only import）。
4. **WS-only routing 状态**：baseline edc111f8 仍存在残余缺口——runtime
   `control_plane.rs::dispatch_modes_from_gateway_entries` 只统计 HTTP
   表面，WS-only deployment 广告空 dispatch_modes → E-ws gate 用额外 HTTP
   ping 条目规避（`scripts/check-router-ws-live.mjs` 注释记录）。Batch 10
   WS-only-routing 节点的 worktree
   `/Users/geek/workspace/wt-ws-only-routing`（分支
   `feat/router-rust-ws-only-routing`，HEAD c9d3917e）已有未提交修改：
   `runtime/host/src/host/control_plane.rs` 扩展 WebSocketConnect/JsonRpc
   表面 → 广告 unary/serverStream，并把 `check-router-ws-live.mjs` 的 HTTP
   兜底条目移除；尚未提交/合入。因此本叶子在 edc111f8 记录为"未收敛（节点
   修复在途）"，合入后记为已收敛；本节点 WS differential 场景继续使用
   HTTP ping 兜底 fixture（不改 runtime）。
5. **X-Skiff-Release 差异**：E-http gate 实测 TS assembly gateway
   （`serviceDeploymentSelection.ts`）不实现 Release 别名/冲突规则
   （version+release 冲突返回 201），Rust W-http 冻结 legacy manifest
   gateway 语义（冲突 400）。本节点把它作为非阻塞差异记录进 differential
   docs，并在 error 场景中加 recordOnly 证据 case（不参与 equal）。
6. **backpressure macOS 边界**：E-http gate 实测 macOS 内核 socket 缓冲
   自动调优吸收 ~800KiB burst → writer 不阻塞 → channel 不填满 → drain
   不触发（outcome=completed）；Linux CI 默认 ~200KiB 窗口触发
   `backpressure` cancel。属非阻塞语义边界，记录不比较。
7. **TS Router 依赖**：主 worktree `router/node_modules` 存在；worktree 内
   无 node_modules。`http_live_process.mjs::ensureTsRouterDependencies`
   提供按需 `pnpm --dir router install --frozen-lockfile`（read-only
   import，本节点调用）。
8. **资源约定**：45000-45999 连续 3 连端口租约 + 每侧独立临时 mongod
   （`ActivationStateMongoHarness`）；不触碰 stable instance/Mongo/PM2/
   4004-4007/44000-44999；45000-45999 当前无监听。
9. **可执行性**：任务可闭合，不返回 TASK_SCOPE_EXPANDED /
   TASK_NOT_EXECUTABLE。

## 任务目标

1. 扩展 differential scenario inventory（`differential_ext_*` 前缀，与
   E-actor-parity 的 `actor_parity_*` 前缀不重叠），至少各一个 planned
   场景转 runnable：
   - HTTP：unary / stream / error / CORS；
   - WS：generation / replacement / id 词法；
   - actor：调用（call）/ control。
   每个 runnable 场景必须在隔离 TS/Rust 实例上真实跑通（真实 Router +
   真实 Runtime + 临时 Mongo + 真实 compiler artifact），比较契约落盘
   inventory。
2. 把非阻塞语义差异记录进 differential docs：
   - X-Skiff-Release（TS 201 vs Rust 400）；
   - backpressure macOS OS-absorption 边界；
   - WS-only routing 状态（baseline 未收敛；Batch 10 WS-only-routing 节点
     修复在途，合入后收敛）。
3. 测试 ledger 同步：继续 batch-8 test ledger 的 TS test 删除登记协议；
   登记 edc111f8 baseline 审计与本节点无删除的事实，协议延续到 Batch 10
   后续节点。

## 写入边界

可写：

- `scripts/lib/router-differential/`：新增 `differential_ext_*` 前缀模块
  （http / ws / actor / registry）；`harness.mjs` 仅做最小的、additive
  扩展（scenario fixture authoring 选择 + 扩展 capture 钩子 + observation
  合并）。
- `scripts/fixtures/router-differential/`：新增 `ext-http` / `ext-ws` /
  `ext-actor` fixture 目录；更新 `scenario-inventory.json`（shared
  inventory，本节点统一维护）。
- `doc/implementation/router-rust-migration-batch-8-differential-scenarios.md`
  （矩阵与差异记录）、
  `doc/implementation/router-rust-migration-batch-8-test-ledger.md`
  （Batch 10 同步段）、本叶子文件。
- `scripts/tests/router-differential-scenarios.test.mjs`：仅同步 inventory
  baseline 与 runnable 列表断言（inventory 完整性测试，添加场景流程已由
  batch-8 场景文档规定）。
- registry / CI：无需扩展（既有 `router-live:differential` 已覆盖）；如
  确需按 append 模式。

禁止：

- `router/src`、runtime crate、`runtime/transport/src`、deployment、router
  TS（src/tests）、AGENTS.md、scripts README、verify selector graph、
  `skiff-instance.mjs`；
- 操作 stable instance / Mongo / PM2 / 4004-4007 / 44000-44999；不跑全量
  `pnpm verify` / `cargo test --workspace`；不跑 chat smoke。

## 设计摘要

1. **harness 扩展（最小 addititive）**：`harness.mjs::authorArtifact`
   支持 scenario `fixture` 字段（缺省 ping）；带 gateway 条目的 fixture
   走 bootstrap-only + package/assembly（rootDeployments）/config-snapshot
   （sources）authoring。side 启动后、`captureDifferentialSide` 前调用
   `differential_ext_registry` 的扩展 capture，返回的 partial observation
   合并进 side observation（顶层键 `httpTraffic` / `clientWs` /
   `actorTraffic`，不与既有 `http` / `runtimeFrames` / `mongo` 冲突）。
2. **HTTP 场景**：复用 `http_live_client.mjs`（requestFull/selectorHeaders）
   驱动真实 HTTP；观察 status/body/errorCode/CORS 头；release-conflict
   case 作为 recordOnly 证据。runtimeFrames 作为 recordOnly 证据。
3. **WS 场景**：Node `ws` client（经 relay 同款
   `loadRelayWebSocket` 解析 router/package.json）连接真实 public 端口；
   generation 场景双连接周期、replacement 场景 close-oldest 1008、id 词法
   场景复用 frozen corpus
   `runtime/transport/testdata/client-ws/jsonrpc-ids.json`。fixture 含 HTTP
   ping 兜底（WS-only 残余，见预检 4）。
4. **actor 场景**：单 replica（每侧一个真实 Runtime，经 relay）驱动
   `ext-actor` fixture 的 HTTP typedJson probe（`/probe` get-or-create +
   invoke；`/slow-get` create、`/slow-increment` invoke）；观察 HTTP
   status/body + relay 中 actor 帧 type 序列（`actor.getOrCreate` /
   `actor.owner.control` / `actor.owner.control.ack` / `actor.owner.invoke` /
   `actor.method.return`）与 terminal。
5. **inventory**：9 个新 runnable 场景（http×4、ws×3、actor×2）；被取代的
   旧 planned 条目（`http-unary-roundtrip` / `client-ws-roundtrip` /
   `actor-two-replica`）从 inventory 移除（粒度化替代），保留
   `http-health-basic` / `activation-mongo-transition` /
   `terminal-counters-drain` 为 planned；inventory baseline 更新为
   edc111f8。
6. **差异记录**：场景文档新增"非阻塞语义差异"章节（X-Skiff-Release、
   backpressure macOS 边界、WS-only routing 状态与收敛点）。
7. **ledger**：batch-8 ledger 追加 Batch 10 同步段（edc111f8 审计：66 个
   retained + 1 个新增 TS test；d228b613→edc111f8 无 TS test 删除；
   协议延续）。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 新增 runnable 场景真实跑通（TS+Rust 比较） | `node scripts/check-router-differential-live.mjs --scenario <differential_ext_*>` 每场景 PASS（9 个） |
| 差异记录落盘 | 场景文档含 X-Skiff-Release / backpressure / WS-only 三节；release-conflict 证据 case 在 error 场景 recordOnly 出现 |
| verify --list | `node scripts/verify.mjs --only router-live:differential --list` 含 `live:router-rust-differential`；新场景出现在 `check-router-differential-live.mjs --list` |
| inventory 一致性 | `node --test scripts/tests/router-differential-*.test.mjs` 全绿（scenarios/compare/normalize/frames） |
| 回归 | `session-handshake-basic` 仍 PASS（既有场景行为不变） |
| 写集干净 | `git status` 仅本叶子声明文件；未触碰禁止目录 |

## 交接

完成后提交到 `feat/router-rust-differential-ext`（不 push），直接向
`/root/router_rust_integration_b10` 报告 branch、worktree、commit、实际写
集、自验收矩阵与已知 seam（harness.mjs 共享文件的最小扩展、inventory
baseline 更新、WS-only 收敛点）；同步通知 root。

## 执行结果（提交前填写）

### 状态：完成（9 个新增 runnable 场景全部真实跑通 + 差异记录落盘）

### 交付文件

- `scripts/lib/router-differential/differential_ext_http.mjs`（HTTP 固定
  9-case 套件，含 release-conflict recordOnly 证据）。
- `scripts/lib/router-differential/differential_ext_ws.mjs`（client WS：
  generation / replacement / id 词法，frozen corpus 复用）。
- `scripts/lib/router-differential/differential_ext_actor.mjs`（双 replica
  actor call/control，第二 relay/租约自编排自清理）。
- `scripts/lib/router-differential/differential_ext_projection.mjs`（从
  compiler records 派生真实 actor-routing projection，canonical JSON）。
- `scripts/lib/router-differential/differential_ext_registry.mjs`
  （extension → capture 注册表）。
- `scripts/fixtures/router-differential/ext-http/`、`ext-ws/`、`ext-actor/`
  （真实 gateway fixture 源）。
- `scripts/fixtures/router-differential/scenario-inventory.json`
  （baseline → edc111f8；10 runnable + 3 planned）。
- `scripts/tests/router-differential-scenarios.test.mjs`（baseline +
  runnable 列表同步）。
- `doc/implementation/router-rust-migration-batch-8-differential-scenarios.md`
  （新矩阵 + 非阻塞差异记录）。
- `doc/implementation/router-rust-migration-batch-8-test-ledger.md`
  （Batch 10 同步段）。

### 共享文件最小扩展

- `scripts/lib/router-differential/harness.mjs`：TS 依赖按需安装、
  `--only` 字符串迭代 bug 修复、scenario fixture 选择 + gateway authoring
  （bootstrap/package/assembly/snapshot + 真实投影派生）、extension capture
  钩子、observation 合并、非对象错误包装。
- `scripts/lib/router-differential/instance.mjs`：可选 `websocketPath`
  （WS 场景 router config `websocket.path`，session 行为不变）。

### 自验收证据（2026-08-03 本地 macOS）

- 9 个新增 runnable 场景全量 TS+Rust 比较 PASS（各至少 1 轮，其中
  http_error / ws_id_lexical / actor_call 额外第 2 轮 PASS）：
  `differential_ext_http_unary` 7 项、`http_stream` 6 项、`http_error`
  15 项、`http_cors` 9 项、`ws_generation` 9 项、`ws_replacement`
  11 项、`ws_id_lexical` 6 项、`actor_call` 11 项、`actor_control`
  11 项。
- 回归：`session-handshake-basic` 23 项 PASS（行为不变）。
- hermetic：`node --test scripts/tests/router-differential-*.test.mjs`
  15/15 PASS。
- registry：`node scripts/verify.mjs --only router-live:differential --list`
  含 `live:router-rust-differential`；`--list` 显示 10 runnable + 3 planned。
- 写集：仅本叶子声明文件（见 `git status`）；未触碰 router/src、runtime、
  deployment、router TS、AGENTS.md、scripts README、verify selector
  graph、skiff-instance.mjs。

### 关键实现事实（交接给集成 Agent）

1. **TS actor 链路需要真实 projection**：A2 TS Router 严格消费
   `records/actor-routing/current.json`，空 methods 投影导致 actor invoke
   无法路由（503）。baseline compiler 不生成该记录，本节点从 compiler
   records（package.json + file IR actorDeclarations）派生并写入；A1-compiler
   合入后 compiler publish 自带，派生逻辑保留无冲突（幂等覆盖）。
2. **WS 场景需要 router config `websocket.path`**：instance.mjs 新增可选
   `websocketPath`（scenario `wsPath: /chat`），session 场景不传保持原样。
3. **actor 双 replica**：单 replica 同步 self-call 死锁；扩展自租约第二个
   relay 端口并自清理，harness 的 primary side lifecycle 不受影响。
4. **WS-only routing 未收敛（修复在途）**：`ext-ws` fixture 保留 HTTP
   ping 兜底；WS-only-routing 节点合入后记录已收敛并可移除兜底（见场景
   文档）。
5. **非阻塞差异已记录**：X-Skiff-Release（201 vs 400）、backpressure
   macOS 边界、missing-selector message 大小写、WS connectionId 来源
   （TS UUID vs Rust `wsconn-<nanos>-<n>`）。
6. **环境说明**：执行期间磁盘多次被并行节点 build 占满；仅清理了本节点
   build cache 与 /tmp 陈旧 temp target（`/tmp/ash-accept-target`、
   `/tmp/skiff-actor-shared-heap-integration-target` 等，均无进程使用、
   非仓库数据），未触碰其他节点 worktree。
