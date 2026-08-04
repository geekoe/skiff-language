# Leaf Task: F2a actor-method durable task 继承 test-case capability

## 引用链

- 权威契约：`doc/reference/testing.md`（Runtime Execution And Effect Policy：
  root 发起的 direct dispatch、recursive dispatch、同步 Actor method call 与
  `dispatch actor.method(...)` 都是同一 case 的派生请求，携带同一 capability 与
  active parent request id；同一 service、同一 origin Runtime connection；跨
  service / 跨 Runtime / 父请求终结后迟到一律 fail closed）。
- 权威契约：`doc/architecture/test-runner-runtime-isolation.md`（testCaseCapability
  是 Router/Runtime Host 间不透明 authority；`task.submit.request` wire 只携带
  `callerRequestId`，不得重复携带 capability / test parent id；Router 只从同一
  session 上仍 active 的父请求派生；已 admit 的 Actor method 保持该 case 直到
  terminal，root finalization 等待它结束）。
- 权威契约：`doc/architecture/durable-task-dispatch.md`（task attempt 是普通
  request，复用同一 Runtime admission / 资源模型；task.submit.response 只等待
  durable commit，不等待 terminal；位置透明是 production 语义）。
- 失败证据（gate_final2 triage）：subject `actor-test-effect-capability` 的
  `root finalization waits for spawned and recursively spawned actor effects`
  报 `unused test effects (2 outcome(s))` 后被 `blocked network target` 拦截；
  `router/src/task/admission.rs` 三处硬编码
  `test_case_capability: None` / `test_effects_enabled: false`。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`b2aae64a`（`dispatch-e-integration` HEAD，`git rev-parse` 确认）。
- worktree：`/Users/geek/workspace/skiff-f2a-capability`，branch
  `task-case-capability`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

1. 父 test-case capability / test effects enabled 沿 task 提交链路传递：
   task.submit.request wire（若需要新字段）→ TaskRecord（task-control 持久化）→
   admission → actor get-or-create 请求与 owner.invoke 帧（ActorInvokeInput）→
   执行。
2. 遵守 same-service / same-Runtime connection 约束；跨 connection 不得携带。
3. 函数 target 的 task attempt 若同款丢失一并修（有证据说明）。
4. 测试：修复 FAIL case 全绿（`verify --only skiff-tests` 4 个 entry 全 PASS）；
   router/task 单测补 capability 正/负例；既有 task wire corpus /
   task_control_unit / task_actor_method_execution 回归全绿。
5. 叶子文档记录设计决策、写集与证据。

## 预检结论（只读，锚定 b2aae64a）

- `router/src/task/admission.rs` 三处硬编码确认：
  - `build_request`（函数 target `request.start` task attempt 帧）
    `test_effects_enabled: false` / `test_case_capability: None`；
  - actor-method 路径的 `ActorGetOrCreateRequest`（activateInitial 控制帧）与
    `ActorMethodInvokeFrameHeader` / `ActorInvokeInput`（owner.invoke 帧）全部
    `None`。
- **函数 target 同款丢失**：预检证据是 `build_request` 硬编码；Runtime 侧
  `task_eval_adapter` 已有 `begin_derived(capability, router_session, request_id)`
  支持带 capability 的 task request.start，因此函数路径只需 Router 在帧中恢复
  capability 并钉住 origin connection（本次一并修复）。
- wire 契约：`task.submit.request`（`TaskSubmitRequestFrameHeaderV2`）只携带
  `caller_request_id` / `caller_kind`；`test-runner-runtime-isolation.md` 明确
  禁止 wire 重复携带 capability / parent id。**不需要新 wire 字段**，corpus
  不需要变化。
- 直接 actor call 参考路径：Runtime `actor_method_adapter.rs` /
  `eval_capability_adapter/actor.rs` 在 `actor.getOrCreate.request` /
  `actor.method.invoke` 帧中携带 `test_case_capability` +
  `test_case_parent_request_id`（= caller request id）；Router
  `actor_sink.rs` 转发并登记；Runtime Host `begin_actor_method` /
  `begin_derived_from_parent` 用 capability + active parent 注册 case 派生请求。
