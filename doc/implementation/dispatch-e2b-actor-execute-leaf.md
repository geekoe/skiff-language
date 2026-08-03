# Leaf Task: E2b actor-method dispatch 执行侧（get-or-activate + settlement）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（Actor-method target 节
  完整阅读：五个 get-or-activate 分支；registry entry 已存在时永远沿用 entry 创建
  输入、snapshot 只用于缺失重建；恢复 entry、普通 get 与 owner claim 在同一个
  identity fencing 下竞争；task lease 不提供第二套顺序；旧 task 与升级竞争以 actor
  owner 原子 admission / upgrade fencing 为准）。
- 用户面契约：`doc/reference/dispatch.md` §4（actor 方法 target 规则）与 §7 测试
  矩阵第 18 条（五个 get-or-activate 分支）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration`；本节点 E2 actor_task_target 的执行侧拆分）。
- 已合并代码：E2a `947a9075`（提交侧冻结 `ActorActivationSnapshot`，wire 携带
  runtime 形式 `expectedTypePlan`，见
  `doc/implementation/dispatch-e2a-actor-submit-leaf.md`）；D2 router 控制面
  `3a0c138c`（`router/src/task/`，`unsupportedTarget` 拒绝待移除）；C1 task-control
  （`DetachedCallTarget::ActorMethod`、`ActorActivationSnapshot`、`TaskRecord`）；
  W-actor 基建（`router/src/actor/`：`ActorOwnershipRegistry` /
  `ActorActivationRequestBroker` / `ActorInvocationRelay` / `ActorMethodCatalogView`）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`db74fb60`（`dispatch-e-integration` HEAD，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-e2b-actor-exec`，branch `actor-task-execute`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要（执行侧）

1. 移除 router 提交侧的 `unsupportedTarget` 拒绝；actor-method task 正常入 store
   （wire `actor_method` metadata → `DetachedCallTarget::ActorMethod`，snapshot
    base64 解码 + expected plan 衔接）。
2. scheduler admission seam 增加 actor-method 分支：按权威设计五个分支执行
   get-or-activate；attempt 关联 actor 方法调用的 terminal，映射 settlement
   （成功 → succeeded；target throw/reject → failed；
   `ActorVersionRejectedError` → platform-failed；`ActorUpgradingError` →
   可恢复，release/backoff 后续 attempt；disconnect/不确定 → 不 settle 走 lease
   recovery）。
3. `expectedTypePlan` 衔接：E2a wire 的 runtime 形式 expectedTypePlan 与
   task-control store 的 artifact `RecoverableExpectedTypePlan` 桥接；创建 / claim /
   执行 decode 使用 linked expected plan（store 不解释 plan）。
4. 保持 actor owner 唯一性：不得绕过 `ActorOwnershipRegistry` / incarnation
   fencing；task lease 不替代 actor owner lease；snapshot 只用于 entry 缺失重建。
5. 测试：五个 get-or-activate 分支各至少一例；提交侧不再拒绝 actor target；
   settlement 映射；`task_control_unit` 与既有 actor/task 测试同步更新；Mongo
   live probe 扩展按需。

## 预检结论（只读，锚定 db74fb60）

- `router/src/task/sink.rs::handle_submit` 在 image resolve 之前对
  `TaskTargetKind::ActorMethod` 以 `unsupportedTarget` definite 拒绝；函数 target
  已可正常入 store。`router/src/task/admission.rs::build_request` 对
  `DetachedCallTarget::ActorMethod` 返回 `None` → `PermanentFailure`
  （"unsupported until stage E"）。
- router actor 基建是同步 reducer owner：`ActorOwnershipRegistry`（in-memory
  identity / incarnation / owner fence + claim reserve/commit/abort）、
  `ActorActivationRequestBroker`（get-or-create dedup + activateInitial 发送 +
  ACK 关联，outcome 按 rpc_id 保存）、`ActorInvocationRelay`（method
  return/error/cancel 精确 fence 关联）、`ActorMethodCatalogView`（A3 投影查询）。
  普通 actor 调用由 `router/src/supervisor/actor_sink.rs` 处理：catalog 命中 →
  `registry.current_owner` → `relay.invoke` → `actor.owner.invoke` 帧 →
  `actor.method.return/error` / `actor.owner.failure` 回传 caller。
- runtime `actor.owner.invoke` 已支持 `activation_bootstrap`（
  `runtime/host/src/host/actor_owner_execution.rs::execute_actor_owner_invoke_inner`
  冷激活后直接执行 method），但 **router 侧不能直接用它绕过 registry**：必须走
  `ActorActivationRequestBroker` 的 reserve/commit 与 put-if-absent，否则可能并发
  创建两个 live incarnation。
