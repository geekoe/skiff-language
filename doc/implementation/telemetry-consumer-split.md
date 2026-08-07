# Telemetry 生产/消费分仓 — 协议只管生产，消费端独立仓库

日期：2026-08-07
状态：实现计划（决策已定，待排期执行）

## 1. 背景与动机

2026-08-04 的 scheduler 热循环事故暴露了可观测性缺失，催生
`doc/architecture/observability-requirements.md`（平台级 / Service owner 级双视角
需求草案）。本计划解决两个层面的问题：

1. **License 边界**：skiff 语言、runtime、router 全部开源，但监控运维产品
   （telemetry server、查询、指标聚合、告警、dashboard、权限）不开源。边界划在
   **数据处**：开源仓库负责"产生事实"（协议 + 发射 + 默认落盘），消费端是独立
   闭源仓库，与 skiff 同级（本地 `internals/` 体系之外、`/Users/geek/workspace`
   下独立目录），只在数据面（WS / 文件）对接，不 import 开源内部实现。
2. **协议过度设计**：现有 `log/trace/metric/health/debug` 5 个 topic 把
   "来源/形态"混在一个维度里，并让 span 配对、指标分类这类**消费端职责**
   渗透进协议。原则改为：**协议只管生产**。

## 2. 已定决策（本计划依据）

| # | 决策 | 内容 |
| --- | --- | --- |
| D1 | 消费端独立仓库 | telemetry 消费端（server/查询/聚合/告警/dashboard/权限）独立仓库，与 skiff 同级，先在本地。建议名 `skiff-telemetry`（路径 `/Users/geek/workspace/skiff-telemetry`，可改） |
| D2 | 协议只管生产 | 去 topic；事件自描述；形态识别（日志/span/指标/状态）是消费端职责 |
| D3 | 默认落文件 | 无 `telemetry.endpoint` 时生产端默认写 JSONL 文件；配置三态：endpoint / file / disabled |

## 3. 协议重构（去 topic）

### 3.1 变更点

- `runtime/transport/src/protocol/control.rs`：
  - 删除 `TelemetryTopic` enum（:51）；
  - `TelemetryRegisterEnvelope` 删除 `topics` 字段（:141）；
  - `TelemetryEvent` 删除 `topic` 字段（:147）。
- router / runtime 配置中的 `telemetry.topics` 删除；按 topic 的采样/限流不再
  存在（后续按 name/level 过滤，属消费端策略，不进协议）。
- `telemetry/src/protocol.ts`（随消费端迁出）同步去 topic。

### 3.2 事件 schema（保留全部字段，仅去 topic）

```
ts, source, visibility,
service_id, revision_id, build_id, activation_identity,
runtime_id, provider_id, provider_revision, provider_capability, provider_target,
request_id, client_request_id, trace_id, error_id,
span_id, parent_span_id, target,
level, name, message, attrs, error, duration_ms, dropped
```

- **归属维度**：`service_id` 有/无 + `visibility`（Operational / Restricted）。
  skiff 代码发出的事件必带 service 归属（无 request context 时 no-op，
  `capability_context/telemetry.rs:192`）；平台 Rust 事件 `service_id` 为空。
- **来源维度**：`source`（gateway / router / runtime / provider / test）。
- **关联维度**：`request_id` / `trace_id` / `span_id` / `parent_span_id` /
  `client_request_id`。span 的 start/end 配对由消费端按 span_id 聚合，
  生产端只填关联字段，不声明 span 结构。
- **形态识别（消费端）**：`message`/`level` 非空 → 日志；`name`+`duration_ms`
  且带 span 关联 → 时间线片段；`name`+数值 `attrs`（约定如 `task.backlog`）→
  指标；健康/状态由消费端从事件流派生。

### 3.3 发射侧适配（skiff 仓库）

- `runtime/host/src/host/telemetry.rs`：`config.topics` 过滤（:351）、health
  drop 事件条件（:366）、按 topic 的采样优先级表（:718-723）删除；
  `telemetry.queue` drop 事件保留（无条件注入）。
- `runtime/host/src/capability_context/telemetry.rs`：
  - 删除对象形态 emit 分支（:41-75）与 `decode_telemetry_topic`（:263-274）——
    std 侧从未暴露对象形态，属于死面；
  - restricted diagnostic（:89）适配 `telemetry_event()` 新签名。
- `runtime/host/src/capability_context/actor.rs:591`、`host/control_plane.rs:191,
  206`、`host/request_supervisor.rs:136,161,186,221,292,341`：`telemetry_event`
  / `emit_trace` 调用去 topic 参数。
- `runtime/host/src/telemetry.rs`：`telemetry_event()` 签名去 topic。
- `router/src/telemetry.rs`：topic 编解码（:101-127）删除，事件构造适配；
  `backlog_metric_event`（:464）保持（无 topic，name 语义不变）。
- `std/log.skiff`、`std/telemetry.skiff`：**不变**（三参形态，log 事件）。

## 4. 文件 sink（默认落点）

### 4.1 三态

| 配置 | 行为 |
| --- | --- |
| `telemetry.endpoint` 非空 | WS exporter（现状，连接消费端） |
| endpoint 缺省 / 空 | **文件 sink（新默认）** |
| `telemetry.enabled: false` | no-op（现状保留） |

### 4.2 行为

- 每批事件 flush 写 **JSONL**：每行一个事件（与 WS batch 内事件同 schema，
  camelCase 字段一致），消费端可直接复用同一套解析。
