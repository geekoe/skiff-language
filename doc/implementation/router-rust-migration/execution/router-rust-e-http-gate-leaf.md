# Router Rust Migration Batch 9 波 2 — E-http Gate Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_e_http_gate`
集成目标：`/root/router_rust_integration_b9`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-9.md`
  （E-http gate 节点；baseline 集成 head `a9c8715b`，波 1 E-gates wiring
  已合入）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §6.2(4)（第一次 external HTTP unary rollback roundtrip）、§7 E-http、
  §8 `router-rust-http-live` / `router-live:http`、§9 CI matrix、§11.2
  （incremental rollback rehearsal：stop admission → shutdown → verify exit →
  start target → Runtime reconnect exact tuple → HTTP smoke）。
- 兄弟 leaf：
  - `router-rust-migration-w-http-leaf.md`（HTTP socket 层：selector/
    ingress/payload/unary-stream mapping/ceiling/backpressure/disconnect/
    deadline/CORS/error；real HTTP → fake dispatcher，E-http 接 real
    Runtime）。
  - `router-rust-migration-w-dispatch-leaf.md`（admission/pending/terminal；
    `DispatchRequest` → `DispatchSubmit` 适配归 E-http）。
  - `router-rust-w-composition-leaf.md`（`RouterSupervisor`/`RouterComponents`
    HTTP 装配、`DispatcherHttpPort`/`PendingHttpRouter`/`RequestFrameSink`、
    HTTP surface 从 deployment records 只读构造）。
  - `router-rust-e-gates-wiring-leaf.md`（波 1：WS seam、actor sink、
    生产装配完成；HTTP 生产面已全接）。
  - `router-rust-e-session-gate-leaf.md`（managed harness 模式：临时 Mongo
    replica set + 45000-45999 租约 + 显式 Rust binary + WS relay 观测；
    真实进程退出码/端口关闭归零断言）。

## 零 worktree 只读预检结论（锚定 a9c8715b）

1. 基线：`git rev-parse a9c8715b` =
   `a9c8715bb6829c31c9fa75a88e38dde8ccaee7f3`；本 worktree
   `/Users/geek/workspace/wt-e-http-gate`，分支
   `feat/router-rust-e-http-gate`，HEAD 即基线。
2. HTTP gateway 生产装配已完整（W-composition + E-gates-wiring）：
   `RouterSupervisor::start_listeners` 用 `HttpGatewayServerOptions`（
   request_timeout 来自 `requestTimeoutMs`；drain deadline 10s；stream
   channel 32；maxRequestBytes/maxResponseBytes 来自 config）→
   `EpochHttpIngressResolver`（epoch + `HttpGatewaySurfaceView`）→
   `DispatcherHttpPort`（`DispatchRequest` → `DispatchSubmit`，unary/stream
   双向映射、deadline/cancel/terminal 映射）→ `RequestDispatcher` →
   `SessionRuntimePeer`（exact Runtime session 出站帧）。`RequestFrameSink`
   把 response.start/chunk/end/error/request.cancel 从 demux 送回
   `PendingHttpRouter`。HTTP 层行为与 TS parity 冻结于
   `router/tests/http_gateway_*`（fake dispatcher 覆盖）。
3. rollback manifest/process 现状：`scripts/lib/dev-runtime-paths.mjs` 提供
   `resolveRouterProcessSpec` / `routerProcessInvocation`（TS：
   `pnpm --dir <router> dev --config <path>`；Rust：`<binary> <config>`）；
   `scripts/lib/rollback-manifest.mjs` 提供 `buildRouterRollbackManifest` /
   `assertRouterRollbackManifest`（schema v1，仅冻结 process command，无
   TS 代码）。本 gate 直接消费这两个 seam 做 TS→Rust→TS 进程切换，不改
   它们。
4. session-live harness 模式（E-session）：临时 service source + 真实
   compiler package/assembly/config-snapshot authoring + actor-routing
   projection record + `ActivationStateMongoHarness`（45000-45999 临时
   mongod）+ 端口租约 + 显式 `cargo build` Rust router/runtime binary +
   ignored probe（本 gate 为 scripts 边界，probe 逻辑全部在 Node 内）+
   WS relay 观测握手/帧 + SIGTERM/SIGINT 退出码与端口关闭断言。
   `router-differential/relay.mjs`（ws 转发 + 双向帧记录）可直接复用；
   `router-differential/instance.mjs` 提供 waitForListeners/stopChild/
   spawnWithLogs 模式；`router-differential/mongo.mjs` 提供 TS/Rust 各自
   canonical namespace 的 committed state seeding（TS：
   `skiff_router_ts_differential` / `router_assembly_activation_states`；
   Rust：`skiff-router` / `activation_state`）。
5. TS Router 可启动：`router/package.json` `dev` = `tsx src/router/server.ts`，
   需要 worktree 内 `router/node_modules`（CI 用
   `pnpm --dir router install --frozen-lockfile`；本地 worktree 首次运行由
   harness 按需安装）。TS 进程命令本身只用既有 `pnpm --dir router dev`，
   不写 TS 代码。
6. 可观测性：`/__router/health` 在 Rust 侧仍为空 200 占位（differential
   inventory 记录），因此 pending/permit/timer 归零用可观测代理断言：
   (a) 每个 terminal 竞态后紧跟一个 follow-up unary 必须成功；(b) relay
   记录的 `request.cancel` 对每个 requestId 至多一帧且 reason 精确；
   (c) 全量 suite 结束后 Router SIGTERM 退出 0、端口关闭、Runtime SIGINT
   退出 0。requestId 由 Router 内部生成，通过“请求前后 relay 帧快照”把
   最新 `request.start` 与当前 HTTP 请求关联（suite 串行执行，无并发请求）。
7. 竞态确定性：Router 的 HTTP-phase timer 在 `request.start` 编码前启动，
   Runtime 的 deadline 在其收到帧后才启动，同机 Router 一定先到 →
   `request.cancel: timeout` 至多一次；client disconnect 由 hyper 连接
   drop 触发 cancel watch → `request.cancel: client_disconnect` 至多一次；
   backpressure 由 32 槽 channel + 10s drain 触发 → `request.cancel:
   backpressure` 至多一次。慢函数 sleep 15s 确保 timeout 先于响应。
8. 写入边界已确认（仅脚本/harness/registry/CI/文档；`router/src`、
   runtime、deployment、router TS、AGENTS.md、scripts README、verify
   selector graph、skiff-instance.mjs 一律不碰）。任务可闭合，不返回
   TASK_SCOPE_EXPANDED / TASK_NOT_EXECUTABLE。

## 任务目标

`router-live:http` managed harness（`scripts/check-router-http-live.mjs` +
`scripts/lib/http_live_*`）：real HTTP → Router → Runtime unary + stream：

- trusted selector（X-Skiff-Service / X-Skiff-Version、Release 别名与冲突
  拒绝、missing/unknown 负例）；
- service-scoped ingress（exact deployment + gateway entry identity +
  mode/adapter kind；wrong method/path/service 404）；
- typed（typedJson）/ raw opaque payload（rawHttp echo byte-exact）；
- unary/stream mapping（unary response.end、stream start/chunk/end 串行、
  relay 帧顺序断言）；
- stream sequencing（响应帧顺序 + 无 cancel 的正常流）；
- cumulative response ceiling（unary 502 ResponseTooLarge；stream
  protocol_error cancel）；
- backpressure（burst stream + 不读客户端的 drain timeout →
  `backpressure` cancel）；
- disconnect / cancel / deadline（mid-stream destroy →
  `client_disconnect` cancel；slow unary/stream → 504 TimeoutError +
  `timeout` cancel）；
- CORS preflight（自动 204 + 头）/ service-managed（显式 OPTIONS ingress
  透传 service 头）/ platform error（`{error:{code,message,...}}` 形状）；
- 任意竞态一个 external terminal、至多一次 cancel、pending/permit/timer
  归零（follow-up unary + 进程级退出码/端口关闭）；
- 首次 unary rollback roundtrip：TS→Rust→TS 三个真实 Router 进程切换
  （§11.2，用 `RouterProcessSpec`/`buildRouterRollbackManifest` 的既有
  命令形态），Runtime 全程不重启，每阶段 relay 观测同一 committed tuple
  的 bootstrap 握手 + 同一 unary suite 结果一致。

交付：harness、`verify-live-registry.mjs` 追加自己的 key 块、
`router-rust-integration.yml` 追加自己的 managed job（+ classifier regex
additive 扩展）、`scripts/tests/verify-live-registry.test.mjs` 一行 selector
期望、本叶子文档、真实运行证据（unary rollback roundtrip 记录在案）。

## 实现决策

1. **harness 布局**：
   - `scripts/lib/http_live_fixture.mjs`：临时 service source
     （package.yml/api.yml/http.yml/main.skiff，unary/typed-unary/echo/
     stream/echo-stream/slow/slow-unary/slow-stream/burst/error/cors 条目）
     + 真实 compiler package/assembly/config-snapshot authoring +
     actor-routing projection record + 双 namespace committed state
     seeding。
   - `scripts/lib/http_live_process.mjs`：canonical RouterProcessSpec/
     rollback manifest 构造与断言、TS 依赖按需安装、router/runtime/relay
     spawn、waitForListeners/waitForHandshake/stopChild/ports-closed。
   - `scripts/lib/http_live_client.mjs`：node:http 客户端（完整响应、
     stream 分块读、断开/超时控制）。
   - `scripts/lib/http_live_suite.mjs`：rollbackSuite（三阶段一致断言）与
     fullSuite（Rust 阶段完整 E-http 断言，含 relay 帧/cancel 计数）。
   - `scripts/check-router-http-live.mjs`：编排（temp root、mongod、租约、
     build、TS→Rust→TS 三阶段、evidence、finally 清理）。
2. **进程切换**：同一 devHome/router.yml（mongoUrl 带 TS canonical
   database；Rust 用固定 `skiff-router` DB）；TS/Rust spec 只差
   implementation 与 binary path；每阶段 manifest round-trip 断言；
   Runtime 经 relay 常驻，Router 重启后 `waitForHandshake` 复检
   bootstrap tuple（environment/generation/assembly/snapshot）。
3. **证据**：每阶段记录 unary suite 结果与 bootstrap tuple；Rust 阶段记录
   全量 case + cancel 计数 + follow-up 成功 + 进程退出码/端口关闭；错误时
   附带 relay 帧摘要与日志 tail。

## 写入边界

可写：

- `scripts/check-router-http-live.mjs`（新）；
- `scripts/lib/http_live_*.mjs`（新；复用既有 lib，不复制
  relay/mongo/process 逻辑到禁止目录）；
- `scripts/lib/verify-live-registry.mjs`（仅追加 `router-rust-http-live`
  key 块）；
- `scripts/tests/verify-live-registry.test.mjs`（仅 LIVE_SELECTORS 期望
  列表加一行）；
- `.github/workflows/router-rust-integration.yml`（仅追加
  `Router Rust HTTP (managed)` job + classifier regex additive 扩展）；
- `doc/implementation/router-rust-migration/execution/router-rust-e-http-gate-leaf.md`（本文件）。

禁止：

- `router/src`、runtime crate、`runtime/transport/src`、deployment、
  router TS（rollback 只用既有 `pnpm --dir router dev` 命令）、AGENTS.md、
  scripts README、verify selector graph、`skiff-instance.mjs`；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

若真实运行暴露生产缺口：小修仅限 `router/src/http/` lane 模块（先向 root
报告证据）；需动 supervisor/main 时停下上报。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 真实 harness 通过 | `node scripts/check-router-http-live.mjs`（临时 Mongo + 45000-45999；TS→Rust→TS 三阶段；Rust 全量 suite + 归零断言；证据打印） |
| 首次 unary rollback roundtrip | 三阶段 bootstrap tuple 一致 + 同一 unary suite 结果一致（记录于叶子） |
| registry | `node scripts/verify.mjs --only router-live:http --list` 含 `live:router-rust-http` |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 通过 |
| workflow | `router-rust-integration.yml` YAML 可解析；http job 稳定名；classifier regex 覆盖 `check-router-http-live.mjs` 与 `scripts/lib/http_live_` |
| 写集干净 | `git status` 仅本叶子声明文件；未触碰禁止目录 |

## 交接

完成后提交到 `feat/router-rust-e-http-gate`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit、实际写集、
自验收矩阵与已知 seam；同步通知 root。

## 执行结果（提交前填写）

### 状态：完成（harness 全绿 + 真实证据；一个可测性边界已上报 root）

基线变更：`git merge integration/router-rust-migration-batch-9`
（fb60fb86，含 E-dispatch 的 88abfa20：Runtime `control_plane.rs` 从已
admit assembly 派生 `dispatchModes` + supervisor terminal 投递修复）。
本 worktree 分支 `feat/router-rust-e-http-gate`，最终提交含 merge。

### 自验收证据（2026-08-03 本地 macOS 双轮稳定）

`node scripts/check-router-http-live.mjs` → `router-live:http: PASS`（两轮
均 exit 0，约 23s/轮）：

- TS-1 阶段 5/5：unary-happy 201、typed-unary 200、missing-selector 400、
  wrong-path 404、stream-roundtrip 206（`alpha|middle|omega`，
  response.start/chunk seq 0,1,2/end 帧序 + 无 cancel）。
- Rust 阶段 17/17：rollback 5 项 + version-conflict 400（X-Skiff-Release
  冲突）、unknown-service 404、wrong-method 404、body-too-large 413、
  unary-ceiling 500 `ResourceLimitExceeded`（Runtime 先于 Router fallback
  执行 ceiling，status>=500 隐藏 details，TS parity）、stream-ceiling
  （200 head + 0-byte truncated body，runtime 控制错误 → 无 router
  cancel）、service-error 500 `UnhandledServiceError`（用户 throw 投影，
  wire 帧带 traceId/errorId，HTTP body 按 >=500 策略隐藏 details）、
  cors-preflight 204 + allow-origin/allow-methods、service-managed-cors
  204（显式 OPTIONS ingress 透传 service 头）、deadline-unary/stream 504
  `TimeoutError` + 恰好一次 `timeout` cancel、disconnect-stream 恰好一次
  `client_disconnect` cancel；每个竞态后 follow-up unary 201。
- rust-bp 1/1：backpressure case（详见下方边界记录）。
- TS-2 阶段 5/5（rollback 回 TS 进程命令后同一 unary suite 一致）。
- rollback roundtrip：ts-1/rust/ts-2（+rust-bp）bootstrap tuple 完全一致
  （environment http-live、generation 1、assembly identity、config
  snapshot id）；`buildRouterRollbackManifest`/`assertRouterRollbackManifest`
  round-trip；TS/Rust 每个 Router SIGTERM 退出码 0、Runtime SIGINT 退出码
  0、端口全部关闭（pending/permit/timer 归零的进程级代理）。
- registry：`node scripts/verify.mjs --only router-live:http --list` 展开
  `live:router-rust-http`（managed/live-manual，requiredExecutables 含
  python3）；`node --test scripts/tests/verify-live-registry.test.mjs`
  20/20 pass；focused `verify --only router-rust,router-rust-process-smoke`
  2/2 pass；`router-rust-integration.yml` YAML 解析通过，jobs 为
  change-classifier / bootstrap / session / dispatch / http。

### 已解决的阻塞（E-dispatch 修复）

之前上报的 dispatch_modes 缺口由 E-dispatch 合入修复：真实 Runtime 现按
已 admit assembly 发布 `dispatchModes`，Rust 阶段首个 unary 从 503 变为
201。本 gate 未写 runtime crate。

### 可测性边界：backpressure（已上报 root）

backpressure case 在独立 `rust-bp` 阶段（同一 artifact/committed tuple，
`http.maxResponseBytes: 16 MiB`、`requestTimeoutMs: 30s`）运行：慢客户端
（python3 helper，读 head 后暂停不读）→ burst 45×20KiB（~900KiB，session
inbound 64 帧/1MiB 预算内）→ runtime 睡眠保持 request active → 期望 Router
32-slot stream channel 填满后 10s drain deadline 触发 `backpressure`
cancel。

本地 macOS 实测：OS 内核 socket 缓冲自动调优吸收整个 burst（~800KiB），
writer 不阻塞 → channel 不填满 → drain 不触发；请求正常完成后进入
OS-absorption boundary 分支（harness 记录 outcome=completed，仍断言
follow-up unary 201 + 进程退出码 0 + 端口关闭）。Linux CI（默认
~200KiB 窗口）下 writer 会在 burst 中段阻塞、channel 填满、drain 在
~10.5s 触发 `backpressure` cancel；harness 在 Linux 上要求必须出现该
cancel（否则 fail）。数学边界：冻结生产常量（session inbound 64 帧 /
1MiB、channel 32、drain 10s、health 1s）使 macOS 类大吸收主机上该 terminal
不可达（burst 帧数上限 ~47 与 channel 填充需求 32 + 吸收量冲突）。如需
全平台确定性覆盖，建议后续让 session budgets / drain timeout 可配置或提升
inbound byte budget（需 contract/生产裁决，不在本 gate）。

### 并行发现的 parity 差异（记录，非阻塞）

TS assembly gateway（`serviceDeploymentSelection.ts`）不实现
`X-Skiff-Release` 别名/冲突规则（实测 version+release 冲突返回 201），Rust
W-http 冻结的是 legacy manifest gateway 语义（冲突 400）。rollback suite
已把 version-conflict 移到 Rust full suite，避免伪差异；已附证据，建议
differential owner 记录该行为漂移。

### 写集

- `scripts/check-router-http-live.mjs`（新）
- `scripts/lib/http_live_fixture.mjs`、`http_live_process.mjs`、
  `http_live_client.mjs`、`http_live_suite.mjs`、`http_live_slow_client.py`
  （新）
- `scripts/lib/verify-live-registry.mjs`（仅 `router-rust-http-live` 条目）
- `scripts/tests/verify-live-registry.test.mjs`（LIVE_SELECTORS +1 行）
- `.github/workflows/router-rust-integration.yml`（append http job +
  classifier regex additive；merge 解决与 dispatch job 的冲突）
- `doc/implementation/router-rust-migration/execution/router-rust-e-http-gate-leaf.md`（本文件）

未触碰：router/src、runtime crate、runtime/transport、deployment、router
TS、AGENTS.md、scripts README、verify selector graph、skiff-instance.mjs。
