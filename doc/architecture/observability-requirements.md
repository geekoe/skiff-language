# 可观测性需求（Observability Requirements）

日期：2026-08-04
状态：需求草案（供实现与验收对齐）

## 1. 背景与动机

2026-08-04 本地稳定实例出现 router task-control scheduler 热循环：一次 `wake()`
后，`wait_for_cycle` 因 watch receiver 版本未推进而永远立即返回，scheduler 以
最大速度空转，对 `skiff-router.tasks` 持续产生约 550 updates/s、1000 queries/s
的负载，mongod 与 router CPU 均被明显占用，其它所有 DB 写延迟被放大。

该问题在平台内没有任何可观测信号：

- `/__router/health` 的 task 计数全部正常（backlog=0、renewing=0、pending=0、
  settlementsSucceeded 正常），因为热循环不改变任何任务状态；
- telemetry 服务连接正常但 `insertedEvents=0`，`/logs`、`/traces` 为空，因为
  普通 HTTP、service call、DB 操作不产生任何 trace / metric / log；
- service-db 没有 per-operation 计数或延迟，`w: majority` 写 ~20ms 的问题不可见；
- `std.log` 在导出 service 方法内会导致 service call 边界不可用（UnknownEffect），
  服务无法用平台日志 API 自插桩。

本需求的目标是让这类问题在发生后能被及时发现，并在下一次回归前被测试或指标拦住。

## 2. 目标

- 平台级：router / runtime / service-db 的健康、速率与延迟可观测，异常负载
  （热循环、写放大、背压、overload）有明确信号。
- Service owner 级：每个 service 能对自己发出的请求、服务调用、DB 操作和业务
  阶段（如 first-token 延迟）做链路级诊断。
- 同一事件双视角：同一份数据既可按平台聚合（总量、p95、速率），也可按
  serviceId / 业务属性下钻。
- 观测不承担业务正确性：best-effort、bounded queue、不得阻塞业务路径。

## 3. 分层与归属

### 3.1 平台级（Skiff 基础设施 owner）

面向 router / runtime / service-db 的运营者，以及 telemetry 消费端（独立仓库
`skiff-telemetry`）自身的运营者，回答：
“平台是否健康、负载是否异常、请求是否被平台环节拖慢”。

### 3.2 Service owner 级（agine / aihub / codex-relay 等）

面向每个 service 的开发者，回答：
“我的请求为什么慢、慢在哪一段、是不是平台或依赖服务的问题”。

### 3.3 双视角属性

以下两类事件必须同时支持平台聚合与 service 归属：

- service call 延迟：平台看总量与分位数，owner 看自己发起的调用明细；
- DB 操作延迟：平台看每集合 / 每服务的总量与分位数，owner 看自己请求内的操作。

### 3.4 运行归属

执行事件的运行归属必须同时包含 exact deployment `buildId`、`runtimeId` 和
`runtimeSessionId`（session epoch）。`buildId` 是本次执行已 pin 的 ServiceDeployment build id，不是
Package build id 或 release pointer 的当前值。通用运行归属不得使用 deployment activation identity、
runtime activation identity 或 assembly generation。Actor activation id 和 client socket generation 可保留在
语义明确的 Actor / socket 生命周期事件中，但不能代替上述归属字段。

### 3.5 生产 / 消费职责划分

- 生产端（skiff 仓库：router / runtime）负责事件发射、基础脱敏 / 限长 / 采样、
  bounded buffering，以及默认文件落盘（无 `telemetry.endpoint` 时 JSONL 写
  `<devHome>/logs/telemetry/<producerId>.jsonl`）。
- 消费端（独立仓库 `skiff-telemetry`，闭源）负责接收、存储、聚合、查询、告警、
  dashboard 与权限，通过 WS 与文件双输入面对接生产端。
- 协议只管生产：事件自描述，形态识别（日志 / span / 指标 / 状态）归消费端；
  span 配对、指标聚合、状态派生都是消费端职责。

## 4. 需求明细

### 4.1 telemetry 管道必须“真的在工作”

- telemetry 消费端（独立仓库）的注册 / 批量上报 / Mongo 落库必须可验证：
  `/health` 暴露 `insertedEvents`、`acceptedBatches`、`rejectedMessages`、drop
  计数。
- 生产端（router / runtime）与 telemetry server 断开、批量失败、队列 drop 必须
  产生 warn 事件，并在健康端点可见。
- **空遥测本身就是信号**：连续 N 分钟（默认 5）没有任何事件时产生 warn，
  避免“管道静默失效但看起来正常”（空窗检测与告警属消费端职责）。
- 遥测只依赖平台自带 producer；服务代码不直接写 telemetry server。

### 4.2 指标（metrics）

本节是发射条款：生产端负责发射计数与延迟事件；速率、分位数等聚合计算与查询
由消费端（独立仓库）承担。

#### 4.2.1 task-control scheduler（本次热循环的直接指标）

至少包含以下计数器与速率：

- `scheduler.cycles`（总轮数、每秒轮数）
- `scheduler.wakes`（wake 次数，区分合并前原始调用与合并后消费）
- `scheduler.scan_cycles` / `scheduler.claims` / `scheduler.claim_rejected`
- `scheduler.renews` / `scheduler.renew_rejected`
- `scheduler.recoveries`
- `scheduler.batch_saturated`（scan 结果达到 batch_limit 的次数，用于发现
  batch 太小或 backlog 堆积）
- `scheduler.last_cycle_duration_ms`
- `tasks.backlog` 按 state 的计数与变化速率（已有累计值，需要速率视角）

要求：生产端发射累计计数与相关事件；速率 / 分位数等聚合计算与查询由消费端
（独立仓库）承担。

