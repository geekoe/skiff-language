# Skiff Observability Reference

本文负责：Skiff observability 的产品语义，包括 event source、事件模型（事件自描述，形态识别归消费端）、trace / request / span、`std.log`、归属、查询和交付承诺。

本文不负责：telemetry 存储后端 schema、fixture 文件、OpenTelemetry 映射、告警规则、采样算法、queue / timer 可靠调度语义、业务审计或计费事件。消费端（存储、聚合、告警、dashboard、权限）在独立仓库 `skiff-telemetry` 实现，不在本仓库。

## 定位

Observability 是平台能力，不是业务服务自己接日志数据库 SDK。

它要回答：

- 哪个 service、revision、exact deployment `buildId` 和 runtime session 产生了事件。
- 慢在哪里，timeout / cancel / unavailable 发生在哪个 target。
- 某条日志属于哪个 trace、request、span、service 或 runtime。
- 某个 runtime、target、request frame 或 deployment image 是否健康。

Observability 只承载可丢失的运行观测数据。它不能承载业务正确性依赖的数据。

不能走 telemetry 的内容：

- 业务事件。
- 审计。
- 扣费。
- queue / cron / task。
- outbox。
- 必须送达的通知。
- 跨进程 cache invalidation。
- 丢失后会改变业务正确性的任何数据。

可靠业务事件需要 durable queue / event / outbox 等单独能力。

## Event Sources

runtime、router、gateway 和测试设施都可以是 event source，但职责不同；telemetry 消费端（独立仓库）只负责接收与消费，不产生事件。

runtime 负责从 `std.log`、request frame、span 生命周期、runtime error、health counters 等位置产生事件，自动补充当前 execution context，做基础脱敏、限长、采样和 bounded buffering，并按 router control plane 下发的 telemetry 配置导出事件：`telemetry.endpoint` 非空走 WS exporter，缺省落文件 sink（见「File Sink」）。

router / gateway 是自身运行事件源，也负责转发或下发 telemetry 配置，但不直接承担 telemetry 存储和查询。

`skiff-telemetry`（独立仓库，闭源）负责接收、校验、二次脱敏、采样、聚合、写存储，并支撑查询、告警和 UI。它通过 WS 与文件双输入面对接生产端：WS 批量事件与 JSONL 文件行共享同一事件 schema；事件自描述，形态识别（日志 / span / 指标 / 状态）由它承担，不 import 开源内部实现。

`telemetry.enabled: false` 时生产端 no-op；telemetry 不可用不能改变业务返回值。

## Event Model

协议只管生产：事件自描述，协议中没有 topic 维度。形态识别（日志 / span / 指标 / 状态）是消费端职责，生产端只填写字段（`level`、`name`、`message`、`attrs`、`durationMs`、span 关联等），不声明事件形态。

规则：

- 事件不按 topic 分类；`audit` 不走 telemetry，因为审计需要可靠送达。
- durable queue 也不是 telemetry；telemetry queue 是 lossy 观测通道。
- 事件字段固定，业务代码不能扩展字段（协议拒绝未知字段）。
- 事件 schema 见共享协议 fixture（`../architecture/fixtures/observability-minimal.json`），生产端（router / runtime）测试与消费端仓库复用。
- **事件形态由两个生产端接口约束，互不混淆**：
  - 业务日志：`std.log.*`（`level` + `message`，无 `name`），以及 Runtime 的统一异常兜底。
    二者共用 runtime 内唯一的 `business_log_event` 构造入口；普通平台 Rust 事件不可达，只有
    payload-free 的 Runtime exception DTO 可以进入该兜底。
  - 平台事件：router 控制面 / runtime host 等平台代码统一走 `PlatformEvent` 接口
    （`name`，可带 `attrs` / `error` / `durationMs`），**不得携带 `level` / `message`**。
  - 底层共用同一 `TelemetryEvent` 管线，但消费端按形态分流：`level` + `message` 存在即业务日志
    （`/logs`），`name` 存在即平台事件（`/records`）。平台事件带 `level` 是协议违约，消费端拒绝入库。

## Event Shape

观测事件需要能表达这些维度：

- timestamp 与自描述事件字段。
- event source。
- service 归属：service id、revision id、exact deployment `buildId`。
- runtime 归属：`runtimeId`、`runtimeSessionId`、provider / host 相关摘要。
- 因果链：trace id、request id、client request id、span id、parent span id，以及错误事件的 error id。
- target：stable target id。
- 内容：level、name、message、attrs、error、duration、dropped counters。

