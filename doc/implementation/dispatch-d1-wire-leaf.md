# Leaf Task: D1 dispatch wire / control 契约扩展（共享 wire/control 检查点）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；提交顺序、
  definite rejection / ambiguous acceptance 区分、attempt 映射为一次普通
  request frame、transport `requestId` 只标识 frame 不标识 task identity、
  terminal outcome 归还 task settlement、cancel 只有 before-start、
  TaskStatus / TaskCancelResult 可区分结果）。
- 用户面契约：`doc/reference/dispatch.md`（`dispatch` 表达式、`std.task.TaskRef`、
  `std.task.status` / `std.task.cancel`、TaskStatus / TaskCancelResult kind 拼写）。
- 批次父节点：`doc/implementation/dispatch-d-batch.md`（集成 Agent
  `/root/dispatch_d_integration` 创建，位于集成分支 `dispatch-d-integration`
  commit `83f6aefb`；main 上尚未存在，按批次父文档引用）。
- 已合入共享检查点：`task-control` crate（main `16c177a0` 已含
  `TaskId` / `AttemptId` / `LeaseId` / `ServiceOwner` / `TaskState` /
  `TaskStatus` / `TaskCancelResult` 模型）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`16c177a099a3927eb9b89ce0afd61e419ad91ff7`（main HEAD，共享主
  worktree 干净）。
- worktree：`/Users/geek/workspace/skiff-d1-wire`，branch `dispatch-wire`。
- 集成 Agent：`/root/dispatch_d_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

在 `skiff` 仓库实现阶段 D1：task wire/control 契约扩展（wire/control 契约，
行为不改），供后续 router 控制面 / compiler / runtime 消费：

1. `task.submit.request` 增加 `timing` 字段（kind 三态：`immediate` /
   `after { durationMs: u64 }` / `at { utcMillis: i64 }`）。负数 / overflow
   拒绝语义由后续 compiler/runtime 做，wire 只承载；默认/兼容语义：旧 corpus
   与构造点补 `immediate`（缺省即 immediate，canonical 编码省略该字段保持旧
   corpus byte-exact）。
2. `task.submit.response` 携带 `taskRef`：opaque 字符串（编码 TaskId + owner
   scope）。canonical 编码/解码定义在 `runtime-transport`（预检决定放哪；
   runtime / router 均依赖 transport，compiler 不需要 wire 编解码）；
   不可解码视为 wire 错误。
3. `task.submit.error` 增加明确拒绝码（至少 `invalidTiming` / `payloadInvalid`
   / `quotaExceeded` / `storeUnavailable` / `rejected`），区分 definite
   rejection 与暂时性失败（`storeUnavailable` 为暂时性）。
4. 新增 `task.status.request/response` 与 `task.cancel.request/response` wire
   帧；response 携带 TaskStatus / TaskCancelResult 的 canonical kind 字符串
   投影（与 `doc/reference/dispatch.md` 拼写一致）。
5. attempt 关联：普通 `request.start`（runtimeAssembly task 分支）增加可选
   `taskAttempt` 头（`taskId` + `attemptId` + `leaseId`）；不实现 settlement
   逻辑（D2）。
6. 更新全部受影响构造点/消费点保持编译（`router/src/actor/task_sink.rs`、
   `router/src/supervisor/actor_sink.rs`、`runtime host` 等对 `task.submit`
   帧的构造/解析处机械补 `timing=immediate` / 新字段默认值），不改变任何
   运行时行为。
7. 更新 testdata task-wire corpus（`frames.json` 等）与 codec/协议测试，覆盖：
   submit 三态 timing 编解码、TaskRef 往返、status/cancel 帧、`request.start`
   带/不带 task-attempt 头、错误码投影。

## 预检结论（只读，锚定 baseline 16c177a0）

- canonical task wire codec 是 `runtime/transport/src/protocol/task.rs` 的
  `TaskSubmitRequestFrameHeaderV2`（`callerKind` 闭合枚举）；V1
  `TaskSubmitRequestFrameHeader` 是 legacy shape（缺 `callerKind`，canonical
  codec 拒绝），仍由 `control_mapper.rs` 构造并被 transport 测试消费。
- 生产 task 提交帧构造点：`runtime/eval/src/task_ops.rs`
  （`submit_task_statement` 构造 `TaskSubmitControlRequest`）、
  `runtime/host/src/host/router_session/task_submit.rs`（V2 encoder）、
  `runtime/transport/src/control_mapper.rs`（V1 encoder）、
  `runtime/capability-context`/`runtime/request` 的 re-export 链、
  `router/src/supervisor/actor_sink.rs`（`task.submit.response` 构造）与
  transport `TaskSubmitAcceptance::response_header()`。
- `request.start` 的生产 task attempt 帧是
  `RuntimeAssemblyTaskRequestStartFrameHeader`（`runtime_assembly_request.rs`；
  `invocation.kind="task"`），由 `router/src/supervisor/session_ports.rs`
  `SessionRuntimePeer::send_task_submit` 构造，runtime host
  `request_entry/assembly_wire.rs` 解码。task-attempt 头加在该 header。
- corpus 消费点（均断言 `frames.len() == REQUIRED_FRAMES.len()` 与
  direction/frameType/decodeAs/payloadPresence）：
  `runtime/transport/tests/task_wire_corpus.rs`、
  `runtime/transport/tests/w_model_task_corpus.rs`、
  `runtime/tests/w_model_task_consumer.rs`、
  `runtime/tests/h_task_parent_cut_corpus.rs`、
  `router/tests/w_model_task_consumer.rs`；另有
  `router/tests/task_repair_acceptance.rs` 的逐帧方向表与 response 投影断言。
- TaskRef / TaskStatus 投影放置决策：定义在 `runtime-transport`
  `protocol/task.rs`，不引入 `skiff-task-control` 依赖（task-control 带
  mongodb/tokio，wire crate 不需要；task-control 保持无 runtime 依赖）。
  TaskRef 格式 `skiff-task-v1:<base64url-nopad(owner)>.<base64url-nopad(taskId)>`；
  不可解析即 wire 错误。status/cancel kind 用 wire 投影枚举，`serde
  rename_all="camelCase"` 与 reference 拼写一致，测试断言逐字相等。
- 错误码：`ActorTaskRuntimeErrorFrameHeader` 与 actor 帧共享，不能收成闭合
  枚举（会破坏既有 `ParentNotFound` 等 router 行为）；新增
  `TaskSubmitRejectionCode` 闭合投影枚举（五码）+ `is_transient()` 分类，
  既有 code 字符串保持接受（行为不改），D2 控制面使用新枚举。
- 与兄弟节点无重叠：D2（router control）与 D3/D4（compiler/runtime）均
  pending；本叶子只改 wire/契约 + 机械编译闭合。

## 实际写集（commit 后与交接报告一致）

```text
doc/implementation/dispatch-d1-wire-leaf.md
router/src/supervisor/actor_sink.rs
router/src/supervisor/session_ports.rs
router/tests/actor_live_lane.rs
router/tests/actor_live_probe.rs
router/tests/actor_task_router.rs
router/tests/task_repair_acceptance.rs
router/tests/task_repair_direction.rs
router/tests/w_model_task_consumer.rs
runtime/capability-context/src/{lib.rs,outbound_control.rs,request.rs}
runtime/eval/src/task_ops.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/eval_capability_adapter/actor/tests.rs
runtime/host/src/host/router_session/task_submit.rs
runtime/host/src/host/router_session/tests.rs
runtime/host/src/host/router_session/tests/{connection_lifecycle.rs,
  control_response_lifecycle.rs,h_task_parent_cut.rs,runtime_assembly_request.rs}
