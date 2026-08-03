# Leaf Task: E2a actor-method dispatch 提交侧（冻结 ActorActivationSnapshot 并 wire 承载）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（Actor-method target 节完整阅读；
  Submission And Visibility 的提交顺序、definite rejection 语义、snapshot 只含可恢复
  create 输入、registry entry 已存在时执行侧沿用 entry 创建输入、snapshot 只在 entry
  缺失时重建）。
- 用户面契约：`doc/reference/dispatch.md` §4（actor 方法 `actor.method(...)` /
  `self.method(...)`；create 内禁止 `dispatch self.method(...)`；void/null 返回；
  recoverable 参数提交前 fail closed）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration`；本节点 E2 actor_task_target 的提交侧拆分）。
- 已合并代码（main@033391ba + E1 集成 6dffbd86，已 `git rev-parse` 确认）：
  D3 compiler dispatchSubmit plan、D4 runtime `submit_dispatch_call`、D1/E1 wire
  task.submit.request、C1 task-control `ActorActivationSnapshot` /
  `DetachedCallTarget::ActorMethod`。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`6dffbd86`（`dispatch-e-integration` HEAD，已 `git rev-parse` 确认）。
- worktree：`/Users/geek/workspace/skiff-e2a-actor-submit`，branch `actor-task-submit`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要（提交侧，不含执行）

1. runtime 提交路径扩展：actor-method target 在提交前从当前受认证 actor handle /
   registry entry 取得并完整编码 `ActorActivationSnapshot`（key / create 输入 +
   expected-type plan，不保存 Actor 内存字段）；缺少 entry 或 create 输入不可恢复 →
   提交明确失败（definite rejection），不产生 task；receiver / 参数 / timing 仍各只
   求值一次。
2. wire：`task.submit.request` 为 actor target 承载 snapshot（task-control
   `ActorActivationSnapshot` 的 wire 投影；字段命名与既有帧风格一致）；corpus 增加
   actor target 帧；函数 target 帧 byte-exact 不变。
3. compiler：核对 actor 方法 dispatch target 检查是否完整。
4. router：保持 `unsupportedTarget` 拒绝不动（E2b 移除）；仅做 wire 解析不破坏现状
   的机械适配。
5. 测试：提交侧正例 / 负例 / 函数 target 回归 / wire corpus / 既有测试全绿。

## 预检结论（只读，锚定 6dffbd86）

- D4 `runtime/eval/src/task_ops.rs` 的 `encode_task_actor_method_payload` 已解析
  receiver（ActorRef 或当前 actor execution frame）、校验 method identity、做方法参数
  recoverable gate，并构造 `ActorMethodTaskTargetControl`（actor_ref / declaration_owner /
  abi / implementation / method），**但没有任何 snapshot（key / create 输入 / expected
  plan）**。
- task-control C1 已定义 `ActorActivationSnapshot { key: RecoverablePayload,
  create_input: RecoverablePayload, expected_type_plan: RecoverableExpectedTypePlan }`
  （artifact-model 形式）并支持 Mongo 存取；router 目前对 actorMethod 目标在 record
  构造前以 `unsupportedTarget` definite 拒绝，因此 E2a 只需保证 wire 解析兼容。
- runtime actor 数据面：`ActorInstanceHandle` 只暴露 fence（logical key + epoch + abi /
  implementation / declaration owner），**不保留 bootstrap / create 输入**；
  `ActorExecutionFrame` 是 eval 侧唯一能访问 `ActorInstanceStore` 的入口（Interpreter /
  EvalContext 不持有 store）。`std.actor.get` native 路径产生 canonical JSON array 的
  create args payload，随 `activateInitial` 到达 runtime 并执行 `create` 后即丢弃。
- expected-type plan 数据面：runtime 只能从 linked program 的
  `LinkedActorCreateMethod.parameters` 构造 runtime 形式
  `RuntimeRecoverableExpectedTypePlan`；artifact 形式 `RecoverableExpectedTypePlan`
  （含 identity tables）在 runtime linked image 中不存在，也没有 runtime→artifact
  bridge。task-control 的 store 模型字段属于 C1 且 E2b 拥有 store/执行衔接，E2a 不改
  task-control。
- wire：`TaskSubmitRequestFrameHeaderV2.actor_method` 已是
  `TaskActorMethodTargetFrameMetadata`（actor_ref / declaration_owner / 三 identity），
  无 snapshot 字段；corpus `task.submit.request.actorMethod` 帧与
  `runtime/transport/src/protocol/task/tests.rs` 构造点需同步更新。函数 target 帧不碰。
- compiler：create 内 self dispatch 禁止（source + IR 双层，含 dispatch 测试）、void/null
  返回、receiver 必须 actor 类型均已覆盖；recoverable 参数没有静态检查，现行 enforce
  点是 runtime 提交前 recoverable encode gate（D4 方法参数 gate；E2a 增加 create 输入
  gate）。参考文档矩阵把“不可恢复参数”列在编译层负例，但 D 批次实际落点是 runtime
  fail-closed；E2a 不新增编译器静态 recoverable 检查（避免超出本节点设计范围），以
  runtime gate + 负例测试补足，并在交接中说明。

## 关键实现决策（本叶子执行范围）

- **snapshot 来源 = 当前 Runtime 内经认证的 live incarnation**：
  - `self.method(...)`：当前 actor execution frame 的 handle（受认证的当前实例）；
  - `actor.method(...)`：同一 frame 的 `ActorInstanceStore` 按 logical key + epoch
    精确查找；找到且已 admitted、非 upgrading 才可用；
  - 无可信本地 incarnation（无 frame 且 store 无该 ref）→ 提交前 definite rejection
    （`RuntimeError::ProviderUnavailable` / 新明确错误），不产生 task.submit.request。
  - `ActorInstance` 在 materialization 时冻结 create input（bootstrap payload bytes），
    这是“registry entry 创建输入”在提交侧的可信副本；不保存任何 Actor 内存字段。
- **snapshot 编码**：
  - `key`：Actor logical key 六字段的 canonical JSON bytes（字段拼写与
    `ActorLogicalKeyFrameHeader` 一致）→ base64；
  - `createInput`：创建时保留的 canonical JSON array（create args）→ base64；
    create-less actor 为空数组 bytes；
  - `expectedTypePlan`：`RuntimeRecoverableExpectedTypePlan`（create parameters
    record，runtime 形式）的 serde JSON。这是 task-control `ActorActivationSnapshot`
    的 wire 投影；E2b 拥有 runtime 形式与 store 形式的衔接。
- **recoverable gate**：create input 非空时，先按 create parameters plan 从 wire JSON
  解码为 runtime values，再走 owner-internal recoverable encode（与 D4 方法参数 gate
  同机制）；任一失败 → definite rejection。create-less actor 空输入恒可恢复。
- **求值顺序不变**：receiver / 参数求值一次 → timing 求值一次 → snapshot 冻结 +
  encode → TaskId → wire。snapshot 失败发生在任何 task.submit.request 之前。
- **wire 投影**：
  - request-contract 新增 `ActorActivationSnapshotControl { key: String,
    create_input: String, expected_type_plan: serde_json::Value }`，挂在
    `ActorMethodTaskTargetControl.activation`（必填）；
  - transport 新增 `TaskActorActivationSnapshotFrameMetadata`（`key` / `createInput` /
    `expectedTypePlan`，camelCase，deny_unknown_fields），挂在
    `TaskActorMethodTargetFrameMetadata.activation`（必填）；
  - host/transport encoder 机械映射；wire validate 在 actorMethod 分支要求
    `activation` 存在且字段合法。
- **corpus**：更新 `task.submit.request.actorMethod` 帧（含 snapshot）并新增一条
  actor target snapshot 帧（如 `task.submit.request.actorMethod.snapshot`）覆盖必填
  路径；`task.submit.request.function` 及其余函数 target 帧 frameHex 不变。
- **router**：`router/src/task/sink.rs` 的 `unsupportedTarget` 拒绝不删不改；wire
  codec 改动仅要求 router 测试 corpus 消费仍通过。

## 禁止

- 不改 router 执行 / admission（get-or-activate 是 E2b）；不删
  `unsupportedTarget`。
- 不改 task-control store 模型 / reducer / mongo（C1 冻结；E2b 衔接）。
- 不改 `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**` 既有文件
  （本叶子文件为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。

## 自验收矩阵（实际证据）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| actor 提交前冻结 key/create 输入/expected plan | `task_ops.rs` `actor_activation_snapshot` / `create_request_recoverable_expected_plan`；`actor_instance.rs` `ActorActivationFacts` 冻结 create input | wire 帧 `activation` 必填；函数 target 路径无 snapshot；actor_submit 正例断言 key/createInput/expectedTypePlan | `cargo test -p skiff-runtime-eval --lib task_ops::tests::actor_submit`（5/5） |
| 缺少 entry / create 输入不可恢复 → definite rejection 且无 task | `authenticated_actor_handle`（无 frame / store 未命中 → ProviderUnavailable）；`gate_create_input` 提交前失败 | 负例断言 submissions 为 0 | 同上 |
| receiver/参数/timing 各只求值一次 | 求值顺序不变；snapshot 在 receiver/参数/timing 求值之后、TaskId 之前 | `actor_method_submit_receiver_argument_evaluated_once` 断言仅 1 次提交；既有 D4 嵌套 dispatch 计数测试 | 同上 + `task_ops::tests::canonical` |
| wire 承载 snapshot；函数 target 帧 byte-exact | `TaskActorActivationSnapshotFrameMetadata` + host/transport encoder + `validate_activation_snapshot`；corpus 函数帧 hex 未变 | `git diff frames.json` 仅 actorMethod 帧变化 | `cargo test -p skiff-runtime-transport`（141 lib + corpus） |
| corpus 增加 actor target 帧 | `frames.json` 新增 `task.submit.request.actorMethod.snapshot`，actorMethod 帧含 activation；5 个 REQUIRED_FRAMES 同步 | transport / runtime / router corpus 测试通过 | transport corpus；`cargo test -p runtime --test w_model_task_consumer --test h_task_parent_cut_corpus`；`cargo test -p skiff-router --test w_model_task_consumer --test task_repair_direction` |
| compiler 检查核对 | 预检结论：create 内 self dispatch（source+IR 双层）、void/null、receiver actor 类型均覆盖；recoverable 参数 enforce 点是 runtime gate（既有状态） | lowering 两条 create dispatch 负例测试通过 | `cargo test -p skiff-compiler --test dispatch_grammar`（5）、`cargo test -p skiff-compiler-source --lib dispatch_source_semantics`（14）、`cargo test -p skiff-compiler-lowering --lib`（77） |
| router unsupportedTarget 保持 | `router/src/task/sink.rs` 无 actorMethod admission 改动 | `rg unsupportedTarget router/src` 仍存在 | `cargo check -p skiff-router`；`cargo test -p skiff-router --test task_control_unit --test dispatch_admission_corpus` |
| 既有测试全绿 | 受影响 crate 聚焦测试 | 无回归 | 见验证记录 |

## 实际写集

```text
doc/implementation/dispatch-e2a-actor-submit-leaf.md
runtime/capability-context/src/{lib.rs,outbound_control.rs}
runtime/eval/Cargo.toml                        # base64 移入正式依赖
runtime/eval/src/actor_executor/actor_concurrent_continuation.rs
runtime/eval/src/actor_instance.rs             # ActorActivationFacts + handle_for_actor_ref
runtime/eval/src/task_ops.rs                   # snapshot 冻结 + create recoverable gate
runtime/eval/src/task_ops/tests.rs
runtime/eval/src/task_ops/tests/actor_submit.rs # E2a 提交侧正/负例（5 例）
runtime/host/src/host/router_session/task_submit.rs
runtime/host/src/host/router_session/tests/h_task_parent_cut.rs
runtime/request-contract/src/{lib.rs,outbound.rs,outbound_control.rs}
runtime/transport/src/{control_mapper.rs,protocol.rs}
runtime/transport/src/protocol/task.rs
runtime/transport/src/protocol/task/tests.rs
runtime/transport/testdata/task-wire/frames.json
runtime/transport/tests/{task_wire_corpus.rs,w_model_task_corpus.rs}
runtime/tests/{w_model_task_consumer.rs,h_task_parent_cut_corpus.rs}
router/tests/{w_model_task_consumer.rs,task_repair_direction.rs}
```

未改：task-control（C1 冻结）、`router/src/task/sink.rs`（unsupportedTarget 保留）、
`doc/reference/`、`doc/architecture/`、`doc/implementation/**` 既有文件。

## 验证记录

- `cargo check`：request-contract / capability-context / transport / eval / host /
  router 全部 PASS。
- `cargo test -p skiff-runtime-transport`：141 lib + 全部 integration corpus PASS。
- `cargo test -p skiff-runtime-eval --lib`：463 PASS（新增 actor_submit 5 例）。
- `cargo test -p skiff-runtime-host --lib`：428 PASS（新增 host wire 编码 1 例）。
- `cargo test -p runtime --test w_model_task_consumer --test h_task_parent_cut_corpus`：PASS。
- `cargo test -p skiff-router --test task_control_unit --test w_model_task_consumer
  --test task_repair_direction --test dispatch_admission_corpus`：PASS。
- compiler：dispatch_grammar 5/5、dispatch_source_semantics 14、dispatch_targets 7、
  compiler-lowering 77 全 PASS。
- `git diff --check` PASS。

## 交接注意事项

- E2b 需要把 wire 的 `expectedTypePlan`（runtime 形式）与 task-control store 的
  artifact 形式 `RecoverableExpectedTypePlan` 衔接；E2a 不改 C1 模型，此衔接归 E2b。
- 编译器“recoverable 参数”静态检查缺失是既有状态（D 批次选择 runtime fail-closed）；
  E2a 不补，作为非阻塞 follow-up 上报主 Agent。
- 提交侧只支持本 Runtime 内 live incarnation 作为 snapshot 来源；E2b 引入
  get-or-activate 后可评估是否需要在 router 增加 registry entry 读取面。