- `ActorOwnershipRegistry` 的 entry **不保存创建输入**；`std.actor.get` 的
  `bootstrap_bytes` 在 `ActorGetOrCreateRequest` 里可用。为了满足
  "registry entry 已存在时永远沿用 entry 创建输入"，执行侧需要把创建输入保存到
  entry（`ensure_present` 时 put-if-absent 冻结，`commit` 不覆盖）。
- upgrade 控制器（`actor.replace` / `MarkUpgrading` / `Discard` / `Activate`）
  在 Rust router 尚未 wiring（`actor.replace.request` fail closed）；runtime
  `ActorInstanceStore` 有 `upgrading` 状态但 owner 侧不发送
  `ActorUpgradingError` / `ActorVersionRejectedError` 帧。E2b 的 forward-target
  分支落地为：task attempt 的 invoke 走普通 actor admission，owner runtime 在
  upgrading 期间返回的 `ActorUpgradingError`（wire 已支持）映射为可恢复
  release/backoff；`ActorVersionRejectedError`（wire 已支持）映射为 platform-failed。
  升级控制器的路由侧触发/等待不在本节点范围，叶子交接中说明。
- E2a wire 的 `expectedTypePlan` 是 runtime 形式 `RuntimeRecoverableExpectedTypePlan`
  的 serde JSON（create 参数 record）；task-control C1 的
  `ActorActivationSnapshot.expected_type_plan` 是 artifact 形式
  `RecoverableExpectedTypePlan`。仓库不存在 runtime→artifact plan bridge；执行
  decode 一律使用 linked expected plan（runtime 冷激活按 linked create 声明解码
  bootstrap），因此 store 里的 plan 是 durable witness，不是执行输入。
- 任务控制平面已具备 `DurableTaskControl`（settlement / deadline sweep /
  forget lease）与 `RouterTaskAttemptAdmission`（image authority + dispatcher
  task-attempt 提交）；`ActorFrameSink` 拥有 invocation correlation map
  （`invocations`），task attempt 需要复用该 correlation 并把 terminal 送回 task
  控制平面。

## 关键实现决策（本叶子执行范围）

### 1. get-or-activate 五个分支（`router/src/task/admission.rs`）

actor-method attempt 不再走 ordinary `request.start`。admission seam 持有
`Arc<ActorComponents>`（registry / activation broker / relay / catalog）+
`SessionHandle` + `Arc<ActiveRoutingEpochStore>` + `Arc<dyn WsSessionWriter>`，
按以下分支执行：

1. `registry.current_owner(key)` 存在且 fence implementation == task
   implementation：按普通 Actor admission 排队执行 method —— `relay.invoke` +
   发送 `actor.owner.invoke`（`activation_bootstrap: None`），并注册 task-attempt
   invocation correlation → `Accepted`。
2. 无 owner 但 registry entry 存在且 entry implementation == task
   implementation：用 **entry 保存的创建输入**（`ensure_present` 时冻结）走
   `ActorActivationRequestBroker::get_or_create` 冷激活，再执行 method。
3. registry entry 丢失：用 task snapshot 的 `createInput` 恢复最小 entry
   （`ensure_present` + reserve + activateInitial 同一 fencing），再执行 method；
   首次成功恢复获胜（put-if-absent）。
4. forward target / upgrading：invoke 被 owner runtime 以
   `ActorUpgradingError`（含 `retryAfterMs`）拒绝 → 可恢复
   `release(now + retryAfterMs)`，后续 attempt 带退避。
5. fence / entry implementation 与 task implementation 不同（已被新实现接管）：
   `AdmissionDecision::PermanentFailure`，reason 带 `ActorVersionRejectedError`，
   scheduler settle 为 `platform-failed`；不切回旧实现、不把旧 payload 交给新代码。

owner 候选选择复用 actor lane 的确定性
`sha256(actorIdHash) % sorted candidates`（把 `pick_owner_candidate` 从
`actor_sink.rs` 提取到 `router/src/actor/owner_candidate.rs`，两处共用）。激活等待
用 `ActorActivationRequestBroker` 新增的 `tokio::sync::Notify`：outcome 写入时
notify，admission 侧有界轮询（激活 deadline + 余量），超时归 `RejectedProvable`
（release/backoff），避免跨 reducer 锁 await。

### 2. expectedTypePlan 桥接（`router/src/task/actor_plan.rs`）

wire 的 runtime 形式 JSON 在 router 侧反序列化为等价 DTO（`deny_unknown_fields`
fail closed），然后投影为 artifact `RecoverableExpectedTypePlan`：