runtime/request-contract/src/{lib.rs,outbound.rs,outbound_control.rs}
runtime/request/src/{lib.rs,outbound.rs}
runtime/tests/{h_task_parent_cut_corpus.rs,w_model_task_consumer.rs}
runtime/transport/src/{control_mapper.rs,protocol.rs}
runtime/transport/src/control_mapper/tests.rs
runtime/transport/src/protocol/task.rs
runtime/transport/src/protocol/task/tests.rs
runtime/transport/src/runtime_assembly_request.rs
runtime/transport/src/runtime_assembly_request/tests.rs
runtime/transport/testdata/task-wire/frames.json
runtime/transport/tests/{task_wire_corpus.rs,w_model_task_corpus.rs}
```

`Cargo.lock` 不变（无新依赖）。

## 集成注意事项

- 基线 main `16c177a0` 存在与本叶子无关的既有失败，已在独立基线 worktree
  （同 commit、独立 target）复现确认：
  - `cargo test -p skiff-runtime-transport --test registration_handshake_corpus`
    的 `frame_catalog_is_byte_exact_and_complete` 与
    `handshake_sequences_match_frozen_semantics`：`health.empty` frameHex 仍是
    `spawnedTasksActive`，header JSON 已是 `taskRequestsActive`（rename 批次遗留
    的 stale golden；本叶子未触碰 registration-handshake）。
  - `cargo check -p skiff-router --test actor_live_lane`：`SessionBudgets` 缺
    `inbound_frames` / `inbound_bytes` 字段（测试与 API 漂移；本叶子只在该文件
    补 `timing: None` 机械字段）。
- `request.start` 不属于 task family direction 表（task family 前缀 `task.`）；
  task-wire corpus 纳入 request.start 帧只冻结其 wire 形态，方向由 request
  family codec 负责（`task_repair_acceptance` 已按此断言）。
- D1 未安装 status/cancel handler：`ActorTaskFrameSink` 仍只接受
  `task.submit.request`，status/cancel request 到达 sink 后按
  `MalformedFrame` fail-closed（demux 测试已固化该行为），D2 替换为真实 handler。

## 禁止

- 不改 compiler / runtime-eval 行为（除上述机械构造点补默认值外）。
- 不实现 router 控制面 handler / scheduler 接入 / settlement 逻辑（D2）。
- 不改 `doc/reference/` 与 `doc/architecture/` 既有文档；不改
  `doc/implementation/**` 既有文件（本叶子文件为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree。

## 自验收矩阵

覆盖范围：聚焦验证；完整 gate 未跑（按任务约定）。

| 条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| timing 三态承载 + 旧 corpus/构造点补 immediate | `protocol/task.rs`
  `TaskSubmitTiming`（`immediate`/`after{durationMs}`/`at{utcMillis}`）；
  V1/V2 header `timing: Option`（serde default + skip_none）；eval 构造点补
  `Immediate`；host/control_mapper encoder 映射（Immediate→None） | 旧
  `task.submit.request.function/actorMethod/legacy` frameHex 未变且 roundtrip
  byte-exact；`timing.after/at` 新 golden 帧 | `cargo test -p
  skiff-runtime-transport`（139 lib + 各 corpus 全过） |
| taskRef opaque 编码 TaskId+owner；不可解码 wire 错误 | `TaskRef`
  encode/parse + `Serialize`/`Deserialize`；response/status/cancel header 校验；
  malformed 用例（前缀/分隔/空段/非法 base64/padding）拒绝 | `task.submit.response`
  golden 含 taskRef 且 roundtrip byte-exact；`submit_response_task_ref_is_a_wire_error...`
  通过 | 同上 + `cargo test -p skiff-runtime-host --lib
  capability_context::actor::tests`（MissingTaskRef/InvalidTaskRef 新用例） |
| 明确拒绝码 + definite/transient 分类 | `TaskSubmitRejectionCode` 五码 +
  `is_transient`（仅 `storeUnavailable`）；错误帧 corpus 投影 | 既有
  `ParentNotFound` corpus 与 router 错误字符串未改（行为不改） |
  `rejection_code_projection_and_transient_classification` | 同上 |
| status/cancel wire 帧 + canonical kind 投影 | 新 frame 类型 + codec；
  `TaskStatusWire`/`TaskCancelResultWire` 拼写与 reference 逐字一致（8+4 全量）
  | direction 表覆盖 4 新帧；demux 测试固化 D2 前 fail-closed |
  `status_and_cancel_frames_round_trip...` + `cargo test -p skiff-router --test
  task_repair_direction` | 同上 |
| request.start 可选 taskAttempt 头 | `RuntimeAssemblyTaskAttemptFrameHeader` +
  decode 非空校验；session_ports 构造补 None；corpus 带/不带两帧 | 既有
  request-wire corpus 未改；`runtime_assembly_task_request_optional_task_attempt_is_validated`
  通过 | `cargo test -p skiff-runtime-transport runtime_assembly_request` |
| 全部构造点/消费点编译闭合，行为不改 | 写集清单（38 文件，全部为契约/机械
  闭合）；`git diff --name-only` 无 compiler/runtime-eval 行为文件、无
  doc/reference、doc/architecture | 旧 corpus 帧 byte-exact 保持 |
  `cargo check --workspace`（PASS）；`cargo test -p skiff-runtime-host --lib`
  （427 PASS）；`cargo test -p skiff-router --lib`（66 PASS） |
| task-wire corpus 覆盖新契约 | frames.json 新增 13 帧 + response 更新，全 corpus
  byte-exact roundtrip（transport/runtime/router 三个消费面） | REQUIRED_FRAMES
  5→18 同步 5 个消费点 + task_repair_acceptance 方向表 | `cargo test -p
  skiff-runtime-transport`、`cargo test -p runtime --test
  w_model_task_consumer --test h_task_parent_cut_corpus`、`cargo test -p
  skiff-router --test w_model_task_consumer --test task_repair_acceptance` |
| 既有基线失败不入本叶子 | 基线 worktree（16c177a0）复现
  registration_handshake_corpus 2 失败与 actor_live_lane 编译失败；本叶子未触碰
  对应文件 | `git diff --name-only` 无 registration-handshake /
  session.rs / budget.rs | 见“集成注意事项” |
| 不改既有文档 / 不 push / 不动主 worktree | 写集见上；主 worktree 只读 |
  `git diff --check` 通过；`Cargo.lock` 无 diff | `git status --short`（仅本叶子
  文件） |

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_d_integration`，并通知主 Agent `/root`。