事件字段必须能被脱敏和限长。secret、完整 prompt、完整 external raw payload、完整文件内容默认不得进入 telemetry。

execution 事件上的 `buildId` 必须是本次执行 pin 住的 exact ServiceDeployment build id，不是
Package build id、release pointer 的当前值、deployment activation identity、runtime activation identity 或
assembly generation。`runtimeSessionId` 表示本次物理连接的 session epoch，与稳定的 `runtimeId` /
replica identity 分开。Actor activation id 和 client socket generation 只能作为明确 Actor / socket
生命周期事件的 scoped attrs，不能
代替通用执行归属。

runtime error 可以携带当前service的完整诊断帧。帧应引用当前 deployment image 内的 source id，
并依赖事件上的 exact deployment `buildId` 回查 source map；telemetry 不保存源码全文。最初throw、跨service转换和
后续传播使用同一`traceId`并以`errorId`关联，使每一跳各自记录的本地栈能组成同一错误因果链。

完整本地栈只进入受限telemetry/log。service error response不能携带私有source id、源码路径、函数名或原始
私有错误字段；caller只能得到当前request的新异常栈和一帧包含service/operation/errorId等安全字段的
remote-boundary诊断。

每个新异常在 Runtime 首次分配 `errorId` 的同一边界自动产生一条 `error` 业务日志，因此异常即使随后被
Skiff `catch` 捕获仍可观察。`catch`、普通传播、`rethrow`、跨 service import 和 Host terminal 不再次记录；
它们保留原 correlation，从而不会因跨层传播重复。兜底日志使用固定 message，并只携带安全类型 identity、
identity hash、有限 reason 分类、可选 callable，以及自动附加的 trace / request / target 上下文。它不接受
异常 heap value、任意 payload、用户编写的 message/reason、源码或开放 attrs；telemetry 失败也不改变执行结果。

本文不复制 fixture schema。共享协议 fixture 留在 `../architecture/fixtures/observability-minimal.json`，由 router、runtime 测试与消费端仓库复用。

## File Sink（默认落点）

无 `telemetry.endpoint` 时，生产端默认写文件而非外发：

- 每批事件 flush 写 **JSONL**：每行一个事件，与 WS batch 内事件同 schema（camelCase 字段一致），消费端可直接复用同一套解析。
- 文件首行 header：`{"type":"fileHeader","protocol":"skiff-telemetry-v1","producerId":"...","source":"...","createdAt":"..."}`；轮转后新文件重写 header。
- 默认路径 `<devHome>/logs/telemetry/<producerId>.jsonl`（router / runtime 各自 producer，天然按进程分文件）；`telemetry.filePath` 可覆盖。
- 轮转：按大小（默认 64MB）与保留数（默认 8），参数可配置。
- flush 节奏复用 batch 参数（`batchMaxEvents` / `batchMaxBytes` / `flushIntervalMs`）。
- 与 WS 的差异：无 register 交互、无连接 / 重连；文件写入失败即告警（写回自身错误事件或 stderr）。文件是消费端（独立仓库）的第二输入面（tail / 解析）。

## Trace, Request And Span

`traceId`、`requestId` 和 `spanId` 是不同层次的标识。`traceId` 表示跨 target 的因果链；`requestId` 表示一次内部 transport execution frame 的配对 id；`clientRequestId` 只记录客户端业务 payload 中的请求标识；`spanId` 表示 trace 内一个可计时节点。

要求：

- request frame 事件必须能关联到 stable target id、service revision、exact deployment
  `buildId`、`runtimeId`、`runtimeSessionId` 和 trace / span。
- trace 可以是 event-only；第一版 telemetry 不强制 start / end 成对。
- 长时间业务 run 应使用业务 durable id，例如 run id、thread id 或 tool call id，而不是 request id。

## std.log

`std.log` 是语言标准库日志入口，不是数据库客户端，也不是可靠消息通道。

语义：

- `std.log.*` 是业务代码唯一可调用的 runtime telemetry intrinsic；Runtime 异常兜底是不可由业务代码调用的
  内部入口。
- 它产生 best-effort 日志事件（`message` / `level` 形态由消费端识别，不声明 topic）。
- `level` / `message` 只属于业务日志形态：平台事件不得携带（见「Event Model」）。除封闭的异常兜底 DTO
  外，平台代码没有构造业务日志的接口。