- `root`：结构节点映射为 `TypeRefIr`（`Nullable` / `Union` / `Literal` /
  `Record`；`Json`/`JsonObject`/`bytes`/`date`/`string`/`TaskRef`/`bool`/`number`/
  `integer`/`null`/`Stream`/`Array`/`Map` 用 compiler 既有 canonical builtin
  拼写，`Array`/`Map`/`Stream` 带参数）；
- `Representation` / `AnyInterface`：结构上投影为 `TypeRefIr::Builtin { name:
  <identity 规范串>, args }`，并把 identity 规范串写入 `root_type_identity_ref`；
  identity 规范串按运行时 identity kind 前缀命名（`type:` / `service:` /
  `package:` / `artifact:` / `interface:`），确定性、无碰撞；
- `Unresolved` 节点 fail closed（E2a 的 create plan 由 `from_linked` 产生，不会
  出现 unresolved；出现即提交拒绝）。

同时把 wire 的 runtime 形式 JSON 原样保存到
`ActorActivationSnapshot.expected_type_plan_runtime`（见决策 4）。创建 / claim /
执行 decode 不使用 store 中的 plan：执行 decode 由 runtime 按 linked expected
plan 完成（`actor.owner.invoke` bootstrap 冷激活 + `ActorInstanceStore` 按 linked
create 声明解码），store plan 只是 durable witness。

### 3. task-attempt actor invocation terminal（`router/src/task/actor_attempt.rs` +
`router/src/supervisor/actor_sink.rs`）

`ActorFrameSink` 的 `InvocationCorrelation` 增加
`task_attempt: Option<TaskAttemptCorrelation { request_id, task_id, attempt_id,
lease_id }>`；admission seam 通过新的
`ActorFrameSink::register_task_attempt_invocation` 注册。owner 侧 terminal 分流：

- `actor.method.return` → `ActorAttemptTerminal::Succeeded`；
- `actor.method.error`：
  - `ActorUpgradingError { retry_after_ms }` → `Upgrading { retry_after_ms }`；
  - `ActorVersionRejectedError` / `ActorIncarnationReplacedError` →
    `VersionRejected`（platform-failed）；
  - 其它 error payload（当前 wire 只有上述三类）→ `TargetFailed`；
- `actor.owner.failure` → `TargetFailed`（owner 明确拒绝执行该 attempt）；
- invocation deadline（`on_relay_deadline`）→ `TargetFailed`（普通 request
  timeout 语义）；
- owner session 断开（`on_runtime_session_closed`）→ `Uncertain`（不 settle，
  lease recovery）。

`DurableTaskControl` 实现 `ActorAttemptTerminalSink`：
`Succeeded` → settle `succeeded`；`TargetFailed` → settle `failed`；
`VersionRejected` → settle `platform-failed`；`Upgrading { retry_after_ms }` →
`TaskStore::release(now + retry_after_ms)` + `scheduler.forget_active_lease`
（可恢复退避，不 settle）；`Uncertain` → 现有 `forget_now`（lease expiry 驱动
recovery）。

### 4. task-control store 模型小扩展（C1 兼容）

`ActorActivationSnapshot` 增加
`expected_type_plan_runtime: Option<serde_json::Value>`（`#[serde(default)]`
风格，Mongo DTO 可选字段 `expectedTypePlanRuntime`）。理由：runtime 形式是提交侧
唯一能冻结的权威 plan，artifact 投影有损；执行侧不消费 store plan，但保留权威
形式避免未来把投影误当执行输入。既有 25 unit + 3 memory contract + 9 scheduler
测试语义不变（fixture 补 `None`，Mongo 未设置时缺失字段）。

`ActorOwnershipRegistry::ensure_present` 增加 `create_input: &[u8]` 参数并在
entry 缺失时 put-if-absent 保存；新增 `entry(key)` 读取
`ActorRegistryEntry`（identity + 创建输入）。`commit` 不覆盖创建输入
（put-if-absent 语义）。

`DetachedCallTarget::ActorMethod` 增加 store 形式
`declaration_owner: ActorDeclarationOwner`（wire `ActorDeclarationOwnerFrameHeader`
的投影）。理由：分支 3 恢复最小 entry 时，owner Runtime 必须用精确 declaration
owner 坐标解析 linked actor 声明并执行 `create`；该事实只在提交 wire 中存在，
C1 模型未保存。

### 5. 提交侧（`router/src/task/sink.rs`）

