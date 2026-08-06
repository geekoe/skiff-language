# Router Rust Migration C-process-lifecycle：停机顺序冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；PR 0b 后随 installed components 扩展）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §5.4
  （C-process-lifecycle 冻结顺序）、§3.2（RouterSupervisor owner）、§3.6
  （session barrier）、§6.2(2)、§7 E-session/E-cutover。
- 父批次：`doc/implementation/router-rust-migration-batch-3.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-session-leaf.md`。
- 同链契约：`router-rust-migration-c-model-registration-contract.md`、
  `router-rust-migration-c-session-contract.md`；
  listener 机制：`router-rust-migration-c-net-contract.md`。

冲突时以权威设计为准；本文件只冻结契约，不写 production。

## 1. 范围

冻结 Router 进程 graceful shutdown 的**总顺序、每步总 deadline、超时
fail-stop 语义**，以及 composition shutdown test 的扩展规则。C-net 已冻结
listener 层的 stop-accept / graceful_shutdown / deadline abort 机制；本契约
在其上冻结业务 owner 的 drain 顺序，供 PR 0b 之后每次新增 lane 时扩展。

## 2. 停机顺序（冻结，不可跳过、不可重排）

```text
S1 stop public/control admission
S2 stop new activation + reconcile in-flight durable decision
S3 drain HTTP/client WS finalizers
S4 terminal dispatcher/broker/actor pending
S5 release Runtime generation leases
S6 close Runtime sessions via barrier
S7 join blocking loader/tasks/timers
S8 close Mongo
```

| Step | 内容 | 总 deadline（默认） | 超时行为 |
| --- | --- | --- | --- |
| S1 | public HTTP 与 control/runtime listener 停止 accept 新连接；已有连接进入 graceful drain | 5s | 非零退出 / fail-stop |
| S2 | 停止新 activation；in-flight durable decision 按 durable state reconcile（decision 前断连 abort、decision 后读 state reconcile），不新写 pending | 10s | 非零退出 / fail-stop |
| S3 | HTTP 请求 finalizers 与 client WS 连接 finalizers（C-client-lifecycle）完成或到达各自 deadline | 15s | 非零退出 / fail-stop |
| S4 | `RequestDispatcher` / `WebSocketRequestBroker` / actor owners 的 pending 全部 terminal（reservation/tombstone/permit 归零） | 10s | 非零退出 / fail-stop |
| S5 | `RuntimeGenerationPinLedger` 释放全部 Runtime generation leases；release 超时按 client terminal 处理，不静默保留 pin | 10s | 非零退出 / fail-stop |
| S6 | 全部 Runtime session 经 cancellation + close barrier 关闭；consumer 全 ACK 后 directory 删除；barrier ACK 超时或 reserved slot 失效 → fail-stop | 20s | 非零退出 / fail-stop |
| S7 | join blocking loader pool、spawned tasks、timers；shutdown residue 归零 | 15s | 非零退出 / fail-stop |
| S8 | 关闭 Mongo client/连接池（不等待新业务；仅 flush/close durable 所需连接） | 10s | 非零退出 / fail-stop |

规则：

- 每步有**总 deadline**（覆盖该步所有并发子项），不是 per-item 无限等待；
  任一 deadline 超时 → 进程以非零状态退出 / fail-stop，不得“尽力继续”后
  正常退出。
- 顺序严格：前一步未完成（或已 fail-stop）不进入下一步；S1 完成后不再接受
  新 admission；S2 之后不再发起新 activation；S6 之后不再有 session 写入。
- 每新增 lane 都扩展 composition shutdown test（把新 owner 的 finalizer 加入
  对应 step 或明确声明无 session/无 pending），不等 cutover。

## 3. composition 扩展规则

- `RouterSupervisor` 是唯一 lifecycle owner；composition 由 integration owner
  维护，消费稳定 `RouterComponents` manifest。
- 新 installed component 必须声明：shutdown step 归属、总 deadline、finalizer
  （或显式无状态）、health residue counter；checker 在 composition test 中
  验证 manifest 覆盖全部 installed components。
- shutdown 期间的 fail-stop 计数与 barrier 语义见 C-session 契约 §3.2(3)/§5。

## 4. §5.4 contract pack 必填项

### 4.1 owner / invariant

- Owner：`RouterSupervisor`（config、construction、listener/task join、
  shutdown；不拥有业务 mutable state）。
- Invariant：停机时先停止入场（admission/activation），再 drain 在场
  （finalizers/pending/leases），最后关闭基础服务（sessions/loader/Mongo）；
  每步 bounded；超时必 fail-stop；shutdown residue 计数全部归零后才可正常
  退出。

