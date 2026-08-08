# Router Rust Migration Batch 8 — Differential Scenario Inventory

日期：2026-08-02
归属：W-differential（`feat/router-rust-w-differential`，baseline
`origin/main@d228b613`）
机器可读源：`scripts/fixtures/router-differential/scenario-inventory.json`
（本文件是其人工可读镜像；两者必须保持一致）。

更新：2026-08-03（Batch 10 differential 扩展，baseline
`origin/main@edc111f8`，归属 `feat/router-rust-differential-ext`）——
新增 HTTP/WS/actor 九个 runnable 场景（`differential_ext_*`），替换旧的
planned 占位条目；记录非阻塞语义差异（X-Skiff-Release、backpressure
macOS 边界、WS-only routing 状态）。

## 目标

实现权威设计 §9 的 implementation-neutral differential harness：TS/Rust
Router 使用独立端口、artifact root、runtime home、Mongo namespace；不共享
Runtime、不镜像 live traffic；对比 HTTP、WS、Runtime frames、health、
Mongo state/audit、terminal counters。

## 观察类型

| 类型 | 捕获方式 | 当前状态 |
| --- | --- | --- |
| `http` | 每侧独立 http/control 端口上的真实 HTTP 请求（status/body） | `session-handshake-basic` 比较 control health status；body 记录为证据 |
| `clientWs` | 每侧独立 public 端口的 client WebSocket 帧 | planned（Rust client WS 未装配） |
| `runtimeFrames` | test-only WS relay：real Runtime ↔ relay ↔ Router，双向记录 SKBF 帧（direction/type/header） | `session-handshake-basic` 全序列比较 |
| `health` | `GET /__router/health` JSON | planned（d228b613 Rust listener 返回 200 空体） |
| `mongoState` | 各实现 canonical namespace 内 decode 的 `EnvironmentActivationState` 文档 | `session-handshake-basic` 比较 |
| `mongoAudit` | 各实现 canonical audit collection 的条目 | `session-handshake-basic` 比较（0 条） |
| `terminal` | SIGTERM/SIGINT 后 exit code、端口关闭状态 | `session-handshake-basic` 比较 |
| `logs` | router/runtime stdout/stderr | 记录为证据（normalization 政策允许无语义 log order） |

## Normalization 政策（§9 严格子集）

只允许四种 normalization，且每个 scenario 必须显式声明应用到哪些
observation path；未声明的值差异直接判失败：

1. `uuid`：UUID 值替换为 `<uuid>`。
2. `timestamp`：ISO-8601 或 epoch-millis 时间值替换为 `<timestamp>`。
3. `port`：等于该侧租约端口的值替换为 `<port>`。
4. `logOrder`：日志行按字典序排序（无语义 log order）。

artifact root、mongoUrl、runtime home 等场景配置值**不属于**
normalization：它们通过 `sideExpected` 断言与各侧自己的配置精确一致，从而
在不掩盖实现差异的前提下满足"独立 artifact root / runtime home / Mongo
namespace"约束。当 `equal` 的整对象比较需要排除这类按侧配置值时，使用
`equal[].exclude` 显式列出子路径；被排除路径必须同时被 `sideExpected` 或
`recordOnly` 覆盖，否则 inventory 校验拒绝（见
`scripts/lib/router-differential/scenarios.mjs`）。

## 场景矩阵