移除 `unsupportedTarget` 分支；actor-method 与 function 共用 image authority /
timing / payload quota / TaskId-idempotent create 流程。`actor_method` metadata
缺失、snapshot base64 非法、expected plan 不可投影 → definite `rejected`，不产生
task（wire 层 `validate_task_submit_request` 已保证 metadata 存在与 base64
canonical，router 侧仍 fail closed）。

## 禁止

- 不改 compiler 语法 / D3/E2a 已定的 runtime 提交侧语义；发现 E2a 缺漏先报告主
  Agent，不自行改设计。
- 不改 `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**`
  既有文件（本叶子文件为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。
- task lease 不替代 actor owner lease；不绕过 `ActorOwnershipRegistry` /
  incarnation fencing；snapshot 不保存 Actor 内存。

## 实际写集

```text
doc/implementation/dispatch-e2b-actor-execute-leaf.md
task-control/Cargo.toml                              # serde_json 正式依赖
task-control/src/model.rs                            # ActorActivationSnapshot + expected_type_plan_runtime
task-control/src/model.rs                            # DetachedCallTarget::ActorMethod + declaration_owner
task-control/src/mongo.rs                            # expectedTypePlanRuntime / declarationOwner DTO + round-trip
task-control/src/reducer.rs                          # actor fixture 补字段
router/src/actor/owner_candidate.rs                  # pick_owner_candidate 提取（新）
router/src/actor/ownership.rs                        # ActorRegistryEntry + entry create_input 冻结/读取
router/src/actor/activation.rs                       # Notify 等待通道 + ensure_present 传 bootstrap
router/src/actor/mod.rs                              # 导出
router/src/supervisor/actor_sink.rs                  # task-attempt correlation + terminal 分流
router/src/task/actor_plan.rs                        # runtime plan → artifact plan（新）
router/src/task/actor_attempt.rs                     # ActorAttemptTerminal + sink trait（新）
router/src/task/actor_ports.rs                       # TaskActorOwnerPort（新，测试注入 seam）
router/src/task/actor_target.rs                      # snapshot key / declaration owner 转换（新）
router/src/task/admission.rs                         # get-or-activate 五分支
router/src/task/control.rs                           # ActorAttemptTerminalSink 实现
router/src/task/sink.rs                              # 移除 unsupportedTarget；actor target 入 store
router/src/task/mod.rs                               # 导出
router/src/supervisor/mod.rs                         # 组装新 seam 依赖
router/tests/dispatch_harness/mod.rs                 # build_epoch_with_actor_methods
router/tests/task_control_unit.rs                    # 提交正例 + 分支/terminal 测试
router/tests/task_actor_method_execution.rs          # 五分支 + settlement（新，10 例）
router/tests/{actor_ownership_registry,actor_activation_broker,actor_lease_scheduler,actor_live_lane,actor_zero_pending,composition_components,gates_wiring_actor}.rs
                                                     # ensure_present 签名机械补参
task-control/tests/mongo_probe.rs                    # actor record round trip
runtime/transport/src/protocol/task.rs               # UnsupportedTarget doc comment 更新
```