- 旧 spawn 参考路径：git 历史 `79057f25 fix(router): inherit spawn authority
  from parent request`（TS era）显示旧易失 spawn 的 `handleSpawnSubmit` 在回
  `spawn.submit.response` 前先完成 derived dispatch（admission 收敛）；D2 durable
  化后 submit response 只等 durable commit，test case 因此无法让 root 等待
  attempt——这是本次在 test 路径恢复“admission 收敛后才回 response”的依据。

## 关键实现决策（本叶子执行范围）

- **wire 不加字段**：capability 由 Router 在提交时从同一 session 上仍 active 的
  父请求 / Actor invocation 派生，随 TaskRecord 持久化；attempt 时恢复进普通
  request / actor 帧。`task.submit.request` / corpus 保持 byte-exact。
- **TaskRecord 新增 `test_case: Option<TaskTestCaseAuthority>`**（task-control
  model + Mongo DTO）：含 capability、parent request id、origin runtime id 与
  connection generation。Mongo 缺字段按旧记录兼容解码为 `None`。
- **父派生端口**：`TaskSubmitParentResolver` 按 `caller_kind` 分别查
  RequestDispatcher pending（capability 在 admit 时从 request.start 头保留）与
  ActorInvocationRelay pending（capability 在 `invoke()` 时从
  `ActorInvokeInput` 保留），且都要求精确 session/connection。父不可解析时按
  production 提交处理（`None`），不引入新的拒绝面；valid 流程中 test 父必然
  active。
- **admission 恢复 + fail closed**：
  - 函数 target：`request.start` task attempt 帧带
    `test_effects_enabled = true` 与 capability；`TaskAttemptSubmit.prefer_session`
    钉住 origin session（无 candidate 或 revalidate 失败 → 不 reselect，直接
    PermanentFailure / fail closed）。
  - actor-method target：target service 必须等于父 service（先于 catalog
    检查）；owner 必须是 origin session（精确 replica + connection generation）；
    `ActorGetOrCreateRequest`、`ActorMethodInvokeFrameHeader`、
    `ActorInvokeInput` 全部携带 capability 与 parent request id。
  - production（`test_case: None`）路径完全不变：位置透明、普通 owner 选择。
- **test 提交响应 gating（本轮 gate 实证需要）**：capability 恢复后首次 gate 运行
  仍失败——attempt 的 owner.invoke 在 root case finalize 之后到达，Runtime 按
  `begin_actor_method` 拒绝“parent unknown/finalized”。因此 test-case 立即任务在
  durable commit 后、写 `task.submit.response` 前等待**首次 admission 收敛**
  （`Accepted` / `PermanentFailure`，最多 5s；不等待 terminal）。Router 在 create
  前订阅 admission 广播、再 wake scheduler，owner.invoke / request.start 帧先于
  submit response 进入同一 writer 队列，Runtime 按序注册 case 派生请求，root
  finalization 才能等待它。这与旧易失 spawn 的 admission-wait 对齐；production
  提交不做任何等待。
- **函数路径证据**：函数 target 同样修复（`build_request` + prefer_session）；
  无独立 gate 用例，但单测覆盖 capability 正例/负例与 origin 钉住。

## 禁止

- 不改 `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**`
  既有文件（本叶子文件为新增）。
- 不改 `task.submit.request` wire / corpus（契约禁止携带 capability）。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。

## 自验收矩阵（实际证据）

