# Leaf Task: E3b dispatch observability 事件补齐 + 文档收敛

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（Observability And Retention
  节完整阅读：submission accepted/rejected、scheduled→ready 延迟、claim/attempt/
  Runtime selection/eligible wait、lease renew/loss/stale settlement、cancel 各结果、
  target success/application failure/infrastructure recovery、permanent platform
  failure、duplicate notification absorbed、artifact retention blocked/unavailable、
  terminal age/backlog depth/oldest eligible；TaskId 主 correlation key，AttemptId/
  requestId 细化；Canonical Contract Ownership 最后两条：runtime-deployment-topology
  的 retained task image cold activation lane、actor-model 的 detached Actor call
  收敛到 durable actor-method target）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration`；本节点 E3 e2e_observability 的代码/文档拆分，
  与 E3a E2E 并行）。
- 已合并代码：D2 `3a0c138c`（router 控制面 + `counters.tasks`：renewing/pending、
  backlog、oldest due、submit/status/cancel/settle/admission 计数）；E1 `2fab6d66`
  （status/cancel not_found 计数）；telemetry 基础设施：`telemetry/` Node server、
  `runtime/transport` `TelemetryEvent`/`TelemetryBatchEnvelope`、
  `runtime/host/src/host/telemetry.rs` producer/exporter（skiff-telemetry-v1
  WebSocket）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`2dd96402`（`dispatch-e-integration` HEAD，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-e3b-obs-docs`，branch `obs-docs`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读；不改 `doc/implementation/**`
  既有文件（本叶子文件为新增）。

## 任务合同摘要

1. observability 事件：按权威清单补齐 task dispatch telemetry 事件（router 控制面 +
   runtime 提交侧），复用既有 telemetry 机制（skiff-telemetry-v1 帧与
   `TelemetryEvent` schema，不新增第二套协议）；事件带 TaskId（有则带
   AttemptId/requestId）correlation；至少覆盖：submission accepted/rejected、
   scheduled→ready、claim、lease renew/loss/stale settlement、cancel 各结果、
   target success/failure、infrastructure recovery、platform failure、backlog 深度。
   已有 `counters.tasks` health 计数器保留并与之对齐。
2. 文档收敛（只按权威文档收敛，不发明新语义）：
   - `doc/architecture/runtime-layered-crate-architecture.md` 与
     `doc/architecture/test-runner-runtime-isolation.md`：内部 `spawn.submit` /
     spawn routes 等 wire 标识 → `task.*`（机械收敛，查证后改）。
   - `doc/architecture/runtime-deployment-topology.md`：新增 retained task image 的
     cold activation lane 小节（与 active/draining generation pin 分开；不能为了
     未来 task 阻止旧 Runtime 正常退出；不能在到期时解析 latest assembly/config
     snapshot）。
   - `doc/architecture/actor-model.md`：detached Actor call 段落收敛到 durable
     actor-method target（ActorActivationSnapshot、get-or-activate、无独立易失
     spawn 队列）；历史“spawn 到 actor”示例可保留为历史。
   - 审计 `doc/reference` 与 `doc/architecture` 是否还有与 durable-task-dispatch.md
     冲突的 dispatch 语义（除历史白名单外），列出结果。
3. 测试：telemetry 事件发射聚焦测试（router/task 相关 + runtime 提交侧）；文档用
   rg 验证无断链/残留（白名单除外）。

## 预检结论（只读，锚定 2dd96402）

- router 目前没有任何 telemetry producer：`router/src/config/mod.rs` 已解析
  `telemetry.{endpoint,protocol,topics,...}`，`TelemetrySource::Router` 在
  transport/TS 协议中合法（`TELEMETRY_REGISTER_SOURCES` 含 router），但 router 只把
  telemetry 配置转给 runtime control envelope，自身不发射。E3b 需要在 router 侧补一个
  复用 skiff-telemetry-v1 的 producer/exporter（WebSocket `/telemetry`，register +
  batch 帧），只发射 task dispatch 事件。