| ID | Lane | 状态 | 内容 | Blocked on |
| --- | --- | --- | --- | --- |
| `session-handshake-basic` | session | runnable | real Runtime bootstrap/capabilities/Register/registered/health 经 relay 双向捕获；HTTP control health status；seeded Mongo state/audit；SIGTERM/SIGINT terminal | 无（d228b613 已满足） |
| `differential_ext_http_unary` | http | runnable | real HTTP rawHttp unary（201）+ typedJson unary（200）经 trusted selectors → real Runtime；status/body/terminal 跨 TS/Rust 一致 | 无（edc111f8 已满足） |
| `differential_ext_http_stream` | http | runnable | real HTTP server-stream（206 `alpha|middle|omega`）→ real Runtime；status/body/terminal 一致 | 无 |
| `differential_ext_http_error` | http | runnable | service ProtocolError（500 `UnhandledServiceError`）、missing selector（400）、wrong path（404）status/code 一致；X-Skiff-Release conflict（TS 201 vs Rust 400）仅记录 | 无 |
| `differential_ext_http_cors` | http | runnable | automatic preflight（204 + echoed allow-origin）+ service-managed OPTIONS（204 + service 头）一致 | 无 |
| `differential_ext_ws_generation` | ws | runnable | 两个 connect/status/close 周期：connect status + business response 一致（connectionId 为 router mint，仅记录） | 无 |
| `differential_ext_ws_replacement` | ws | runnable | maxConnections 1 / close-oldest：第二个连接以 1008 supersede 第一个并服务新 roundtrip | 无 |
| `differential_ext_ws_id_lexical` | ws | runnable | frozen JSON-RPC id 词法 corpus（`1e0→1`、`-0→0`、string 保留、safe-integer 边界、-32700/-32600 platform errors）一致 | 无 |
| `differential_ext_actor_call` | actor | runnable | 两个 real Runtime replica：get-or-create + 两次增量 invoke；HTTP typed 响应 + actor 帧 type counts 一致 | 无 |
| `differential_ext_actor_control` | actor | runnable | 两个 replica：slow create claim + invoke + 第二个 actor claim；owner control/ACK 帧 counts 一致 | 无 |
| `http-health-basic` | http | planned | health JSON parity（activeAssembly/replicas/capabilityConnections） | W-composition / E-http（Rust health JSON） |
| `activation-mongo-transition` | activation | planned | activate HTTP → Mongo state/audit transition | W-composition / E-activation |
| `terminal-counters-drain` | terminal | planned | 流量后 shutdown，counters 归零且两实现一致 | differential_ext_http_* / differential_ext_ws_* / differential_ext_actor_* + health parity |

## Batch 10 扩展：harness 扩展机制

`scenario-inventory.json` 条目可声明 `extension`（`http` / `ws` / `actor`）
与 `fixture`（`ext-http` / `ext-ws` / `ext-actor`），以及场景专用参数
（WS：`wsPath` + `wsMode`；actor：`actorMode`）。`harness.mjs` 在每侧
Runtime handshake 后调用
`scripts/lib/router-differential/differential_ext_registry.mjs` 注册的
capture 模块，把 partial observation（`httpTraffic` / `clientWs` /
`actorTraffic` / `actorFrames`）合并进 side observation；比较契约按这些
路径声明。带 gateway 条目的 fixture 走 bootstrap-only + package/assembly
（rootDeployments）/config-snapshot（sources）authoring；actor fixture 的
`records/actor-routing/current.json` 由
`differential_ext_projection.mjs` 从 compiler 不可变 records 派生真实
projection（A2 TS hard cut 需要，A1-compiler 合入后 compiler publish 自带
该记录；派生逻辑保留用于 baseline）。

actor 场景每侧使用两个真实 Runtime replica（第二 replica 经独立 relay 与
租约端口，由 `differential_ext_actor.mjs` 编排，扩展返回前停止并关闭端口）；
同步 self-call 在单 replica 上会死锁，双 replica 是既有 E-actor-rust
full-chain 的已验证拓扑。actor 帧比较用 combined per-type counts（对
"哪个 replica 执行了 probe" 与跨 relay 交织顺序鲁棒），per-relay sequence
作为 recordOnly 证据。

## 非阻塞语义差异记录（Batch 10）

以下差异经真实 TS/Rust 差分场景或兄弟 gate 实测确认，当前**不参与**任何
`equal` 断言；按 §9 normalization 政策也不允许用 normalization 掩盖，因此
全部登记为 recordOnly 证据 + 本文档记录。

### X-Skiff-Release：TS 201 vs Rust 400

TS assembly gateway（`serviceDeploymentSelection.ts`）不实现
`X-Skiff-Release` 别名/冲突规则：`X-Skiff-Version` 与 `X-Skiff-Release`
同时给出时返回 201 并正常 dispatch；Rust W-http 冻结 legacy manifest
gateway 语义，同请求返回 400 `InvalidVersionHeader`。`differential_ext_http_error`
场景把该 case 作为 `httpTraffic.8` recordOnly 证据（实测 TS 201 / Rust
400），不做 equal。出处：E-http gate leaf（`router-rust-e-http-gate-leaf.md`）。

### backpressure macOS OS-absorption 边界