## 自验收矩阵（实际证据）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 提交侧不再拒绝 actor target；snapshot/declaration owner 入 store | `sink.rs::resolve_target`（base64 解码 + `project_runtime_expected_type_plan` + `declaration_owner_from_frame`）；`task_control_unit::submit_actor_method_target_creates_durable_actor_record` | `rg unsupportedTarget router/src` 为空 | `cargo test -p skiff-router --test task_control_unit`（18/18） |
| expectedTypePlan 桥接（runtime 形式 → artifact witness，runtime 形式原样保存） | `actor_plan.rs`（DTO + `TypeRefIr` 投影 + identity 规范串 + Unresolved fail closed）；`ActorActivationSnapshot.expected_type_plan_runtime` | `actor_plan::tests` 3 例；Mongo round trip 断言 target 相等 | `cargo test -p skiff-router --lib actor_plan`；`cargo test -p skiff-task-control`；Mongo probe |
| 分支 1 live 同 implementation → 普通 admission | `admission.rs` branch1（fence 命中 + `invoke_actor_method`） | `task_actor_method_execution::branch1_*` 断言 activation_bootstrap=None 且 payload 原样 | 同上（10/10） |
| 分支 2 entry 存在 → 用 entry 创建输入冷激活 | `admission.rs` entry 分支（`entry.create_input` 优先，put-if-absent） | `branch2_*` 断言 activateInitial bootstrap=[9] 且 snapshot=[1] 被忽略 | 同上 |
| 分支 3 snapshot 恢复最小 entry + 首次恢复获胜 | `admission.rs` None 分支（snapshot bootstrap + broker get-or-create）；`ActorOwnershipRegistry::ensure_present` put-if-absent | `branch3_*` 断言恢复 entry、单次 activateInitial、restore ∈ {[1],[2]} | 同上 |
| 分支 4 ActorUpgradingError → release + retry 退避 | `actor_sink.rs::actor_error_terminal` → `Upgrading`；`control.rs` `ControlEvent::Release`（`store.release(now+retryAfterMs)` + forget lease） | `branch4_*` 断言 Ready + retry_not_before>now + counters | 同上 |
| 分支 5 被新 implementation 接管 → platform-failed | `admission.rs::version_rejected`（PermanentFailure，reason 含 ActorVersionRejectedError）；owner 帧 `ActorVersionRejectedError` → `VersionRejected` → PlatformFailed settle | `branch5_*` + `actor_attempt_version_rejected_*` 断言 PlatformFailed | 同上 |
| settlement 映射（成功/failed/platform-failed/不确定） | `control.rs` `ActorAttemptTerminalSink` 实现；`actor_sink.rs` Return/Error/owner.failure/disconnect 分流 | `actor_attempt_return_*`、`actor_attempt_owner_failure_*`、`actor_attempt_version_rejected_*`、`actor_attempt_owner_disconnect_*`（Running 不 settle） | 同上 |
| owner 唯一性：不绕过 registry / fencing；task lease 不替代 owner lease | 冷激活全部走 `ActorActivationRequestBroker`（reserve/commit/put-if-absent）；invoke 仅在有 fence 后发送；`activation_bootstrap: None` | `branch3_concurrent_*` 单 claim 共享；无直接 `actor.owner.invoke` + bootstrap 的 router 发送点 | `rg "activation_bootstrap: Some" router/src/task` 为空 |
| task-control 既有 25+3+9 语义不破坏 | 新字段 `#[serde(default)]` 风格（Mongo 缺失字段 → None）；fixture 补字段 | `cargo test -p skiff-task-control` 25+3+9 全绿 | 同上 |
| Mongo live probe 扩展 | `mongo_probe.rs::actor_record_round_trip`（runtime plan + declaration owner + target 相等） | 真实 rs0 PASS | `SKIFF_TASK_CONTROL_MONGO_URL=... cargo test -p skiff-task-control --test mongo_probe -- --ignored` |
| 受影响 crate 编译与既有 actor/task 测试全绿 | router actor lane 测试、composition、transport、runtime w_model/h_task、task wire corpus | 无回归 | 见下方验证记录 |

## 验证记录

- `cargo check`：task-control / router / transport / eval / host /
  request-contract / capability-context 全 PASS（仅既有 warning）。
- `cargo test -p skiff-task-control`：25 unit + 3 memory contract + 9 scheduler
  PASS；Mongo probe（真实 rs0）PASS。
- `cargo test -p skiff-router --lib`：69 PASS（含 actor_plan 3 例）。
- `cargo test -p skiff-router`：actor_ownership_registry 7、actor_activation_broker
  5、actor_invocation_relay 6、actor_lease_scheduler 4、actor_live_lane 2、
  actor_wire_corpus 6、actor_catalog_view 7、actor_zero_pending 3、
  gates_wiring_actor 5、task_control_unit 18、task_actor_method_execution 10、
  dispatch_admission_corpus 2、task_repair_direction 6、w_model_task_consumer 4、
  composition_components 10、composition_session_seams 3 全 PASS。
- `cargo test -p skiff-runtime-transport`：141 lib + 全部 corpus（task_wire_corpus
  11/11）PASS。
- `cargo test -p runtime --test w_model_task_consumer --test h_task_parent_cut_corpus`：
  4 + 4 PASS。
- `git diff --check` PASS。

## 交接注意事项

- 五个分支的 router 侧证据全部落在 `router/tests/task_actor_method_execution.rs`
  （10 例）。
- upgrade 控制器的路由侧 wiring（`actor.replace` / forward-target 触发等待）不在本
  节点；E2b 的 forward 分支通过 owner runtime 的 `ActorUpgradingError` /
  `ActorVersionRejectedError` 帧落地，后续 E-actor upgrade 节点补 router 侧
  触发/等待后即可端到端闭合。
- `ActorOwnershipRegistry` entry 现在冻结创建输入（put-if-absent）；entry 已存在
  时 task 执行永远沿用 entry 创建输入，snapshot 只在 entry 缺失时重建。
- task-control store 模型新增 `expected_type_plan_runtime` 与
  `DetachedCallTarget::ActorMethod::declaration_owner`（C1 兼容，Mongo DTO 可选 /
  新增字段）；执行 decode 仍使用 linked expected plan。