- 事件点定位：
  - `router/src/task/sink.rs`：submit 四分支（accepted / rejected / transient /
    ambiguous-then-accepted）、status/cancel 各分支（已有计数，E3b 只补事件）。
  - `router/src/task/control.rs`：settle 结果目前被 `let _ = store.settle(...)`
    丢弃，需要读取 `SettleOutcome` 补 stale/conflict 事件；uncertain terminal →
    `forget_now`（infrastructure recovery）；release → 可恢复 backoff。
  - `task-control/src/scheduler/mod.rs`：claim/renew/recover/duplicate 在 scheduler
    循环内部，router 不可见；需加 `SchedulerObservation` seam（默认 no-op），
    router 实现 telemetry observer，不改执行语义。
  - `router/src/task/admission.rs`：claim 后的 Runtime selection（function
    request_id / actor owner）与四类 admission 决策、artifact 不可用/被阻塞。
  - `router/src/health/aggregator.rs`：`task_control.backlog()` 每轮 health render
    读取；补 `task.backlog` metric 事件（含 terminal age，需扩展
    `BacklogObservation`）。
  - runtime 提交侧：`runtime/host/src/capability_context/actor.rs`
    `RequestClient::submit_task_with_scope` 是 wire 往返点；`RequestClientContext`
    目前没有平台 telemetry。E3b 给 `RequestClientContext` 增加可选
    `RequestTelemetryContext`（host 已有 `RequestTelemetryContext`，eval 层不可见），
    由 `eval_capability_adapter` 三个构造点（assembly_execution_context /
    actor_method_adapter / activation_execution_rebinder）把 `self.telemetry_context`
    传入，事件带 TaskId + caller requestId。
- 文档现状：`runtime-layered-crate-architecture.md` 6 处 spawn wire/概念残留（308
  spawn worker、609 spawn.submit、612 internal spawn invocation branch、619 spawn
  submit receipt、621 spawn 专用 start/accepted frame、724 spawn routes）；
  `test-runner-runtime-isolation.md` 1 处（90 行 `spawn.submit` wire）；
  `runtime-deployment-topology.md` 无 retained task image cold activation lane；
  `actor-model.md` “dispatch 到 actor 方法”节缺 durable snapshot/get-or-activate/
  无易失 spawn 队列表述，且“平台不持久化待执行调用队列”与权威契约冲突。
- 与 E3a 重叠边界：E3a 拥有端到端可观测性流程/探针（telemetry server + router +
  runtime 全链路）；E3b 只拥有事件发射代码、producer/exporter、文档与聚焦测试。
  双方不共享文件写入；E3b 为 E3a 提供可消费的事件面。

## 关键实现决策（本叶子执行范围）

- **复用既有 telemetry 协议**：router 侧新增 `router/src/telemetry.rs`，实现与
  runtime host 同构的 bounded producer（queue/batch/register/WebSocket exporter，
  source=Router，producerId=`router-<environment>`），不新增第二套协议；无
  `telemetry.endpoint` 或 disabled 时用 no-op sink，业务路径零额外阻塞。
- **事件 schema**：`TelemetryTopic::Log`（level info/warn/error）+ `Metric`（backlog
  gauge），`name` 用 `task.<phase>.<outcome>` 点号命名；correlation 放顶层
  `requestId`/`traceId`/`serviceId`/`runtimeId` + `attrs.{taskId,attemptId,leaseId}`；
  不新增 TelemetryEvent 顶层字段（避免动 telemetry/TS 校验，保持最小改动）。
- **task-control 观察 seam**：`SchedulerObservation` trait（on_due_ready /
  on_claim / on_claim_duplicate / on_renew / on_recover / on_release），默认 no-op；
  `Scheduler::new` 不变，新增 `Scheduler::with_observer`。`BacklogObservation` 增加
  `observed_at` / `terminal_count` / `oldest_terminal_at`（memory + mongo 同口径；
  health counters 字段不变）。
