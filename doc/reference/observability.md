# Skiff Observability Reference

本文负责：Skiff observability 的产品语义，包括 event source、topic、trace / request / span、`std.log`、归属、查询和交付承诺。

本文不负责：telemetry 存储后端 schema、fixture 文件、OpenTelemetry 映射、告警规则、采样算法、queue / timer 可靠调度语义、业务审计或计费事件。

## 定位

Observability 是平台能力，不是业务服务自己接日志数据库 SDK。

它要回答：

- 哪个 service、revision、build、activation 或 runtime 产生了事件。
- 慢在哪里，timeout / cancel / unavailable 发生在哪个 target。
- 某条日志属于哪个 trace、request、span、service 或 runtime。
- 某个 runtime、target、request frame 或 activation 是否健康。

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

runtime、router、gateway、telemetry service 和测试设施都可以是 event source，但职责不同。

runtime 负责从 `std.log`、request frame、span 生命周期、runtime error、health counters 等位置产生事件，自动补充当前 execution context，做基础脱敏、限长、采样和 bounded buffering，并按 router control plane 下发的 telemetry endpoint 导出事件。

router / gateway 是自身运行事件源，也负责转发或下发 telemetry 配置，但不直接承担 telemetry 存储和查询。

`skiff-telemetry` 负责接收、校验、二次脱敏、采样、聚合、写存储，并支撑查询、告警和 UI。

如果 router 没有下发 telemetry 配置，runtime 可以只维护本地 drop / health counters，不外发事件。telemetry 不可用不能改变业务返回值。

## Topics

topic 是平台路由、采样和保留策略维度，不是业务分类机制。

固定 topic 是 `log`、`trace`、`metric`、`health` 和 `debug`，分别覆盖日志、调用链、指标、运行健康和开发 / profiling 诊断。

规则：

- 业务代码不能自定义 topic。
- topic 不表达权限、归属、target、错误类型或业务上下文。
- `audit` 不是 telemetry topic，因为审计需要可靠送达。
- durable queue 也不是 telemetry topic；telemetry queue 是 lossy 观测通道。

## Event Shape

观测事件需要能表达这些维度：

- topic 和 timestamp。
- event source。
- service 归属：service id、revision id、build id、activation identity。
- runtime 归属：runtime id、provider / host 相关摘要。
- 因果链：trace id、request id、client request id、span id、parent span id。
- target：stable target id。
- 内容：level、name、message、attrs、error、duration、dropped counters。

事件字段必须能被脱敏和限长。secret、完整 prompt、完整 external raw payload、完整文件内容默认不得进入 telemetry。

runtime error 可以携带诊断帧。帧应引用当前 build / assembly 内的 source id，并依赖事件上的 build id 或 assembly identity 回查 source map；telemetry 不保存源码全文。

本文不复制 fixture schema。共享协议 fixture 留在 `../architecture/fixtures/observability-minimal.json`，由 router、runtime 和 telemetry 测试复用。

## Trace, Request And Span

`traceId`、`requestId` 和 `spanId` 是不同层次的标识。`traceId` 表示跨 target 的因果链；`requestId` 表示一次内部 transport execution frame 的配对 id；`clientRequestId` 只记录客户端业务 payload 中的请求标识；`spanId` 表示 trace 内一个可计时节点。

要求：

- request frame 事件必须能关联到 stable target id、service revision、runtime activation 和 trace / span。
- trace 可以是 event-only；第一版 telemetry 不强制 start / end 成对。
- 长时间业务 run 应使用业务 durable id，例如 run id、thread id 或 tool call id，而不是 request id。

## std.log

`std.log` 是语言标准库日志入口，不是数据库客户端，也不是可靠消息通道。

语义：

- `std.log.*` 是 runtime telemetry intrinsic。
- 它只产生 `log` topic 的 best-effort 事件。
- runtime 自动补充 request frame、trace、span、service、runtime 和 target context。
- attrs 应是可脱敏、可限长的结构化数据。
- telemetry 不可用、队列满或发送失败不能影响业务返回值。

`std.log` 的 effect 是 telemetry write。它不参与普通 external effect 冲突判定，也不能作为业务正确性依赖。

业务代码不应把 full prompt、secret、原始外部 payload 或大对象内容直接写进 log。需要排查时，应记录摘要、id、长度、错误 code、有限片段或脱敏结构。

## Ownership

归属由事件字段表达，不由 topic 表达。

因果归属包括 trace id、request id、span id、parent span id 和 client request id。运行归属包括 source、runtime id、provider id / revision、build id 和 activation identity。service / 权限归属包括 service id、revision id、stable target id、actor ref 摘要和可选 tenant id。

`userId` 不作为平台硬编码字段。业务身份应先映射成 ActorRef 或等价摘要，观测事件只记录可审计且可脱敏的摘要。

## Query

查询入口围绕结构化字段，而不是日志文件。

常用查询维度：

- time range。
- topic。
- trace id、request id、span id。
- service id。
- revision id、build id、activation identity。
- runtime id。
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
- actor put / remove、method call、owner lease renewal、spawn submit / execution 可以产生 trace / metric / health。
- queue wait、claim batch、lease renew、deadline miss、cancel、timeout、failure 可以产生 trace / metric / log。
- runtime request start / end / error / cancel 是 request frame 的基础 trace 事件。

这些事件用于诊断和告警；真正的业务状态仍在 service-owned database、queue store、timer store 或业务 durable state 中。

## 当前不支持

- telemetry 承载可靠业务事件。
- 自定义 topic。
- audit / billing / outbox 走 lossy telemetry。
- 业务代码直接连接 telemetry storage。
- 通过 topic 表达权限、租户、actor 或 target。
- 把 client request id 当作内部 request id。
- 把 request id 当作长时间业务 run id。
- 在 telemetry 中保存源码全文、完整 prompt、secret 或完整外部 raw payload。

## 未定问题

- OpenTelemetry 兼容映射。
- 错误聚合算法。
- 告警规则。
- 长期采样策略。
- 生产存储后端 schema 和 retention。
- queue / timer / actor 的完整观测字段集合。
- profile / debug 数据的权限、保留和采样策略。