#### 4.2.2 service-db

- 按操作类型（find / insert / update / delete / transaction）统计次数、p50 /
  p95 / p99 延迟；
- 按 service / package / logical collection 归属；
- transaction 单独统计（开始、提交、中止、冲突重试、commit 延迟）；
- 暴露当前 Mongo 连接配置中的 write concern / read concern / retry 设置，
  便于识别 majority 写等配置性延迟；
- 慢操作（默认 > 100ms）自动携带 request / service / 操作上下文。

#### 4.2.3 router HTTP 与 service call

- HTTP：按 entry / service / path 统计 request 数、延迟分位数、terminal source
  分布（已有 terminal 计数，补延迟与按 entry 聚合）；
- service call：按 (caller service, target service, operation) 统计次数与延迟
  分位数；
- overload / backpressure / pending 计数已有，补充持续时间和速率。

#### 4.2.4 runtime

- service call 边界 dispatch 延迟（进入 provider 的固定成本）；
- VM fiber 的 create / runnable / park / resume / complete 计数、ready queue 深度与 park 时长；
- 跨 service / Actor / callback owner transfer 的次数、耗时和 terminal / rejection 分类；
- managed heap 的 allocation bytes / objects、current / peak bytes、limit rejection，以及 GC 次数、
  pause 耗时和 reclaimed bytes；
- deployment image load / link / verify 分阶段耗时、cache hit / miss / wait，load rejection 按
  missing / decode / structural validation / link / semantic verification / resource limit / timeout 分类。

上述指标必须可按 exact deployment `buildId`、`runtimeId` 和 `runtimeSessionId` 聚合；Actor
activation id 或 socket generation 只在相应 owner / socket 事件中作 scoped 维度。

#### 4.2.5 Mongo 哨兵

- 定期（默认 10s）对服务 DB 执行一次 canary 读与一次 canary 写（w:1），
  记录延迟；
- 统计每集合操作率，超过阈值（默认全库 500 ops/s 或写 200 ops/s）产生 warn；
- 哨兵本身不进入业务路径，失败只告警。

### 4.3 追踪（traces）

- 每个 HTTP request 与 service call 都产生 trace，包含 start / end、duration、
  serviceId、operation、requestId；跨 router → runtime → service call → DB
  传播同一 trace id。
- service call 的子 span 记录 provider owner transfer、参数 / 返回物化耗时，并携带
  provider 的 exact deployment `buildId`、`runtimeId` 与 `runtimeSessionId`。
- DB 操作默认不单独建 span（避免噪声），由延迟直方图覆盖；慢操作
  （> 阈值）挂到当前 span。
- 服务可向 span 附加业务属性（chatId、runId、messageSeq），属性必须可脱敏、
  限长。
- 本地 query API（`/traces`）可按 traceId、serviceId、时间范围查询
  （消费端能力）。

### 4.4 日志（logs）

- **`std.log` 必须可用于导出 service 方法**：当前编译器把含 `std.log` 的
  service-call 边界标记为 UnknownEffect 导致不可用，属于阻断性缺陷，优先级
  最高。
- 日志级别 debug / info / warn / error；结构化 attrs；
- 自动附加 request / trace / service / exact deployment `buildId` / `runtimeId` /
  `runtimeSessionId` 上下文；
- 默认限长、脱敏；禁止记录完整 prompt、secret、原始外部 payload；
- 平台自身日志（router / runtime）与 service 日志同协议、同查询入口。

### 4.5 健康与自检

- `/__router/health` 对关键计数提供**速率**视角（cycles/s、DB ops/s、
  HTTP rps），而不是只有累计值；
- scheduler 健康段：cycle 速率、wake 速率、batch 饱和次数、最近 cycle 耗时；
- 阈值告警（warn 级别 telemetry + health 字段；告警引擎为消费端职责，生产端
  只提供事件与健康字段）：
  - scheduler cycles/s 持续 > 20；
  - DB 写 p95 > 50ms（或 canary 写 > 50ms）；
  - 全库操作率 > 阈值；
  - telemetry 空窗 > 5 分钟；
  - task backlog 增长速率异常。
- 阈值必须可配置，默认值适合本地 dev，release 可另行设置。

### 4.6 访问与保留

- 平台级指标 / 日志只对平台 operator 可见；service owner 只能查询自己
  serviceId 范围内的 logs / traces / metrics，以及与自己相关的 service call
  明细（权限过滤为消费端职责）。
- telemetry 数据有保留 / TTL 策略（消费端策略，独立仓库实现）；本地 dev 至少
  保留 24h，release 按容量配置。

## 5. 验收标准

以本次热循环为反例，修复后应满足：

- 在热循环复现时，`/__router/health` 的 `scheduler.cycles/s` 与
  `scheduler.batch_saturated` 明显异常，即使 backlog 为 0；
- 消费端 telemetry `/metrics`（或等价查询）能看到 service-db 写 p95 升高与
  `skiff-router.tasks` 操作率异常；
- service owner 能通过 trace 看到一次 `/chat/send` 的完整阶段耗时；
- runtime 查询能按 exact deployment `buildId` 和 `runtimeSessionId` 分解 image load / link / verify /
  cache / rejection、fiber / owner transfer 与 heap / GC 指标；
- `std.log` 可以出现在导出 service 方法中并被 `/logs` 查询到；
- 空遥测窗口会产生 warn。

## 6. 非目标

- 本需求不定义告警投递渠道（邮件 / IM / on-call）；
- 本需求不要求对每次 DB 操作都建 span；
- 本需求不把遥测变成业务正确性依赖；
- 本需求不定义 release 形态的采样率与成本模型，但实现必须支持采样与限流。
