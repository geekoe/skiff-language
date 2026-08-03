# Skiff Queue And Scheduling Reference

本文负责：Skiff 平台持久调度的能力边界——durable queue item / policy / claim / lease /
timer、同步请求排队、cancel / timeout，以及它们和 runtime request 的关系。

本文不负责：actor、`dispatch` 后台提交、service-owned database、queue store 物理实现、
wire envelope、具体 manifest 字段、观测 schema。

## 定位

durable queue / timer 是平台调度机制，用于同步请求、后台任务、业务实体事件和未来唤醒。
它与 actor 不互相替代：actor 负责可寻址对象和方法路由；durable queue / timer 负责持久排队、
跨 runtime 调度、deadline、lease、cancel intent 和 future wake。

长时业务的关键事实必须落到 service-owned database 或业务自己的 durable state。runtime
request frame 和 telemetry 都不能成为业务正确性的唯一来源。

## Durable Queue Model

Durable queue 是平台底层调度机制。service task queue 和 entity queue 是包装，不是两套
基础设施。

queue item 表达一次可调度执行：

- queue：调度和并发限制的主要命名空间。
- service id / service version：冻结执行语义线。
- target：claim 后要调用的 handler / operation id。
- traffic class：同步或异步。
- key：可选业务实体或分片 key，用于 key concurrency。
- payload、deadline、trace、status、attempts、lease、cancel / timeout 标记等
  字段用于承载执行、观测和恢复所需的调度事实；平台不提供业务 dedupe key，两次独立
  提交永远是两个独立 item。

payload 是 owner-internal recoverable boundary。入队前必须证明 payload 是可恢复值；plain data、可恢复 nominal
object、durable native handle 和 `carrier = Local` 且 self payload 全可恢复的 `any I` 可以进入；callback、`Stream`、
transaction、live connection、file descriptor、无 durable adapter 的 native handle、`carrier = Remote` 的 `any I` 等
native/request-local resource fail closed。encode 失败时不得创建 queue item。完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

queue item 成功 claim 后进入 leased，只是开始一次 execution attempt，不构成 terminal
交付。普通 failure、cancel、timeout 不自动重跑；lease expiry 或无法证明 attempt 已可靠
settlement 时，平台按 at-least-once 恢复规则为同一 item 生成新的 attempt（平台 bounded
backoff）。业务需要重试或继续推进时显式创建新 item 或 timer。

## Queue Policy

queue + target 需要声明 policy。核心限制：

- `concurrency` 限制该 queue + target 当前最多 leased item。
- `keyConcurrency` 限制同一 key 当前最多 leased item；默认同 key 串行。
- `leaseTtl` 定义 runtime 持有执行权的续租窗口。

并发占用从 item leased 开始，到 item terminal 后释放。它不是单次 claim 的瞬时限速。

key concurrency 的冲突作用域至少覆盖 service id + queue + key。默认不包含 service
version 或 build id，否则同一业务实体在发布切换时可能被两个版本并发处理。

调度权重分两层：平台层看 traffic class 和 service weight，同步可以更高权重但不能绝对
优先；业务权重只在同一 service 的业务队列内部排序，不能抢占其他 service 的平台份额。

## Claim And Lease

scheduler 是 queue 的主动消费者（可与 Runtime / Router 共进程部署）。它根据自身 capacity、
已 active 数量、claim batch 上限、queue policy、key concurrency、deadline、visibility 和
调度权重 claim item。

claim 成功后，item 进入 leased，写入 lease owner / id / expiry，attempts 增加并开始一次
execution attempt。该 item 计入 active count，直到 terminal。

runtime 必须在 lease 过期前 renew。所有 completion、failure、cancel、timeout 或
heartbeat 写入都必须携带当前 lease id；store 拒绝旧 lease id 写入，作为 fencing。

lease 过期后，settlement 与 recovery 在 store authority time 上竞争同一个 CAS：settlement
成功则 item terminal；recovery 成功则原子清除 active lease，为同一 item 生成新 attempt
（at-least-once），不是收敛为 terminal failure。旧 lease 的后续写入被拒绝。外部副作用可能
已经发生，业务必须用幂等、版本号或补偿状态处理。

## Timer

TimerStore 表达未来唤醒。timer 到期后只投递 queue item，不直接调用 runtime。

- timer 在提交时冻结完整 execution image：service owner、service version、exact build /
  assembly identity、deployment revision 与配置 snapshot，以及 target 和 payload schema。
- 到期执行使用提交时冻结的 build / image，不得在投递时按 service version 重新解析
  current build 或 latest。