E-http gate 实测：macOS 内核 socket 缓冲自动调优吸收 ~800KiB burst，writer
不阻塞 → Router 32-slot stream channel 不填满 → 10s drain deadline 不触发，
请求正常完成（outcome=completed）；Linux CI 默认 ~200KiB 窗口下 writer
阻塞、channel 填满、~10.5s 触发 `backpressure` cancel。冻结常量（session
inbound 64 帧/1MiB、channel 32、drain 10s）使 macOS 类大吸收主机上该
terminal 不可达。differential 场景不比较 backpressure outcome；如需全平台
确定性覆盖，建议后续让 session budgets / drain timeout 可配置（contract/
生产裁决，不在本节点）。出处：E-http gate leaf。

### WS-only routing 状态

基线 `edc111f8` 的残余缺口（runtime
`control_plane.rs::dispatch_modes_from_gateway_entries` 只统计 HTTP
gateway 表面，WS-only deployment 广告空 dispatch_modes，E-ws gate 曾用
额外 HTTP ping 条目规避）已由 Batch 10 WS-only-routing 节点关闭：
`feat/router-rust-ws-only-routing`（commit `735e590d`，经集成分支 merge
`5e185c05` 合入）把 WebSocketConnect/WebSocketJsonRpc 表面计入
dispatch_modes，并移除 `scripts/check-router-ws-live.mjs` 的 HTTP 兜底
条目；该 harness 在真实 WS-only artifact 全链上 PASS。**WS-only routing
状态已收敛**。本节点 `differential_ext_ws_*` 场景的 `ext-ws` fixture 保留
HTTP ping 条目仅作为跨实现基线冗余，不再构成路由依赖；后续如需去除由
scripts 侧 owner 处理（本节点写边界外）。

### 附加观察（记录，不比较）

- **missing-selector 错误消息大小写**：TS 返回 `X-Skiff-Service is required`，
  Rust 返回 `x-skiff-service is required`（error code 均为
  `ServiceSelectorRequired`）。`differential_ext_http_error` 只比较
  status + errorCode，message 进 recordOnly。
- **WS connectionId 来源**：TS business 结果中的 `connectionId` 是
  Runtime 侧 UUID；Rust 结果是 Router mint 的 `wsconn-<nanos>-<n>`。
  `differential_ext_ws_*` 只比较业务字段，`rawResponses` 进 recordOnly。
- **actor 投影依赖**：A2 TS Router 严格消费
  `records/actor-routing/current.json`；空 methods 投影会让真实 actor
  invoke 无法路由（503 `ProviderUnavailableError`）。Rust 侧不需要该投影。
  A1-compiler 合入后 compiler publish 自动生成真实投影；baseline 下由
  `differential_ext_projection.mjs` 从 compiler records 派生。

## 运行方式

```bash
# 列出 inventory
node scripts/check-router-differential-live.mjs --list

# 只跑单个 runnable 场景（TS + Rust）
node scripts/check-router-differential-live.mjs --scenario session-handshake-basic

# 只跑一侧（调试用，不比较）
node scripts/check-router-differential-live.mjs --scenario session-handshake-basic --only ts

# 保留临时证据目录
node scripts/check-router-differential-live.mjs --keep-temp
```

资源约定：router/relay 端口租约 45000-45999；每个 side 一个临时
single-node replica set（`ActivationStateMongoHarness`，45000-45999 独立
端口）；不触碰 stable instance、stable Mongo（27017）、PM2、
4004-4007、44000-44999。

verify 注册：`router-live:differential`（live/manual、managed），id
`live:router-rust-differential`；不在 default `verify` / manual `router`
selector 中。

## 添加场景

1. 在 `scenario-inventory.json` 增加条目（id/status/lane/observationTypes/
   normalizations/compare），planned 场景必须给出 `blockedOn`。
2. 需要新观察类型时先扩展 `scripts/lib/router-differential/scenarios.mjs`
   `OBSERVATION_TYPES` 与 capture 实现，再登记；新 capture 模块使用
   `differential_ext_*`（或 E-actor-parity 的 `actor_parity_*`）前缀并在
   `differential_ext_registry.mjs` 注册。
3. 更新本文件矩阵与 `scripts/tests/router-differential-scenarios.test.mjs`
   的完整性断言。
4. runnable 场景必须能在本机隔离实例真实跑通后才能标 `runnable`。