| 设计/任务条款 | 代码证据 | 测试命令 |
| --- | --- | --- |
| submit 时从 active 父派生 capability 并持久化 | `router/src/task/parent.rs` + `sink.rs`（`TaskTestCaseAuthority`）；RequestDispatcher pending / ActorInvocationRelay pending 保留 capability | `submit_from_test_request_parent_captures_test_case_authority`、`submit_success_creates_record_and_returns_task_ref`（None 负例） |
| TaskRecord + Mongo 持久化 | `task-control/src/model.rs` `TaskTestCaseAuthority`；`mongo.rs` `testCase` DTO（缺字段兼容 None） | `cargo test -p skiff-task-control`（memory contract + reducer）；mongo probe 增加 authority round-trip（ignored 需真实 Mongo） |
| 函数 attempt 恢复 capability + origin 钉住 | `admission.rs` `build_request` / `prefer_session`；`dispatcher.rs` 精确 session 过滤且失败不 reselect | `test_case_function_attempt_carries_capability_and_prefers_origin_session`、`test_case_function_attempt_without_origin_candidate_is_permanent_failure` |
| actor-method attempt 恢复 capability / parent；same-service / same-connection | `admission.rs`（owner 选择、GetOrCreate、owner.invoke、ActorInvokeInput）；`invocation.rs` relay 保留 capability | `test_case_actor_attempt_carries_capability_and_parent_on_invoke`、`test_case_actor_attempt_cross_service_is_permanent_failure`、`test_case_actor_attempt_without_origin_candidate_is_permanent_failure`、`branch1_...`（None 负例） |
| production 不受影响 | 所有普通路径 `test_case: None` → 原语义（无 capability、位置透明） | 既有 task_control_unit 22 项、task_actor_method_execution 13 项、task_telemetry 5 项、dispatch_admission_corpus 2 项、w_model_task_consumer 4 项全绿 |
| test 提交 gating | `control.rs` first-admission broadcast + `sink.rs` 订阅于 create 前、等待首次 admission | `test_case_submission_gate_observes_first_admission_outcome` |
| FAIL case 修复 | 无（端到端证据） | `node scripts/verify.mjs --only skiff-tests` → 4/4 entry PASS，subject 2/2 PASS |
| wire / corpus 不变 | 无 wire 字段改动 | `cargo test -p skiff-runtime-transport`（141 lib + task_wire_corpus 11/11）；`w_model_task_consumer` 4/4 |

## 实际写集

```text
doc/implementation/dispatch-f2a-case-capability-leaf.md
task-control/src/model.rs                 # TaskRecord.test_case + TaskTestCaseAuthority
task-control/src/mongo.rs                 # testCase DTO（前向兼容缺字段）
task-control/src/reducer.rs               # 测试 fixture 字段
task-control/tests/support/fixtures.rs    # 测试 fixture 字段
task-control/tests/mongo_probe.rs         # authority round-trip 断言
router/src/dispatch/types.rs              # TaskAttemptSubmit.prefer_session
router/src/dispatch/dispatcher.rs         # Pending capability、parent_test_capability、origin 过滤
router/src/actor/invocation.rs            # PendingInvocation capability + parent_test_capability
router/src/task/parent.rs                 # 新增 TaskSubmitParentResolver / Router / Noop
router/src/task/control.rs                # FirstAdmissionOutcome broadcast（test 提交 gating）
router/src/task/sink.rs                   # 父派生落盘 + admission gating
router/src/task/admission.rs              # attempt 恢复 capability + same-service/connection
router/src/task/mod.rs                    # 导出
router/src/supervisor/mod.rs              # resolver + control 装配
router/tests/task_control_unit.rs         # capability 正/负例、origin 钉住、gating
router/tests/task_actor_method_execution.rs # actor capability / fail-closed 用例
router/tests/task_telemetry.rs            # 构造点适配
router/tests/dispatch_harness/mod.rs      # attempt header 记录 + prefer_session 字段
```

## 验证记录（聚焦）

- `cargo test -p skiff-task-control`：lib 25、memory_contract 3、scheduler_memory
  10 全绿。
- `cargo test -p skiff-runtime-transport`：lib 141、task_wire_corpus 11、其余
  corpus 全绿（wire 未变）。
- `cargo test -p skiff-router`（聚焦 5 个 target）：task_control_unit 22、
  task_actor_method_execution 13、task_telemetry 5、dispatch_admission_corpus 2、
  w_model_task_consumer 4 全绿。
- `cargo check -p runtime`：通过（runtime host 编译不受影响）。
- `node scripts/verify.mjs --only skiff-tests`：
  `[skiff-tests] passed 4 canonical source test entries`，Summary
  `tasks: 1 | passed: 1 | failed: 0`；其中
  `PASS main.__test::root finalization waits for spawned and recursively spawned actor effects`
  与 `PASS main.__test::direct synchronous actor call inherits only its case effect capability`。
- 环境备注：首次 gate 因磁盘满失败；清理 `/tmp` 中无进程持有的旧构建残留
  （router-inst-repro2-Gs1ci3 / router-dbg-target / actor-eval-frame-diet-target
  及 4 个旧 test-bin）并安装 worktree `scripts/`、`telemetry/` 的 pnpm 依赖后通过。