- 文件首行 header：`{"type":"fileHeader","protocol":"skiff-telemetry-v1",
  "producerId":"router:dev","source":"router","createdAt":"..."}`；轮转后新文件
  重写 header。
- 路径：默认 `<devHome>/logs/telemetry/<producerId>.jsonl`（router/runtime 各自
  producer，天然按进程分文件）；`telemetry.filePath` 可覆盖。
- 轮转：按大小（默认 64MB）与保留数（默认 8），参数可配置。
- flush 节奏复用 batch 参数（`batchMaxEvents` / `batchMaxBytes` /
  `flushIntervalMs`）。
- drop 统计照旧：drain 时注入 `name=telemetry.queue` 事件。

### 4.3 与 WS 的差异

- 无 register 交互、无连接/重连；文件写入失败即告警（写回自身错误事件或 stderr）。
- 文件是消费端（闭源仓库）的**第二输入面**（tail / 解析），与 WS 协议共享事件
  schema。

## 5. 开源侧改动清单（skiff 仓库）

### M1 — 协议去 topic + 文件 sink

- `runtime/transport/src/protocol/control.rs`（enum/字段删除）
- `runtime/transport/src/protocol/tests.rs`（协议 corpus 更新）
- `runtime/host/src/{telemetry.rs, host/telemetry.rs, capability_context/telemetry.rs,
  capability_context/actor.rs, host/control_plane.rs, host/request_supervisor.rs}`（发射适配）
- `router/src/telemetry.rs`（topic 删除 + sink 选择）
- 文件 sink 实现（router 与 runtime host 各自 producer 的 sink trait 扩展）
- 配置：`scripts/lib/runtime-stack-config.mjs`（`telemetryEndpoint` 校验改为允许
  缺省，新增 sink/filePath 渲染）、`scripts/lib/dev-runtime-paths.mjs`
  （telemetry 路径）
- 测试适配：`router/tests/task_telemetry.rs`、host lib 测试、transport corpus

### M2 — 消费端独立仓库（本地）

- 新建 `/Users/geek/workspace/skiff-telemetry`（独立仓库，闭源，git 初始化本地）
- 迁入 `telemetry/` 现代码（server / main / config / protocol / queryApi /
  mongoStore / redaction / cli）并做去 topic 适配；补充 `/metrics` 聚合与查询、
  告警引擎、dashboard、权限（平台 operator / service owner，需求 §4.6）、
  保留/TTL 策略
- 本地链路切换：stable instance 的 `process.telemetry` 不再由 skiff instance
  管理（`scripts/lib/stack-instance-spec.mjs` 移除 managed 分支），消费端进程由
  独立仓库自管（PM2 ecosystem 或 LaunchAgent），复用 4002 端口与
  `skiff_telemetry` Mongo db；router/runtime 配置 `telemetry.endpoint` 指向其
  WS 地址
- 验证：chat-smoke / two-hosts / telemetry 落库可查

### M3 — skiff 仓库清理与文档收敛

- `git rm telemetry/`（代码已迁入独立仓库；跨仓库分别提交，不设 submodule）
- verify 清理：`scripts/lib/verify-plan.mjs`（telemetry-tests / telemetry-type-check
  任务删除）、`scripts/lib/verify-selector-graph.mjs`（telemetry selector、
  implementation-tests / type-check 引用删除）
- 文档收敛：
  - `doc/reference/observability.md`：topic 模型 → 事件模型 + 文件 sink +
    消费端独立
  - `doc/architecture/observability-requirements.md`：按新模型收敛（生产端
    发射职责与消费端聚合职责重划分，§47 topic 表述更新；§4.2 指标发射条款保留、
    聚合/告警条款标注为消费端职责）
  - `AGENTS.md`（skiff 仓库）：telemetry 相关条目更新
- 本地无 `telemetry.endpoint` 场景验证文件 sink 默认生效

### M4 — 消费端产品化（独立仓库，另行排期）

- 指标聚合（分位数/速率，需求 §4.2）、空遥测告警（§4.1）、阈值告警（§4.5）、
  dashboard、权限面（§4.6）——全部在独立仓库，不涉及开源侧。

## 6. 验收标准（M1–M3）

- `cargo test` transport / router / runtime-host 全绿（协议 corpus 与发射点
  测试更新，task_telemetry 5/5、host lib 429/429）
- 无 endpoint 时：router / runtime 事件落到
  `<devHome>/logs/telemetry/<producerId>.jsonl`，JSON 逐行可解析，std.log 可查
- 有 endpoint 时：WS 链路照旧，指向独立仓库 server，chat-smoke / two-hosts 通过
- `verify` 无 telemetry selector / task 残留，type-check 通过
- skiff 仓库无 `telemetry/` 目录与引用残留（白名单外）

## 7. 非目标

- 不定义告警投递渠道、采样率与成本模型、dashboard 形态（消费端产品决策）
- 不在协议层做 span 配对、指标聚合、状态派生（消费端职责）
- 不做消费端权限的协议扩展（`service_id` + `visibility` 已够，过滤在消费端）
- 不为旧协议 / 旧 topic 加兼容层（语言未发布，不兼容历史）

## 8. 开放问题（不阻塞排期）

- 文件 sink 轮转默认值（64MB/8 个）是否合适
- 独立仓库仓库名（暂定 `skiff-telemetry`）
- 本地 stable 是否同时保留 WS 链路（M2 起 router/runtime endpoint 指向独立
  仓库 server），还是先全量文件 sink、WS 后接
