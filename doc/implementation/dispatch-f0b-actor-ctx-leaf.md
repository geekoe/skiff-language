# Leaf Task: F0b actor-handle 上下文中的 `dispatch actor.method(...)` 提交（外部上下文 snapshot 冻结）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（Actor-method target 节：
  提交前必须从当前受认证 actor handle / registry entry 取得并完整编码
  `ActorActivationSnapshot`；缺少 entry 或 create 输入不可恢复 → 提交失败，不得
  创建以后必然无法激活的 task；Submission And Visibility 的求值顺序与 definite
  rejection）。
- 用户面契约：`doc/reference/dispatch.md` §4（actor 方法 `actor.method(...)` /
  `self.method(...)`；recoverable 参数提交前 fail closed；提交顺序 receiver /
  参数 / timing 各只求值一次）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration`；F0b 是 E2a actor_task_submit 提交侧的续接
  节点，只改 runtime/E2a 提交侧，不碰 router 执行侧）。
- 已合并叶子：E2a `dispatch-e2a-actor-submit-leaf.md`（提交侧只支持 actor
  execution frame；交接注意事项记录“提交侧只支持本 Runtime 内 live incarnation
  作为 snapshot 来源；E2b 引入 get-or-activate 后可评估是否需要在 router 增加
  registry entry 读取面”）、E3a `dispatch-e3a-e2e-leaf.md`（E2E 缺陷 3：普通
  HTTP request 中 `dispatch actor.method()` 被 definite reject，fixture 改为
  actor 方法内部 `dispatch self.increment()` 绕过）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`62d8ee996361cbf74ab3429e378c6cc3a6db309e`
  （`dispatch-e-integration` HEAD，已 `git rev-parse` 验证；集成 worktree 后续
  推进不影响本节点锚定基线）。
- worktree：`/Users/geek/workspace/skiff-f0b-actor-ctx`，branch
  `actor-submit-context`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

1. 预检：确认外部上下文里的 `ActorRef` 运行时值数据面（logical identity /
   implementation / activation snapshot）、本 Runtime 对未 materialized actor
   能否拿到 registry entry（本地 actor store / router registry 查询面 / handle
   自带），以及 E2a snapshot 来源在哪些入口可用。
2. 实现：`dispatch actor.method(...)` 在外部上下文（HTTP handler 等）可提交——
   按权威语义从 handle / registry entry 冻结 snapshot；entry 缺失或 create
   输入不可恢复 → definite rejection（复用 E2a 的 `ProviderUnavailable` 语义），
   不产生 task；receiver / 参数 / timing 仍各只求值一次。
3. 若外部上下文确实拿不到 registry entry（v1 actor handle 不携带 create 输入且
   无查询面），该路径按权威语义就是 definite rejection——拒绝理由收敛为明确、
   可文档化的错误，并在本叶子说明；不得预先假设，先确认 handle 数据面。
4. 测试：
   - 单元：外部上下文提交成功（snapshot 正确）与 definite rejection 负例；
   - 集成/E2E：E3a durable-task E2E fixture 增加 HTTP 入口
     `dispatch actor.method(...)`（成功路径）；E2a 现有 actor frame 提交回归。
5. 需要改 E2a/runtime 提交侧时直接改（本节点是 E2a 的续）；不改 router 执行侧。

## 预检结论（只读，锚定 62d8ee99）

### ActorRef 运行时值数据面

- `runtime/request-contract/src/actor_ref.rs`：`ActorRef` 只携带六字段 logical
  identity（service_id / actor_type_identity / actor_id_type_identity /
  actor_id_encoding_version / canonical_actor_id_key_bytes / actor_id_hash）与
  `Option<u64>` pinned epoch。**不携带 implementation identity，也不携带
  create 输入 / activation snapshot**。
- `std.actor.get`（`runtime/native/src/dispatch/actor.rs`）→
  `actor.getOrCreate.request`（router 侧 `handle_get_or_create`）→
  `ActorGetOrCreateResponseFrameHeader` 只回 `actor_ref`；create args 只作为
  bootstrap payload 进入 router 的 `ActorOwnerEntry.create_input`，不回传。
- `actor.find.request`（router `handle_find`）只回 `Option<ActorRef>`，无 create
  输入。**不存在任何返回 registry entry create input 的 router 查询面**。
- 结论：v1 actor handle 不携带 create 输入，router 无 entry 读取面。因此
  “registry entry 的创建输入”在提交侧唯一可信副本是本 Runtime
  `ActorInstanceStore` 在 materialization 时冻结的
  `ActorActivationFacts.create_input`（E2a 已确立的结论）。

### 本 Runtime 对未 materialized actor 的 entry 可及性

- `runtime/eval/src/actor_instance.rs`：`ActorInstanceStore::handle_for_actor_ref`
  按 logical key + epoch 精确解析 live、admitted、非 upgrading 的
  `ActorInstanceHandle`；`ActorInstanceHandle::activation_create_input()` 返回
  冻结的 create 输入。该方法是 crate-private，且**当前唯一可达入口是
  `ActorExecutionFrame`**（`current_handle` / `find_handle`），而 frame 只存在于
  actor execution frame 内。
- `RuntimeHost`（`runtime/host/src/host/runtime_host.rs`）持有
  `Arc<ActorInstanceSessionTracker>`（内含 `Arc<ActorInstanceStore>`）；普通
  HTTP/assembly request 的 `ProgramExecutionContext` 由
  `runtime/host/src/host/request_entry/assembly.rs` 构造
  `RuntimeAssemblyEvalAdapterContextInput`，当前**不传入 actor instance
  store**，因此外部上下文拿不到 entry facts。
- `actor.find` / `actor.getOrCreate` 均由 router 处理；runtime 侧没有 router
  registry 查询通道可拿 create 输入（新增查询面会落在 router 执行侧，本节点
  禁止）。

### E2a snapshot 来源可用入口

- `self.method(...)`：当前 actor execution frame 的 `current_handle()`（受认证
  当前实例）；
- `actor.method(...)` 于 actor frame 内：`frame.find_handle(actor_ref)` →
  `ActorInstanceStore::handle_for_actor_ref`；
- 外部上下文（HTTP handler 等）：无 frame、无 store 引用 →
  `authenticated_actor_handle` 直接 `ProviderUnavailable`（E2a 边界，E3a 缺陷 3）。

### 实现可行性结论

- 外部上下文提交成功路径**可闭合且最小**：把 RuntimeHost 已持有的
  `Arc<ActorInstanceStore>` 透传进普通 request 的 `ProgramExecutionContext`，
  提交侧在无 frame 时回退到 `store.handle_for_actor_ref(actor_ref)`。该 store
  只包含 router 认证激活路径（`actor.getOrCreate` / `actor.owner.control` /
  `actor.owner.invoke`）materialize 的 incarnation，因此是“当前受认证 actor
  handle / registry entry”在提交侧的既有可信副本；不需要任何 router / wire /
  grammar 改动。
- 外部上下文里 actor 不在本 Runtime live store（从未在此 Runtime materialize、
  已逐出/升级/未 admit）→ 无 entry facts → 按权威语义 definite rejection
  （`ProviderUnavailable`），错误理由在本叶子明确记录；不产生 task。
- 不改 router 执行侧、不改 wire 帧、不改 compiler 语法、不改 task-control
  store 模型。

## 关键实现决策（本叶子执行范围）

1. **eval 上下文携带本地 actor instance store**：
   - `ProgramExecutionContext` 新增
     `actor_instance_store: Option<Arc<ActorInstanceStore>>`（默认 `None`，既有
     测试/构造路径零行为变化）；新增 `with_actor_instance_store(store)` 与
     `actor_instance_store()` 访问器；`Clone` 与 `switch_activation_owner` 保持
     store（store 是 Runtime 级事实，不随 activation owner 切换重置）。
   - `RuntimeAssemblyEvalAdapterContextInput` /
     `RuntimeAssemblyExecutionContext` 新增 `actor_instance_store:
     Arc<ActorInstanceStore>`，由 `RuntimeHost` 两处构造点
     （`task_direct_request_on_active_assembly` /
     `runtime_assembly_eval_adapter_context`）用
     `Arc::clone(self.actor_instances.store())` 填充，并在
     `program_execution_context()` 挂到 eval 上下文。
   - 测试构造点 `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs`
     同步补字段。
2. **task_ops 外部上下文提交**：
   - `authenticated_actor_handle`：有 frame → 保持 E2a 现有行为不变；无 frame →
     用 `context.context.actor_instance_store()` 查
     `store.handle_for_actor_ref(actor_ref)`；未命中 → `ProviderUnavailable`，
     reason 明确记录“external actor handle has no live authenticated local
     incarnation; no task was created”。
   - snapshot 冻结、create recoverable gate、wire 编码、求值顺序全部复用 E2a
     既有实现（`actor_activation_snapshot` / `gate_create_input` 不改语义）。
3. **E2a 单测**：
   - 新增正例：无 frame、context 挂 store、actor live → 提交成功，断言
     key / createInput / expectedTypePlan 与 submissions == 1。
   - 新增负例：无 frame、store 未命中（或未挂 store）→
     `ProviderUnavailable`、submissions == 0。
   - 既有 actor frame 正/负例原样保留（回归）。
4. **E2E fixture / 探针**：
   - `durable-task-e2e-live/main.skiff` 新增 `submitActorDirect`：HTTP handler
     内 `std.actor.get<Counter>(tag)` 后直接
     `dispatch actor.increment()`，TaskRef 入 `TaskEntry`；
   - `http.yml` 增加 `/submit-actor-direct` 入口；
   - `scripts/lib/durable_task_live_fixture.mjs` 的
     `DURABLE_TASK_LIVE_ENTRYPOINTS` 增加同名字段；
   - `router/tests/durable_task_e2e_live_probe.rs` 新增 Scenario 9：真实 HTTP
     提交成功、status `succeeded`、actor count == 1、`task.submit.request` 与
     `actor.owner.invoke` 计数增加（证明外部上下文提交 + owner 执行）。

## 禁止

- 不改 `doc/reference/` 与 `doc/architecture/`；不改 `doc/implementation/**` 既有
  文件（本叶子文件为新增）。
- 不改 wire / compiler 语法；不改 router 执行侧（含 router registry 查询面、
  `task/sink.rs`、get-or-activate）；不改 task-control store 模型。
- 不 push、不写共享集成分支、不动共享主 worktree、不跑完整 gate。

## 自验收矩阵（实际证据）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 外部上下文从受认证 handle / entry 冻结 snapshot 并提交 | task_ops `authenticated_actor_handle` 无 frame 回退 `actor_instance_store`；`ActorActivationSnapshotControl` 复用 E2a 编码 | actor_submit 外部正例断言 key/createInput/expectedTypePlan 与 submissions == 1 | `cargo test -p skiff-runtime-eval --lib task_ops::tests::actor_submit` |
| 缺少 entry → definite rejection，不产生 task | 无 frame 且 store 未命中 → `ProviderUnavailable`；reason 明确 | 外部负例断言 submissions 为 0 | 同上 |
| create 输入不可恢复 → definite rejection（既有 gate 复用） | `gate_create_input` 提交前失败路径不变 | 既有 unrecoverable 负例通过 | 同上 |
| receiver / 参数 / timing 各只求值一次 | 求值顺序不变；snapshot 冻结在 receiver / 参数 / timing 之后、TaskId 之前 | 既有 evaluated-once 回归通过 | 同上 |
| E2a actor frame 提交回归 | frame 分支代码原样保留 | 既有 5 例全部通过 | 同上 |
| host 把本地 store 透传普通 request 上下文 | `RuntimeAssemblyEvalAdapterContextInput.actor_instance_store` + `program_execution_context` 挂载 | host 测试构造点同步；无其他构造点残留 | `cargo check -p skiff-runtime-host`；host 聚焦测试 |
| E2E HTTP 入口 `dispatch actor.method(...)` 成功 | fixture `submitActorDirect` + http.yml + harness entrypoint + 探针 Scenario 9 | 探针断言 status/actor-count/帧计数 | `node scripts/check-durable-task-e2e-live.mjs` |

## 实际写集（commit 后与交接报告一致）

```text
doc/implementation/dispatch-f0b-actor-ctx-leaf.md          # 本叶子
runtime/eval/src/program_execution.rs                     # ProgramExecutionContext 携带 actor instance store
runtime/eval/src/task_ops.rs                              # 外部上下文 authenticated_actor_handle 回退 store
runtime/eval/src/task_ops/tests/actor_submit.rs           # 外部上下文正/负例 2 例 + 既有 E2a 回归
runtime/host/src/eval_capability_adapter/assembly_execution_context.rs  # context input/execution 透传 store
runtime/host/src/host/request_entry/assembly.rs           # RuntimeHost 两处构造点注入 store
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs  # 测试构造点同步
scripts/lib/durable_task_live_fixture.mjs                 # E2E entrypoint map 增加 submit-actor-direct
test-runner/fixtures/durable-task-e2e-live/main.skiff     # submitActorDirect（HTTP 内 dispatch actor.increment()）
test-runner/fixtures/durable-task-e2e-live/http.yml       # /submit-actor-direct 入口
router/tests/durable_task_e2e_live_probe.rs               # Scenario 9（外部上下文提交成功）
```

## 验证记录

```text
cargo check -p skiff-runtime-eval -p skiff-runtime-host                          PASS
cargo test -p skiff-runtime-eval --lib                                           465/465 PASS
  task_ops::tests::actor_submit                                                   7/7 PASS
  （新增：actor_method_submit_external_context_freezes_snapshot_and_submits_once /
        actor_method_submit_external_context_missing_incarnation_rejects_before_task；
       既有 E2a 5 例原样回归）
cargo check --tests -p skiff-runtime-host                                        PASS
cargo test -p skiff-runtime-host --lib                                           429/429 PASS
cargo check --tests -p skiff-router                                              PASS（含探针编译）
cargo test -p skiff-router --test task_control_unit --test task_actor_method_execution
  --test dispatch_admission_corpus                                               18+10+2 PASS
node scripts/check-durable-task-e2e-live.mjs                                     PASS
  真实 compiler artifact（submitActorDirect 编译通过）+ 真实 router/runtime +
  探针 Scenario 9：HTTP /submit-actor-direct → task.submit.request → succeeded +
  actor count 1 + actor.owner.invoke 增加
git diff --check                                                                 PASS
```

## 磁盘与清理记录

- E2E 首次运行时共享磁盘耗尽（98%→100%）：失败发生在 config-snapshot 工具链编译
  （无代码原因）。清理本 worktree 自有 `build/cargo-target/debug/incremental`
  （约 7Gi，仅速度缓存，不影响已编译产物）后重跑通过。
- E2E harness 自身 finally 已 drop 探针 DB、释放端口并删除临时目录；未触碰其他
  agent 的 worktree / target / /tmp 缓存。

## 交接注意事项

- 外部上下文成功路径依赖 actor 在本 Runtime 的 live incarnation；未 live
  （从未在本 Runtime materialize、逐出、升级中、未 admit）→ definite
  rejection。v1 actor handle 不携带 create 输入、router 无 registry entry
  读取面，因此“跨 Runtime 冷 entry 提交”仍不可用；该边界已在本叶子记录，属于
  权威文档“缺少 entry 提交失败”的可接受实现。
- 完成时把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
  `/root/dispatch_e_integration`，并通知主 Agent `/root`。