- timer 不保证严格准点，只保证 `fireAt` 之后最终尝试投递。
- timer 不应主动早于 `fireAt` 投递，除非系统时钟边界误差不可避免。
- 第一版只需要 one-shot timer；周期任务由 handler 执行后重新 schedule。
- 修改 fire time、payload 或 version 应取消旧 timer，再创建新 timer。
- timer fire 与 queue item 投递应原子提交，重复 scanner 通过 item identity 与 status CAS
  收敛，不提供业务 dedupe key。

timer 可用于 async task timeout、sleep / wake、等待实体超时唤醒、业务补偿 / 重试延迟和
系统维护工作。

## Sync Requests

同步请求也进入 durable queue。它仍然是一段普通 runtime request execution，只是 caller /
gateway 在等待结果。

同步请求特征：traffic class 为 sync；不提供业务 dedupe key；max queue wait 和 deadline
通常更短；调度权重通常高于 async 但不是绝对优先。caller wait timeout、queue wait timeout
和 execution deadline 是三个不同概念。caller 断开或等待超时默认不必然取消 item；是否
cancel、detach 或 discard 由 operation policy 决定。只有显式允许 detach 的 operation，
超时后才可转成后台 async work。

同步请求完成后，runtime 把结果写回等待中的 gateway / router。caller 已断开时，结果按
policy 处理。

## Cancel And Timeout

timeout 分为 queue wait timeout 和 execution timeout。

pending item 还没有 runtime owner，可以直接 terminal cancelled / timed out，并释放
queue / key capacity。

leased item cancel 或 execution timeout 先写 non-terminal 标记，不释放 lease、queue
concurrency 或 key concurrency。runtime 通过 heartbeat、cancel channel 或 await
boundary 观察取消；handler cooperative stop 后写入 terminal cancelled、timed out 或
failed。若无法触达 runtime，item 最终由自然结束或 lease expiry 后的 at-least-once
recovery 处理。

cancel 不是撤销。外部请求、shell command、third-party API 或 LLM 输出可能已经部分执行。
业务必须把关键意图落库，并用幂等和补偿处理结果。

普通 failure、cancel、timeout 不自动生成下一次执行；lease expiry 或无法证明 settlement
的 attempt 由平台恢复为同一 item 的新 attempt（at-least-once）。需要业务级重试或继续
推进时，业务显式创建新 item 或 schedule 新 timer。

## Runtime Request Relation

queue item 被 claim 后才创建 runtime request frame：item leased，按提交时冻结的 execution
image（含 exact build id）创建 `request.start`，runtime 执行并产生 response / stream /
error，最后按当前 lease id ack queue item 为 completed、failed、cancelled 或 timed out。

`requestId` 只用于一次 transport execution frame 配对，不是长时间业务 run 的主键。业务
长流程应使用自己的 durable id，例如 run id、thread id 或 tool call id。

server stream 也只是一次 runtime request 输出流。客户端断线后仍需观察的重要输出，应进入
durable event / outbox 或后续订阅机制；queue 本身不承担业务事件日志。

## Queue Exposure Boundary

第一版不向业务源码暴露 queue、partition、consumer、ack、lease 或 visibility。queue 是平台
底层调度机制，不是第二套用户可见 callable 语义；用户通过 `dispatch` 表达式与 task status /
cancel surface 操作逻辑任务。立即与延迟 detached call、削峰、fairness、并发限制、backlog、
actor-method future wake 和 Runtime replica 之间的位置透明派发都不需要公开 queue。

未来只有 queue 本身成为业务地址和协议时才应设计独立用户 surface。届时 work queue 与
event stream 必须分开设计：work queue 表达一条 message 由一个竞争 consumer 处理，定义
ack / lease / redelivery；event stream / topic 表达多订阅者、retention、offset 与 replay，
不能因底层都使用 log 就伪装成 work queue。公开 queue item 不应偷偷携带
`ExecutableIdentity` 并退化成另一种 `dispatch`；`dispatch` 也不暴露 raw queue name 或
manual ack。

## 当前不支持

- 暴露 queue 对象给业务代码直接操作。
- 普通 failure、cancel、timeout 后自动重试同一个 queue item（lease-loss 的 at-least-once
  recovery 除外）。
- exactly-once execution 或自动补偿。
- timer 严格准点。
- 通过 queue `visibleAt` 表达长期 delay。
- queue / topic / telemetry 承载可靠业务事件。

## 未定问题

- queue policy 在 service config / manifest 中的声明格式。
- runtime capacity、claim API、caller waiter store、result TTL 和 caller gone 默认
  policy。
- weighted fair scheduling 的第一版算法。
- queue payload codec、大 payload store、schema identity 和兼容策略；底线是 payload 必须满足可恢复值 contract。
- queue / timer 的生产索引、并发 claim、session / transaction API。