### 4.2 typed inputs / outputs

- Inputs：`ShutdownCommand`（watch/信号）、各 owner 的 `FinalizeResult` /
  `BarrierComplete` / `ResidueReport`、时钟。
- Outputs：`ShutdownStepResult { step, deadline, completed, residue }`、
  `FailStop { step, reason }`、进程退出码（正常 0 / fail-stop 非 0）、最终
  `ShutdownReport`（health 可观测）。

### 4.3 capacity

- 每步并发子项数 = installed components 数；deadline 为每步总预算；
  barrier ACK 集 = manifest 中 session-keyed consumers；无全局无界等待集合。

### 4.4 queue full

- shutdown 期间 mailbox data 满不得阻塞 terminal：reserved slot 保证
  `RuntimeSessionClosed` 可入队；S6 后 writer 只 drain 不接收新帧；
  S7 join 的 task/timer 全部为 bounded set。

### 4.5 timeout / disconnect / replacement / shutdown terminal

- 每步 deadline 超时 → fail-stop；S3 client WS finalizer 超时按
  C-client-lifecycle terminal 处理并继续该步 deadline 收敛；
  S5 lease release 超时按 protocol-unavailable 关闭 exact session；
  S6 barrier ACK 超时 → fail-stop；disconnect/replacement 在 S6 统一经
  cancellation + barrier 收敛。

### 4.6 health fields

- 每步 residue counter（该步 pending/permits/leases/sessions/tasks/timers
  剩余数）、`lastCompletedStep`、`shutdownDeadlineRemainingMs`、
  `failStop { step, reason }`、进程退出码；health 不含 Mongo URL/secret/
  业务 payload。

### 4.7 fake seam

- `FakeComponentManifest`（声明 step/deadline/finalizer）、`FakeClock`、
  `FakeBarrierAck`（注入 ACK 缺失/超时）、fake Mongo client（可注入 close
  失败）；fixture：`runtime/transport/testdata/process-lifecycle/shutdown-sequence.json`
  由 `runtime/transport/tests/process_lifecycle_contract.rs` 校验顺序与
  fail-stop 语义。

### 4.8 real boundary probe（定义）

- 真实 socket drain probe：打开 N 个 HTTP keep-alive + client WS + runtime WS
  连接，触发 shutdown；断言 accept 先停、连接 drain、session barrier 全 ACK、
  进程正常退出且 residue 归零；slow client 超过 S3 deadline 时进程非零退出。
  该 probe 从 PR 0b 的 C-net probe 扩展，逐步加入各 lane composition。

## 5. fixture 规格

`runtime/transport/testdata/process-lifecycle/shutdown-sequence.json`：

```json
{
  "schemaVersion": 1,
  "contract": "c-process-lifecycle-v1",
  "steps": [
    { "id": "stop-public-control-admission", "deadlineMs": 5000,
      "failStopOnTimeout": true,
      "sideEffects": ["public-listener-stop-accept", "control-listener-stop-accept"] },
    { "id": "stop-new-activation-reconcile-durable", "deadlineMs": 10000,
      "failStopOnTimeout": true,
      "sideEffects": ["no-new-activation", "durable-decision-reconciled"] },
    { "id": "drain-http-client-ws-finalizers", "deadlineMs": 15000,
      "failStopOnTimeout": true,
      "sideEffects": ["http-finalizers-drained", "client-ws-finalizers-drained"] },
    { "id": "terminal-dispatcher-broker-actor-pending", "deadlineMs": 10000,
      "failStopOnTimeout": true,
      "sideEffects": ["dispatcher-pending-zero", "broker-pending-zero", "actor-pending-zero"] },
    { "id": "release-runtime-generation-leases", "deadlineMs": 10000,
      "failStopOnTimeout": true,
      "sideEffects": ["generation-leases-zero"] },
    { "id": "close-runtime-sessions-barrier", "deadlineMs": 20000,
      "failStopOnTimeout": true,
      "sideEffects": ["session-barrier-all-acked", "directory-empty"] },
    { "id": "join-blocking-loader-tasks-timers", "deadlineMs": 15000,
      "failStopOnTimeout": true,
      "sideEffects": ["blocking-loader-joined", "tasks-joined", "timers-joined"] },
    { "id": "close-mongo", "deadlineMs": 10000,
      "failStopOnTimeout": true,
      "sideEffects": ["mongo-closed"] }
  ]
}
```