- **counters 对齐**：所有事件发射点与既有 `TaskControlCounters` 递增点并置，
  不新增/不删除计数语义。
- **事件-计数器对照表**（router）：

  | 权威条款 | 事件 name | 发射点 | 对齐计数器 |
  | --- | --- | --- | --- |
  | submission accepted | `task.submit.accepted` | sink.handle_submit | submissions_accepted |
  | submission rejected | `task.submit.rejected` | sink.handle_submit | submissions_rejected |
  | submission ambiguous | `task.submit.uncertain` | sink.handle_submit | submissions_transient |
  | scheduled→ready | `task.ready` | scheduler observer (scan_once) | — |
  | claim / eligible wait | `task.claim` | scheduler observer (claim 成功) | — |
  | Runtime selection / attempt | `task.admission.selection` | admission seam（function request_id / actor owner） | — |
  | admission accepted | `task.admission.accepted` | admission seam | admissions_accepted |
  | admission rejected | `task.admission.rejected` | admission seam | admissions_rejected |
  | admission uncertain | `task.admission.uncertain` | admission seam | admissions_uncertain |
  | permanent platform failure | `task.platform.failed` | admission seam（PermanentFailure）/ actor VersionRejected | admissions_permanent_failure |
  | lease renew | `task.lease.renewed` | scheduler observer | renewing_attempts（状态） |
  | lease loss | `task.lease.lost` | scheduler observer（renew rejected） | — |
  | lease release/backoff | `task.lease.released` | scheduler observer / control Release | settlements_upgrading（释放侧） |
  | infrastructure recovery | `task.recovered` | scheduler observer（recover_expired_lease 成功）+ control Uncertain | settlements_uncertain |
  | duplicate notification absorbed | `task.duplicate.absorbed` | scheduler observer（claim Rejected） | — |
  | target success | `task.settled`（outcome=succeeded） | control settle | settlements_succeeded |
  | application failure | `task.settled`（outcome=targetFailed） | control settle | settlements_failed |
  | stale settlement | `task.settle.stale` | control settle（StaleLease/ExpiredLease/NotLeased/NotFound/Conflict） | — |
  | cancel 各结果 | `task.cancel.{canceled,alreadyStarted,alreadyTerminal,expired,notFound,unavailable}` | sink.handle_cancel | cancel_* |
  | artifact retention unavailable | `task.artifact.unavailable` | admission image_authority None | admissions_rejected |
  | artifact retention blocked | `task.artifact.blocked` | admission 永久分支 | admissions_permanent_failure |
  | backlog / oldest eligible / terminal age | `task.backlog`（metric） | health aggregator | backlog_* / oldest_due_at_ms |

- **runtime 提交侧**：`RequestClient::submit_task_with_scope` 按 wire 结果发射
  `task.submit.accepted` / `task.submit.rejected`（definite 错误码） /
  `task.submit.uncertain`（transport/store 不确定）；eval 层 bounded retry 语义
  不变（每次 wire 往返一个事件）。

## 禁止

- 不改 dispatch 执行语义（提交/claim/settlement/actor 路径；observer 只观察）。
- 不 push、不动共享主 worktree、不改 `doc/implementation/**` 既有文件。
- 不改 `doc/reference/`（审计只读）；不发明新公开语义。
- 不跑完整 gate。

## 文档审计清单（doc/reference + doc/architecture，除历史白名单）

- 已收敛（本叶子修改）：
  - `runtime-layered-crate-architecture.md`：spawn.submit / internal spawn invocation /
    spawn submit receipt / spawn 专用 start/accepted frame / spawn routes / spawn worker
    → task.*（提交链路按 D2/D4 实际实现更新为 durable submit → scheduler claim →
    request.start task invocation）。
  - `test-runner-runtime-isolation.md`：`spawn.submit` wire → `task.submit.request`。
  - `runtime-deployment-topology.md`：新增 retained task image cold activation lane。
  - `actor-model.md`：detached Actor call 收敛到 durable actor-method target；删除
    “平台不持久化待执行调用队列”与权威契约冲突的表述。