- runtime 自动补充 request frame、trace、span、service、exact deployment `buildId`、`runtimeId`、
  `runtimeSessionId` 和 target context。
- attrs 应是可脱敏、可限长的结构化数据。
- telemetry 不可用、队列满或发送失败不能影响业务返回值。

`std.log` 的 effect 是 telemetry write。它不参与普通 external effect 冲突判定，也不能作为业务正确性依赖。

业务代码不应把 full prompt、secret、原始外部 payload 或大对象内容直接写进 log。需要排查时，应记录摘要、id、长度、错误 code、有限片段或脱敏结构。

## Ownership

归属由事件字段表达；事件模型没有 topic 维度。

因果归属包括 trace id、request id、span id、parent span id 和 client request id。运行归属包括 source、
`runtimeId` / `runtimeSessionId`、provider id / revision 和 exact deployment `buildId`。通用执行归属不使用
deployment activation identity、runtime activation identity 或 assembly generation。service / 权限归属包括
service id、revision id、stable target id、actor ref 摘要和可选 tenant id。

`userId` 不作为平台硬编码字段。业务身份应先映射成actor句柄identity或等价摘要，观测事件只记录可审计且可脱敏的摘要。

## Query

查询入口围绕结构化字段，而不是日志文件。

常用查询维度：

- time range。
- trace id、request id、span id。
- service id。
- revision id、exact deployment `buildId`。
- `runtimeId`、`runtimeSessionId`。
- target。
- level。
- error code。
- actor kind、actor subject id、tenant id 摘要。

查询产品需要支持两类路径：从 service / target / time range 出发找错误、慢请求和异常指标；从 trace / request / actor 摘要出发回看因果链和相关日志。

CLI、UI 和测试查询都应复用同一结构化语义。日志文件路径或 runtime 本地 buffer 只能是降级诊断，不是主要查询模型。

## Delivery Promise

Skiff telemetry 是 best-effort。

承诺：

- 不保证 exactly-once。
- 不保证 at-least-once。
- 不保证跨进程全局顺序。
- 不因 telemetry 不可用阻塞业务。
- runtime 必须使用 bounded buffer，不能无限缓存。
- producer 出口必须做基础脱敏和限长。
- drop、sample、export failure 需要通过 counters / health 事件可观察。

这意味着业务代码不能依赖日志是否送达来推进状态、扣费、审计或唤醒其他流程。

## 与 Work / Data 的关系

DB、actor、queue、timer 和 runtime request 都应产生观测事件，但观测事件不替代它们的状态。

示例归属边界：

- DB 慢查询、constraint error、transaction conflict 可以产生 trace / metric / log。
- actor put / remove、method call、owner lease renewal、dispatch submit / execution 可以产生 trace / metric / health。
- queue wait、claim batch、lease renew、deadline miss、cancel、timeout、failure 可以产生 trace / metric / log。
- runtime request start / end / error / cancel 是 request frame 的基础 trace 事件。
- deployment image load / link / verify / cache / rejection、VM fiber / owner transfer 和 managed heap / GC
  应产生可按 exact `buildId` 与 `runtimeSessionId` 聚合的 metric / health 事件。

这些事件用于诊断和告警；真正的业务状态仍在 service-owned database、queue store、timer store 或业务 durable state 中。

## 当前不支持

- telemetry 承载可靠业务事件。
- 自定义事件字段（协议拒绝未知字段；形态识别归消费端，不在协议层表达）。
- audit / billing / outbox 走 lossy telemetry。
- 业务代码直接连接 telemetry storage。
- 通过事件字段表达权限、租户、actor 或 target（过滤由消费端按 `visibility` / `serviceId` 承担）。
- 把 client request id 当作内部 request id。
- 把 request id 当作长时间业务 run id。
- 用 deployment activation identity、runtime activation identity 或 assembly generation 代替 exact deployment `buildId` 和
  `runtimeSessionId` 表达通用执行归属。
- 在 telemetry 中保存源码全文、完整 prompt、secret 或完整外部 raw payload。

## 未定问题

- OpenTelemetry 兼容映射。
- 错误聚合算法。
- 告警规则。
- 长期采样策略。
- 生产存储后端 schema 和 retention。
- queue / timer / actor 的完整观测字段集合。
- profile / debug 数据的权限、保留和采样策略。
