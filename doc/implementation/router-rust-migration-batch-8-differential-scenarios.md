# Router Rust Migration Batch 8 — Differential Scenario Inventory

日期：2026-08-02
归属：W-differential（`feat/router-rust-w-differential`，baseline
`origin/main@d228b613`）
机器可读源：`scripts/fixtures/router-differential/scenario-inventory.json`
（本文件是其人工可读镜像；两者必须保持一致）。

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
| `http-health-basic` | http | planned | health JSON parity（activeAssembly/replicas/capabilityConnections） | W-composition / E-http（Rust health JSON） |
| `http-unary-roundtrip` | http | planned | trusted selector unary → real Runtime → response | W-composition / E-http |
| `client-ws-roundtrip` | ws | planned | client WS JSON-RPC + generation lifecycle | W-composition / E-ws |
| `activation-mongo-transition` | activation | planned | activate HTTP → Mongo state/audit transition | W-composition / E-activation |
| `actor-two-replica` | actor | planned | two-replica actor claim/invoke/control/spawn | W-composition / E-actor-rust |
| `terminal-counters-drain` | terminal | planned | 流量后 shutdown，counters 归零且两实现一致 | 上述 traffic lanes + health parity |

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
   `OBSERVATION_TYPES` 与 capture 实现，再登记。
3. 更新本文件矩阵与 `scripts/tests/router-differential-scenarios.test.mjs`
   的完整性断言。
4. runnable 场景必须能在本机隔离实例真实跑通后才能标 `runnable`。