- 审计无冲突（保持现状）：
  - `doc/reference/dispatch.md`、`queue.md`、`runtime.md`、`db.md`、`observability.md`、
    `any-interface*.md`、`static-semantics.md`、`syntax.md`、`testing.md`：dispatch 均为
    用户表达式 / 普通 request dispatch 语义，与权威设计一致。
  - `durable-task-dispatch.md` 自身对旧 spawn 的表述（旧 surface 取代说明）。
- 历史白名单（不改）：
  - `actor-model.md` “spawn 到 actor 的异常/trace 上下文”历史示例；
  - `recoverable-value.md` “旧 spawn payload”；
  - `any-interface-value.md` 划删除线 “queue/spawn/persistent work item payload”；
  - `actor-shared-heap-design.md` / `tail-call-execution.md`：`tokio::spawn`（任务运行时，
    与 dispatch wire 无关）；
  - `verify-task-runner.md`：测试任务进程 spawn（与 dispatch 无关）。

## 自验收矩阵（实际证据）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 复用既有 telemetry 机制（skiff-telemetry-v1，不新增第二套协议） | `router/src/telemetry.rs`（`RouterTelemetryProducer`/`RouterTelemetryExporter`，register + batch WebSocket 帧，source=Router）；runtime 提交侧复用 `RequestTelemetryContext` | 无新 envelope 类型；`rg "TELEMETRY_BATCH_TYPE|TelemetryBatchEnvelope" router/src` 均来自 transport 协议 | `cargo test -p skiff-router --test task_telemetry`（5/5） |
| submission accepted/rejected/uncertain（router） | `sink.rs::handle_submit` 各分支 emit `task.submit.accepted/rejected/uncertain`，与 `submissions_*` 计数并置；事件带 TaskId + callerRequestId + traceId | `task_telemetry::task_sink_emits_submit_accepted_rejected_and_cancel_events` 断言 event.request_id=parent-request-1、taskId 存在 | 同上 |
| cancel 各结果（canceled/alreadyStarted/alreadyTerminal/expired/notFound/unavailable） | `sink.rs::handle_cancel` 各分支 emit `task.cancel.*`，与 `cancel_*` 计数并置 | `task_sink_cancel_unknown_owner_emits_not_found`；`rg "task.cancel" router/src/task/sink.rs` 6 个事件名 | 同上 |
| scheduled→ready、claim/eligible wait、lease renew/loss、infrastructure recovery、duplicate absorbed、provable release | `task-control/src/scheduler/mod.rs` `SchedulerObservation` seam（默认 no-op）+ `router/src/task/observation.rs` 实现；事件名 `task.ready/task.claim/task.lease.renewed/task.lease.lost/task.recovered/task.duplicate.absorbed/task.lease.released` | `rg "SchedulerObservation" task-control router`；scheduler 语义测试 10/10 不回归 | `cargo test -p skiff-task-control --test scheduler_memory`（10/10，含 observation seam 用例） |
| Runtime selection / admission 四类决策 | `admission.rs::emit_admission_selection`（function 用 dispatcher Accepted session，actor 用 owner_connection）+ `emit_admission_decision`（accepted/rejected/uncertain/platform.failed），与 `admissions_*` 计数并置 | `task_actor_method_execution` 10/10、`task_control_unit` 18/18 全绿 | `cargo test -p skiff-router --test task_actor_method_execution --test task_control_unit` |
| artifact retention blocked/unavailable | `admission.rs` image_authority None → `task.artifact.unavailable`；永久分支 → `task.artifact.blocked`/`task.platform.failed` | 事件名仅在 admission.rs 出现一次 | 同上 + `cargo test -p skiff-router --lib`（69/69） |
| target success/application failure/stale settlement | `control.rs::emit_settle_outcome`（settled 按 outcome 分类 + `task.settle.stale` conflict/staleLease/expiredLease/notLeased/notFound + `task.settle.uncertain`），与 `settlements_*` 计数并置 | `rg "task.settled|task.settle" router/src/task/control.rs` | `cargo test -p skiff-router --lib`；`cargo test -p skiff-router --test task_control_unit` |
| backlog depth / oldest eligible / terminal age | `task-control/src/store.rs` `BacklogObservation` + observed_at/terminal_count/oldest_terminal_at（memory+mongo）；`router/src/telemetry.rs::backlog_metric_event`；health aggregator 每轮 render emit `task.backlog` metric | contract 测试 `backlog_observation` 断言 terminal 字段；`producer_batches_and_backlog_metric_shape` 断言 oldestEligibleAgeMs/terminalAgeMs | `cargo test -p skiff-task-control --test memory_contract`（3/3）；`cargo test -p skiff-router --test task_telemetry` |
| runtime 提交侧 submission accepted/rejected/uncertain | `runtime/host/src/capability_context/actor.rs` `RequestClient::submit_task_with_scope` 按 wire 结果 emit；`RequestClientContext` 携带可选 `RequestTelemetryContext`（assembly/actor-method/rebinder 三处构造点传入） | `task_submit_emits_accepted_event_with_task_id_correlation` 断言 taskId/caller requestId/traceId | `cargo test -p skiff-runtime-host --lib`（429/429） |
| 文档收敛：runtime-layered / test-runner wire 标识 → task.* | 6 处 spawn wire/概念 + 1 处 `spawn.submit` 收敛；flow 段落按 D2/D4 实际提交链路更新 | `rg "spawn\.submit|spawn routes|spawn worker|spawn 专用" doc/architecture/{runtime-layered-crate-architecture,test-runner-runtime-isolation}.md` 为空 | `rg` 反向搜索（见验证记录） |
| 文档收敛：runtime-deployment-topology cold activation lane | 新增 `## Retained Task Image 冷激活通道`（与 active/draining pin 分开；不阻止旧 Runtime 退出；到期不解析 latest assembly/config snapshot；terminal 原子释放 retention） | 小节引用权威设计；无新语义 | `rg "Retained Task Image" doc/architecture/runtime-deployment-topology.md` |
| 文档收敛：actor-model detached Actor call → durable actor-method target | dispatch 到 actor 方法节补 `ActorActivationSnapshot` / get-or-activate / 无独立易失 spawn 队列；生命周期节“平台不持久化待执行调用队列”收敛到 durable task 承载 | 历史“spawn 到 actor”示例保留（白名单）；`rg "易失.*spawn|持久化待执行调用队列" doc/architecture/actor-model.md` 仅收敛后表述 | 同上 |
| 审计 doc/reference + doc/architecture 冲突（除历史白名单） | 无剩余冲突：dispatch 表达式/request dispatch 语义均与权威设计一致；历史白名单为 durable-task-dispatch 旧 spawn 表述、recoverable-value “旧 spawn payload”、any-interface-value 划删除线、actor-model 历史示例、tokio::spawn/进程 spawn | `rg -i spawn doc/architecture doc/reference` 仅白名单/无关项 | 见审计清单（本叶子文件） |
| health counters 保留并对齐 | 所有事件发射点与既有 `TaskControlCounters` 递增点并置，未删除/改名任何计数器 | `git diff` 无 `counters.rs`/`health.rs` 删除 | `cargo test -p skiff-router --test task_control_unit`（18/18） |

## 交接注意事项

- 交付：branch `obs-docs`、worktree
  `/Users/geek/workspace/skiff-e3b-obs-docs`、commit/tree、写集与自验收矩阵直接报告
  集成 Agent `/root/dispatch_e_integration` 并通知主 Agent `/root`。
- router producer/exporter 只服务于 task dispatch 事件；普通 router 请求/会话
  telemetry 不在本节点。
