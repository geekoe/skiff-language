# Leaf Task: D2 router control plane（durable task dispatch 接入）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；Layer Ownership、
  Submission And Visibility、Claim/Lease/Fencing、Runtime Admission And Settlement、
  Cancellation、TaskStatus/TaskCancelResult 语义）。
- 用户面契约：`doc/reference/dispatch.md`（`TaskRef` / `TaskStatus` /
  `TaskCancelResult` kind 拼写）。
- 批次父节点：`doc/implementation/dispatch-d-batch.md`（集成 Agent
  `/root/dispatch_d_integration`；本批次节点 D2）。
- 共享检查点：D1 `doc/implementation/dispatch-d1-wire-leaf.md`（task wire/control
  契约：timing、taskRef、错误码、status/cancel 帧、`request.start` taskAttempt 头）；
  task-control crate（TaskStore/Scheduler/AttemptAdmission，main `16c177a0` 已含）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`e5df67f8`（`dispatch-d-integration` 含 D1 的合并提交，已
  `git rev-parse` 确认：`e5df67f86b3147079d5335274a1a645985f2e377`）。
- worktree：`/Users/geek/workspace/skiff-d2-router`，branch `router-control`。
- 集成 Agent：`/root/dispatch_d_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

在 `skiff` 仓库实现阶段 D2：router 控制面把 task 行为从易失替换为 durable：

1. 组合根：router 进程构造 Mongo `TaskStore`（复用 `service_db.mongoUrl`；库
   `skiff-router` / 集合 `tasks`，沿用 task-control 默认）与 `Scheduler`；与既有
   activation 等 owner 明确隔离。
2. 真实 `AttemptAdmission` seam：claim 后针对冻结 execution image 选择已 admission
   该 image 的 Runtime，构造普通 `request.start` 并携带 `taskAttempt` 头
   （taskId/attemptId/leaseId）；四类决策映射：
   Accepted → pending-attempt 续租/结算追踪；RejectedProvable → release；
   Uncertain → 不 settlement，等 lease 过期 recovery；PermanentFailure →
   settle platform-failed。request 终态映射到 `TaskStore.settle`
   （response.end → succeeded；response.error → failed；普通 timeout → failed；
   disconnect/不确定 → 不 settle，走 lease recovery），并接入 scheduler
   pending-attempt 追踪（必要时扩展 scheduler API，不破坏既有测试与语义）。
3. `task.submit.request` handler：校验 timing/payload，按提交方携带的 TaskId
   幂等 create，立即任务成功后 wake scheduler，成功返回 taskRef；失败按 D1 错误码
   区分 definite/transient；响应不确定时复用原 TaskId 重试查询（不做二次 create）。
4. `task.status.request` / `task.cancel.request` handler：投影到
   `TaskStore.status/cancel`，kinds 与 reference 拼写一致；cancel 与 claim 竞争由
   store CAS 保证。
5. actor-method target：D2 以明确错误码 `unsupportedTarget` 拒绝（归入 definite
   rejection），删除旧易失 actor 派发路径；完整 get-or-activate 由阶段 E 补齐后
   移除该拒绝。
6. 删除 router 侧旧易失 task 路径（`TaskWireStore` / `TaskSubmitRouter` 等被控制面
   handler 取代；保留/清理以编译与测试为准）。
7. 健康/观测：task 控制面（leased 数、backlog、最老 eligible age、提交/结算/取消
   计数）投影到既有 health wire（`counters.tasks` 等既有 key 语义核对后使用），
   不新增第二套观测协议。
8. 测试：router 单元/集成覆盖 submit 成功/拒绝/幂等重试、status/cancel 映射、
   admission 四类结果、settlement 映射（success/failure/timeout/uncertain）、
   立即任务 wake、actor target 拒绝；既有 task wire corpus 与 router task 相关
   测试同步更新；不跑 live 全链（E 阶段做）。

## 预检结论（只读，锚定 e5df67f8）

### 真实入口与调用链

- `task.submit.request` 经 demux Task family → `InboundSinkSet.task` →
  `ActorTaskFrameSink`（易失：parent resolution → dispatcher derived pending /
  actor-method lane → 同步执行 → response）。status/cancel 帧在 D1 后
  fail-closed。
- `request.start` 普通派发：`RequestDispatcher::submit`（候选选择 + permit +
  revalidate + `RuntimePeer::send_request_start` + pending）；response 帧经
  `RequestFrameSink` → `dispatcher.on_frame` → HTTP pending。
- task 相关 health：`counters.tasks`（`SpawnedTaskCounters`）、
  `counters.requestPending.derivedTask`、`counters.actor.task`
  （`ActorTaskHealthDto`）、`loopRisk.dispatcher.pendingUnary`（含 derived）。
- task-control：`TaskStore`（memory/mongo 共享 reducer）、`Scheduler`（scan/claim/
  renew/recover + `AttemptAdmission` seam + wake fast path）、`RetryBackoffPolicy`。
- 配置：`RouterConfig.service_db.mongo_url` 已存在；组合根
  `RouterComponents::assemble/assemble_with`。

### 与兄弟节点重叠

无。当前仅有 main 与 `dispatch-d-integration` 两个 worktree；D3/D4 尚未建 worktree。
本叶子不改 compiler、不改 runtime eval/host 执行语义、不改
`doc/reference/`、`doc/architecture/` 与 `doc/implementation/**` 既有文件。

### 旧易失路径删除清单

```text
router/src/actor/task.rs          # TaskSubmitRouter / resolvers / TaskSubmitAcceptance /
                                  # ActorMethodTaskExecutionSink / ActorLaneTaskControl / TaskErrorCode
router/src/actor/task_sink.rs     # TaskWireStore / PendingTaskWire / TaskWireHealth
router/src/supervisor/actor_sink.rs 中 ActorTaskFrameSink、ActorMethodTaskExecutionSink impl、
                                  # task_invocations / TaskInvocationCorrelation / task_actor_method_execution
router/src/dispatch/dispatcher.rs 中 task_submit / task_function_locked / PendingKind::DerivedTask /
                                  # task_derived / task_actor_lane / task_ambiguous / task_parents /
                                  # register_task_parent / unregister_task_parent / task_parent_facts /
                                  # TaskSubmitResult / TaskRejectReason / DerivedTaskResult
router/src/dispatch/types.rs      # TaskSubmit / TaskTargetKind / ActorMethodTaskDispatch / derived_deadline
router/src/dispatch/frame.rs      # ActorMethodTaskControl、RuntimePeer::send_task_submit
router/src/dispatch/health.rs     # TaskHealth.derived_tasks/actor_lane_tasks/ambiguous_rejects
router/src/supervisor/actor.rs    # DispatcherTaskParentLookup / DeferredActorMethodTaskExecutionSink /
                                  # ActorComponents.task_router/execution_sink/actor_lane_task_control/task_wire_store
router/src/supervisor/session_ports.rs  # SessionRuntimePeer::send_task_submit / with_task_wire_store
router/src/listener.rs            # websocketConnect task-parent registration
router/src/actor/invocation.rs    # is_active_parent / parent_snapshot（仅 task-parent seam 消费）
router/src/actor/health.rs        # TaskHealth（actor task 计数）
router/src/health/counters.rs     # ActorTaskHealthDto / SpawnedTaskCounters actor_task_* 字段
runtime/transport/src/protocol/task.rs  # TaskSubmitAcceptance / response_header
```

### 关键实现决策（本叶子执行范围，不改设计语义）

- task attempt 作为普通 request 走 `RequestDispatcher`（同一 admission pool /
  concurrency / deadline / cancel 机制）；新增 `PendingKind::TaskAttempt` 与
  `task_attempt_submit`，终态通过注入的 `TaskAttemptTerminalSink` port 归还
  控制面。`RequestFrameSink` 对 task attempt 跳过 HTTP 转发。
- request deadline：每次 attempt 使用完整的新普通 request budget
  （router `request_timeout_ms`），与 task lease 分离；router 侧 deadline sweep
  （控制面 worker）调用 `dispatcher.timeout` → settle failed。
- 提交方未带 taskId 时由 router 生成；create 成功后 `due_at <= now` 即
  `scheduler.wake()`；create transient 失败后用同一 TaskId 查询
  （`store.status`），查到即视为成功，查不到回 `storeUnavailable`。
- `TaskSubmitRejectionCode` 增加 `unsupportedTarget`（definite）；D1 既有五码
  字符串不变。
- 健康：`counters.tasks` 改为 task 控制面投影（leased/renewing、backlog、
  oldest eligible age、提交/status/cancel/settle 计数）；`TaskStore` 增加只读
  `observe_backlog`（memory + mongo 实现 + contract 测试）。
- 测试注入：`assemble_with` 使用 `MemoryTaskStore`（与既有测试一致的注入面）；
  新增 `assemble_with_task_store` 显式注入；生产 `assemble` 用 Mongo。

## 禁止

- 不改 syntax/compiler（D3）；不改 runtime eval/host 执行语义（D4）；如编译需要
  机械修引用，在交接报告列出。
- 不改 `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**`
  既有文件（本叶子文件为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 verify。

## 实际写集（commit 后与交接报告一致）

```text
Cargo.lock
doc/implementation/dispatch-d2-router-leaf.md
router/Cargo.toml
router/src/actor/{health.rs,invocation.rs,mod.rs,types.rs}
router/src/actor/{task.rs,task_sink.rs}                 # 删除
router/src/dispatch/{dispatcher.rs,frame.rs,health.rs,mod.rs,types.rs}
router/src/health/{aggregator.rs,counters.rs}
router/src/lib.rs
router/src/listener.rs
router/src/supervisor/{actor.rs,actor_sink.rs,http.rs,mod.rs,session_ports.rs}
router/src/task/{admission.rs,control.rs,health.rs,mod.rs,sink.rs}   # 新增
router/tests/actor_invocation_relay.rs
router/tests/actor_live_lane.rs
router/tests/actor_task_router.rs                        # 删除
router/tests/actor_wire_corpus.rs
router/tests/actor_zero_pending.rs
router/tests/composition_components.rs
router/tests/dispatch_admission_corpus.rs
router/tests/dispatch_consumer_terminal.rs
router/tests/dispatch_harness/mod.rs
router/tests/dispatch_invariants.rs
router/tests/dispatch_live_probe.rs
router/tests/gates_wiring_ws.rs
router/tests/health_http.rs
router/tests/health_projection.rs
router/tests/session_budget_probe.rs
router/tests/task_control_unit.rs                         # 新增
router/tests/{task_parent_ws_connect.rs,task_repair_acceptance.rs}  # 删除
router/tests/task_repair_direction.rs
runtime/transport/src/protocol.rs
runtime/transport/src/protocol/task.rs
runtime/transport/src/protocol/task/tests.rs
runtime/transport/testdata/registration-handshake/frames.json
task-control/src/{memory.rs,mongo.rs,store.rs}
task-control/src/scheduler/mod.rs
task-control/tests/support/contract.rs
```

## 行为变化（既有测试更新记录）

- `task.submit.request` 由易失 parent-resolution / derived-pending 改为 durable
  TaskStore create（TaskId 幂等）+ scheduler wake；不再校验 callerRequestId parent。
- `task.status.request` / `task.cancel.request` 从 D1 的 fail-closed 变为真实
  handler，投影 `TaskStore.status/cancel`。
- actor-method target 以 `unsupportedTarget`（新增 definite 码）拒绝；旧
  actor-method task 派发路径（TaskSubmitRouter/TaskWireStore/ActorMethodTaskExecutionSink
  等）删除。
- dispatcher 不再有 derived-task pending；task attempt 作为普通 unary
  request 进入 dispatcher（`PendingKind::TaskAttempt`），终态通过
  `TaskAttemptTerminalSink` 归还控制面 settlement。
- `task.submit.response.requestId` 在 D2 取 TaskId（提交时还不存在 execution
  request）。
- D1 wire 没有 status/cancel error 帧：transient store 失败在 D2 投影为
  `expired` 并单独计入 health 的 `statusUnavailable` / `cancelUnavailable`
  （记录为 D2 限制，未来 wire 扩展前不回错误帧）。
- `TaskSubmitRejectionCode` 增加 `unsupportedTarget`（既有五码字符串不变）。
- `TaskSubmitAcceptance` 及其 `response_header` 从 transport 删除。
- 基线遗留修复（机械，非行为语义）：registration-handshake golden 的
  `health.empty` frameHex 从改名遗留 `spawnedTasksActive` 修正为
  `taskRequestsActive`；`session_budget_probe` 删除已失效的 inbound budget
  用例并对齐当前 `SessionBudgets`（outbound only）。
- health `counters.tasks` 由 session/actor-task 计数改为 durable 控制面投影
  （renewing/pending/backlog/oldest due + submit/status/cancel/settle/admission
  计数）；`counters.actor.task` 移除；`requestPending.derivedTask` 改为
  `taskAttempt`。

## 自验收矩阵

| 条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 组合根 Mongo TaskStore + Scheduler | `RouterComponents::assemble` 构造
  `MongoTaskStore::connect(service_db.mongo_url)` + `Scheduler` +
  `DurableTaskControl`/`DurableTaskFrameSink`；`assemble_with` 注入
  `MemoryTaskStore`（测试）；与 activation repository 独立 | 无第二套
  store/url 配置；`Cargo.lock` 仅 router 新增 skiff-task-control 依赖 |
  `cargo check -p skiff-router -p skiff-task-control -p skiff-runtime-transport` |
| AttemptAdmission 四类映射 + request.start 带 taskAttempt | 控制面 admission：
  image 匹配 epoch tuple + deployment → 构造 task request.start 并携带
  taskId/attemptId/leaseId；Accepted/RejectedProvable/Uncertain/PermanentFailure
  映射到 scheduler `handle_decision`（新测试覆盖四类 + store 状态） |
  dispatcher 只有 `task_attempt_submit` 一条 task 入口；`rg
  "TaskSubmitRouter|TaskWireStore"` 无生产引用 | `cargo test -p
  skiff-router --test task_control_unit`（14 PASS） |
| settlement 映射 | 控制面 worker：response.end→succeeded、response.error→failed、
  timeout→failed、disconnect→不 settle（forget lease，store recovery）；
  `Scheduler::forget_active_lease` 新增 API | dispatcher 不再有 derivedTask
  pending；`rg derived_task` 无生产引用 | 同上（settlement 4 分支用例） |
| submit 幂等/wake/错误码 | sink create → wake；transient → 同 TaskId 查询；
  `unsupportedTarget` definite；幂等 create 不产生第二条 | 旧易失错误字符串
  不再由 router 产生 | 同上（submit 7 用例，含 1 小时 scan 间隔的 wake
  fast-path 证明） |
| status/cancel 投影 | sink 映射 8+4 kinds，与 reference 拼写一致；cancel/claim
  竞争由 store CAS（memory contract 已覆盖） | 无第二套 task status 协议 |
  同上 + `cargo test -p skiff-task-control`（25 PASS） |
| 删除旧易失路径 | 删除清单全部移除，编译通过 | `rg TaskSubmitRouter/TaskWireStore`
  仅剩 transport corpus 本地同名 helper 与文档引用 | `cargo check -p
  skiff-router --tests` |
| health 投影 | `counters.tasks` 含控制面字段；`observe_backlog` memory/mongo
  实现 + contract 测试 | 无新观测端点 | `cargo test -p skiff-task-control` +
  `cargo test -p skiff-router --test health_http --test health_projection` |
 | 既有测试同步 | task 相关 router/transport 测试更新/删除；D1 记录的两处基线
  失败（registration golden、SessionBudgets 漂移）一并机械修复 | 无旧易失断言
  残留 | `cargo test -p skiff-runtime-transport`（全过，含
  registration_handshake）+ `cargo test -p skiff-router`（全过 ×5） |

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_d_integration`，并通知主 Agent `/root`。
