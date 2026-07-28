# P5-F445H-E4R0 evaluator closure execution preflight result

状态：`READY_FOR_E4R_DAG`。

冻结 production checkpoint 为 `99acfd13`。当前 worktree HEAD `f26244a3` 相对该 checkpoint
只新增本 preflight task；production 与 tests 无差异。J1 已证明 O1–O6 prepared-operation
owner prerequisite 闭合，本次只读检查也没有发现必须回改 O1–O6 core、E1/E2/E3 public API、
host/I6 或公共契约的 blocker。

E4R 冻结为三波五节点：

```text
99acfd13
   |
   v
E4R1 evaluator spine + checkpoint + non-O3 actual-Pending
   |
   +-------------------+-------------------+
   v                   v                   v
E4R2 timeout/catch   E4R3 concurrent     E4R4 stream/current-scope
owner closure        + Actor closure      + activation actual-Pending
   +-------------------+-------------------+
                       |
                       v
              E4R5 combined integration/acceptance
```

不得把 R2/R3/R4 直接建在 `99acfd13` 上。它们必须从 R1 的同一个 implementation commit
分支；R1 是唯一编辑 `runtime/eval/src/eval_context.rs` root/import/match-arm 的节点。

## 1. 预检事实与具体裁决

### 1.1 当前 RED 与结构

- `runtime/eval/src/eval_context.rs` 为 2159 行，四个
  `F445H-E4 evaluator integration is required ...` arm 仍 fail closed。
- production 仍有九组 pre-suspend/resume：
  - `Emit` 三处；
  - remote interface；
  - callback capability；
  - activation-relative service；
  - Actor dispatch；
  - legacy service dependency；
  - native。
- `native_call_suspends` 和唯一静态 `may_suspend` 决策仍在。
- DB operation/transaction/lease 已由 O6 通过 `program_db::wait::await_operation` 消费 E3；
  `DbQuery` 是同步例外。E4R 不重新实现 DB owner，也不复制 O6R13 matrix。
- `program_stream.rs` 的 Actor consumer `next()` 已使用 `await_if_pending`，但只传显式 stream
  signal 与 `execution.cancellation_token()`，尚未消费完整 current
  `execution_scope().cancellation_signals()` 和 owner deadline。
- `program_invocation.rs` 两个 response-stream loop 与
  `async_stream_cancel.rs` 的 provider wait/publication 仍使用 request-like token/generic
  deadline，未闭合 local/inherited owner。
- `eval_context.rs`、`program_stream.rs`、`program_invocation.rs`、
  `async_stream_cancel.rs` 分别为 2159、1452、1797、1992 行；新增责任必须进入 child module。

### 1.2 五个必须判断的问题

| 问题 | 冻结裁决 | 理由 |
| --- | --- | --- |
| timeout statement/expression、owner materialization、ordinary catch 是否拆开 | **不拆，全部归 R2** | statement/expression 共用同一 child scope owner；internal carrier 只有 timeout wrapper 能物化，ordinary catch 本身无需新语义，只能在同一真实 evaluator matrix 证明 inner miss、outer hit 与 parent 恢复。 |
| concurrent statement/value 与 Actor bridge 是否拆开 | **不拆，全部归 R3** | E2 lane outcome 与 E3 child-frame complete/abandon 必须在同一 lane future 中结算；拆开会留下“非 Actor 可运行、Actor 中间态错误”或重复 lane state machine。 |
| owner-aware checkpoint 跟哪个节点 | **root/checkpoint 归 R1；E2 lane checkpoint 保持 E2；stream wait checkpoint 归 R4** | timeout 正确物化和纯 CPU T10 都依赖 checkpoint 先落地；root call-site 的 pre/post-await checkpoint 又与 actual-Pending 使用同一 import/root 区，单独再建串行节点只增加一次 root ownership handoff。 |
| 九处 actual-Pending 能否与 timeout/concurrent 并行 | **不能直接并行编辑 root** | 九组、四个新 arm及 checkpoint 都编辑同一 `eval_context.rs`。R1 一次性把 root 改成薄路由并关闭其中八组；activation 第九组被预分配到独立 child file，由唯一拥有 `async_stream_cancel.rs` 的 R4 原子关闭。R2/R3/R4 后续均不编辑 root。 |
| stream current scope/cleanup 能否独立并行 | **可以，在 R1 后与 R2/R3 并行** | stream core 不需要修改 `eval_context.rs`；R4 只修改预分配的 activation child 以及三个 stream owner root/child。为保持 `async_stream_cancel.rs` 单一终态 owner，activation-relative actual-Pending 同 R4，不由 R1/R4 并发编辑该文件。 |

R1 同时是 shared structural checkpoint，但不是零验证重排：

1. 它必须把 function/block、loop、literal/chunk 与本节点 await 迁到
   `ProgramExecutionContext::checkpoint`，真实关闭 T10；
2. 它必须关闭八组 root actual-Pending，并覆盖 Ready/Pending、同步例外和
   `createFromStream` 组合；
3. 它把 timeout/concurrent/activation 保持明确 fail-closed或旧行为的实现移入三个预分配
   child surface，随后 R2/R3/R4 只改各自 child；
4. focused selector 至少执行 10 个真实 evaluator tests，不能只证明模块能编译。

### 1.3 无用户语义决策

没有新的用户决策。以下语义均已由父节点冻结：

- 只有第一次真实 `Pending` 才提交并释放 Actor segment；
- `Ready` 不释放，且没有语言级 yield；
- local timeout 由精确 wrapper 物化，inherited/request terminal 不物化；
- equal deadline 由最外层 owner 唯一物化；
- ancestor cancel 优先；
- concurrent winner/source order、outer priority 与 stream cleanup owner 不变；
- I6 仍单独负责 request/root boundary 映射。

## 2. DAG 节点、依赖与解除条件

| 节点 | 直接前置 | 本节点完成后解除 | 波次 |
| --- | --- | --- | --- |
| `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint` | `99acfd13` production tree、J1 PASS | R2、R3、R4 的共同 root/module checkpoint | 1 |
| `P5-F445H-E4R2-timeout-catch-owner-closure` | R1 implementation commit | R5 timeout/catch input | 2，可与 R3/R4 并行 |
| `P5-F445H-E4R3-concurrent-actor-evaluator-closure` | R1 implementation commit | R5 concurrent/Actor input | 2，可与 R2/R4 并行 |
| `P5-F445H-E4R4-current-scope-stream-activation-closure` | R1 implementation commit | R5 stream与第九组 actual-Pending input | 2，可与 R2/R3 并行 |
| `P5-F445H-E4R5-combined-integration-acceptance` | R2、R3、R4 implementation commits | I6 前置；不代表 F445H/Phase 05 完成 | 3 |

R2/R3/R4 的 production 写集不重叠。它们的测试声明也由 R1 预分配到不同 child file，
不允许叶子节点重新编辑共享 module/import root。

## 3. 精确写集与 module 结构

下面 production/test 文件是后继任务允许写集；每个节点另可写自己的 task/result 文档。
任何 Cargo、manifest、lockfile、artifact/compiler/linker、capability-context、native owner、
service-db、Router 或 host 文件均不在写集。

### 3.1 R1 evaluator spine、checkpoint 与八组 actual-Pending

Production：

- `runtime/eval/src/eval_context.rs`
- 新增 `runtime/eval/src/eval_context/checkpoint.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending/activation.rs`
- 新增 `runtime/eval/src/eval_context/timeout.rs`
- 新增 `runtime/eval/src/eval_context/concurrent.rs`
- `runtime/eval/src/assembly_execution/mod.rs`

Test/test-only fixture：

- `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs`
- 新增 `runtime/eval/src/assembly_execution/ordinary/test_runtime/scoped_execution.rs`
- `runtime/eval/src/program_execution/execution_scope_tests.rs`
- 新增
  `runtime/eval/src/program_execution/execution_scope_tests/evaluator_checkpoint.rs`
- 预声明并新增
  `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`
- 新增 `runtime/eval/src/eval_context/actual_pending/tests.rs`
- 新增 `runtime/eval/src/eval_context/concurrent/tests.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests.rs`
- 新增
  `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs`
- 预声明并新增
  `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_concurrent.rs`

结构冻结：

- `eval_context.rs` 只保留四个 arm 的薄转发、既有 dispatch和 module声明；
  timeout/concurrent child 在 R1 仍返回当前稳定 fail-closed diagnostic。
- `actual_pending/activation.rs` 在 R1 保持当前 activation pre-suspend 行为，作为 R4 唯一终态
  handoff；其它八组不得继续保留 pre-suspend。
- `checkpoint.rs` 只组合现有 `ExecutionCheckpoint` / `ExecutionCheckpointKind`，不得新增 E1
  kind 或改 instruction budget。
- `assembly_execution/mod.rs` 只允许 callback prepared consumer 的 crate-private 窄接线；
  不得修改 public export、boundary error或 provider owner core。
- ordinary/Actor 共用 `TestExecutionControl` 当前没有 `execution_scope` / `derive_scope`；
  `scoped_execution.rs` 只修复这个 test-only fixture。不得因此给 production control 加 fallback。

明确禁止并行 surface：

- 本节点运行期间不得有其它节点编辑 `eval_context.rs`、`assembly_execution/mod.rs`、
  ordinary `test_runtime.rs`、两个共享 test module declaration 文件。
- R1 完成后这些 root 冻结；后继只能修改预分配 child。

`runtime/eval/src/lib.rs`：**不修改**。`eval_context.rs` 自己声明 child；现有 crate module 已足够。

### 3.2 R2 timeout、owner materialization 与 catch closure

Production：

- `runtime/eval/src/eval_context/timeout.rs`

Tests：

- `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`

结构与禁止面：

- child 对 `EvalContext` 定义 `pub(super)` async helper，使用 parent/child context value ownership；
  不原地替换 parent control。
- 只消费现有 `derive_timeout_child`、`execution_scope`、`RuntimeError::scope_terminal`、
  `ScopeTerminalCarrier::is_owned_by` 与既有 exception/catch identity。
- ordinary `eval_program_catch` 预计无需 production 修改；若实现必须修改
  `exceptions.rs`、`error.rs` 或 E1 scope owner，立即停止。
- 不编辑 `eval_context.rs`、R1 actual-Pending、R3 concurrent或任何 stream file。

`runtime/eval/src/lib.rs`：**不修改**。

### 3.3 R3 concurrent statement/value 与 Actor bridge

Production：

- `runtime/eval/src/eval_context/concurrent.rs`

Tests：

- `runtime/eval/src/eval_context/concurrent/tests.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_concurrent.rs`

结构与禁止面：

- child 定义真实 `ConcurrentLaneExecutor`，只消费 E2
  `ProjectedLane`、`LaneEvaluation`、`LaneExecutionState`、`LaneCompletion` 与 scheduler result。
- Actor path 只消费 E3 `begin_concurrent -> lane -> resume -> complete/abandon ->
  resume_parent`。
- lane future 必须持有 lane-local state；不得借 outer `Env` / `RequestHeap`。
- Actor normal lane 可从 lane-local heap 为 E3 commit 提供 clone，同时把原
  `LaneExecutionState` 交回 E2 handoff；error/cancel/drop 必须 abandon，不得伪造 normal。
- 不编辑 E2/E3 module、`eval_context.rs`、timeout、actual-Pending或 stream file。

`runtime/eval/src/lib.rs`：**不修改**。

### 3.4 R4 stream current scope、cleanup 与 activation actual-Pending

Production：

- `runtime/eval/src/eval_context/actual_pending/activation.rs`
- `runtime/eval/src/program_stream.rs`
- 新增 `runtime/eval/src/program_stream/current_scope.rs`
- `runtime/eval/src/program_invocation.rs`
- 新增 `runtime/eval/src/program_invocation/current_scope.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- 新增
  `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope.rs`

Tests：

- 新增 `runtime/eval/src/eval_context/actual_pending/activation_tests.rs`
- 新增 `runtime/eval/src/program_stream/current_scope_tests.rs`
- `runtime/eval/src/program_invocation/stream_cleanup_tests.rs`
- 新增 `runtime/eval/src/program_invocation/current_scope_tests.rs`
- 新增
  `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs`

结构与禁止面：

- `activation.rs` 原子地完成 prepare / Ready-or-wait / E3 / finalize；
  `serverStream` 保持同步 Ready。
- 三个 `current_scope.rs` 只组合 current `ExecutionScope` 的完整 cancellation signals、
  effective deadline/owner、现有 stream future与 cleanup guard。
- `async_stream_cancel.rs` 在 E4R 中只有 R4 一个终态 owner；R1 不编辑该文件。
- natural End 是唯一 disarm 路径；所有其它 terminal 依赖既有
  `StreamConsumerCleanup` / supervised lease / stream lifetime RAII。
- 不编辑 `eval_context.rs`、R2/R3文件、capability-context stream cleanup owner或 I6。

`runtime/eval/src/lib.rs`：**不修改**。

### 3.5 R5 combined integration/acceptance

Production：**无**。

Tests：

- 新增 `runtime/eval/tests/f445h_e4r_combined.rs`

Result：

- `P5-F445H-E4R5-combined-integration-acceptance-result.md`

禁止面：

- 不修 production。若 combined RED 暴露 defect，退回唯一责任叶子并只使对应证据失效；
  R5 不做影子修复。
- 不修改 Cargo/manifest/lockfile，不运行 stable/live/network/MongoDB。

`runtime/eval/src/lib.rs`：**不修改**。

## 4. Rust privacy、lifetime 与真实消费位置

### 4.1 Child `EvalContext` helper 可行

Rust privacy 允许 `eval_context` 的 descendant module 访问 parent 的 private fields，并在
`impl EvalContext<'_>` 中定义 `pub(super)` helper。现有 `EvalContext` 的 interpreter、
projection、context、execution、heap、env、addr、file、executable 均无需改 visibility。

root match arm必须改变，但只由 R1 一次完成。R2/R3 不再编辑 root，避免相邻四 arm及 import区
的必然冲突。

### 4.2 `async_recursion` / Send

- `exec_program_block`、`exec_program_statement`、`eval_program_expr_ref`、
  `eval_program_expr` 使用默认 `#[async_recursion]`，即 boxed future 必须 `Send`。
- R1/R2 helper 不得使用 `?Send` 降级。
- O1–O6 wait 均为 owned heap/env-free `Send` future；native wait虽然带 lifetime参数，
  仍是 `Future + Send`，且不借 caller heap。
- R3 必须返回现有
  `Pin<Box<dyn Future<Output = LaneCompletion> + Send + 'a>>`；
  不能把 outer mutable borrow捕获进 lane future。
- helper 应在自身 async body内完成 prepare/wait/finalize，不新增向外暴露的 public
  `impl Future` 类型或公共 trait。

### 4.3 E2/E3 的真实消费点

R3 冻结的唯一消费序列：

1. `project_concurrent_plan` 解析真实 linked plan；
2. `run_concurrent_scheduler` 在 lane ready 时调用真实 `ConcurrentLaneExecutor::start_lane`；
3. executor 从 `LaneExecutionState::program_context` 构造 child context；
4. Actor outer 存在时，按 source order claim `bridge.lane(index)`，在真实 evaluator 前
   `resume` 并安装 child frame；
5. Statement执行唯一 direct body，Serial执行完整 block，Tail执行 expression；
6. normal/value 后 E3 `complete`，再把 E2 `LaneCompletion` 交 scheduler；
   error/non-continue/cancel/drop 使用 `abandon`/RAII；
7. scheduler全部结束并关闭 child 后，outer `resume_parent`；
   resume/fence/outer terminal 优先于接纳 lane result。

不得直接调用 E2 fake executor作为 production证据，也不得复制 scheduler ready queue、
winner或 E3 store acquire。

### 4.4 O3 与 stream owner

`async_stream_cancel.rs` 同时包含 activation-relative unary wait 和 provider stream lifecycle。
把它拆给两个并行节点会造成 double-suspend或无 suspend 的中间态，因此：

- R1 只在预分配 `activation.rs` 保持明确旧路径，不编辑 O3 文件；
- R4 同时替换 activation child 和 O3 wait/stream逻辑；
- R4 是该文件唯一 E4R 终态 owner。

### 4.5 现有 child test 结构

- `program_invocation.rs` 已有
  `program_invocation/stream_cleanup_tests.rs`，可扩展真实 response-stream cleanup，并新增
  同目录 `current_scope_tests.rs`。
- `program_stream.rs` 当前有效 focused tests只有四个 prepared-drain tests；大型旧
  `cfg(all(test, any()))` module 不执行，不能作为证据。R4 必须新增有效 child test module。
- `actor_executor::tests` 已有真实 `Fixture`、`execution_frame`、store、pending/drop probe；
  在 `actor_concurrent_continuation_tests.rs` 下挂 descendant test 可复用 private fixture，
  不需要开放 E3 production API。
- `program_execution/execution_scope_tests.rs` 已有 `ScopeAwareControl`、`ScriptedClock`、
  current scope/context fixture。descendant evaluator tests可直接复用
  `with_execution_clock`，不需要扩大 E1 visibility。

## 5. 真实 RED、selector 与测试数门槛

所有节点先用同一 selector执行 `-- --list`，确认主 test binary 的非零数量，再执行
`-- --nocapture`。其它 test binary 的零匹配不计证据。结果必须记录实际测试数。

| 节点 | 真正 RED | selector | 新增测试函数门槛 | 必须穿过的 production入口 |
| --- | --- | --- | ---: | --- |
| R1 | Ready native/service/callback/Actor仍被预释放；四个 CPU/checkpoint路径不产生 owner terminal；普通 test control scope unavailable | `f445h_e4r_spine` | **至少 10** | `Interpreter`真实 linked evaluator、`EvalContext::{exec_program_statement,eval_program_expr,eval_program_call}`、真实 E3 frame、prepared owner wait/finalize |
| R2 | statement/expression timeout仍返回四-arm stable `InvalidArtifact` | `f445h_e4r_timeout` | **至少 8** | 两个真实 timeout arm、child block/expression、真实 ordinary catch/rethrow |
| R3 | statement/value concurrent仍返回 stable `InvalidArtifact` | `f445h_e4r_concurrent` | **至少 9** | 两个真实 concurrent arm、E2 scheduler、真实 lane evaluator、E3 real-store bridge |
| R4 | activation Ready仍 pre-suspend；stream wait看不到 lease child/local deadline owner，非 End cleanup组合不完整 | `f445h_e4r_stream` | **至少 8** | activation-relative evaluator call、`exec_program_stream_for_in`、两个 invocation response loops、provider stream task/publication |
| R5 | combined tests先在 R1-only integration base运行，timeout/concurrent stubs与旧 stream必然 RED；合入 R2/R3/R4 后转 GREEN | `f445h_e4r_combined` | **至少 5** | 同一 fixture中真实四 arm + actual-Pending + stream，不直接调用 child helper |

禁止降级：

- leaf helper、prepared owner、E1 scope、E2 fake scheduler、E3 bridge单测只能作为辅助；
  不得替代上述 production entry。
- mock不得按 binding/effect直接返回预期“是否 suspend”；必须以 future第一次 poll的
  Ready/Pending和真实 frame/store计数断言。
- stream tests不得只 drop `StreamConsumerCleanup`；必须先穿过真实 next/send/publication wait。
- timeout tests不得直接构造 `ScopeTerminalCarrier`冒充 wrapper执行。

### 5.1 T05–T12 ownership

| 原验收项 | 唯一 owner |
| --- | --- |
| T05 statement normal、expression value、child current scope、parent恢复 | R2 |
| T06 local owner物化、inner catch miss、outer catch hit、catch后继续 | R2 |
| T07 inner-earlier、outer-earlier、equal outer-only | R2 |
| T08 inherited/request-like deadline不延长、不物化、不可 ordinary catch | R2 |
| T09 ancestor cancel同刻优先与 timeout scope lifecycle归零 | R2；stream同刻竞争由 R4补真实 wait证据 |
| T10 scripted clock纯 CPU loop与 generated/literal chunk | R1 |
| T11 statement dependency、tail、source-order、outer priority | R3 |
| T11 Actor Ready不切、Pending重叠、同步 segment串行 | R3 |
| T12 winner、未启动 lane、running cancel、late result、outer恢复 | R3 |
| T12 stream End/break/return/error/timeout/cancel/drop/current child scope | R4 |
| native/service/interface/Actor/callback Ready/Pending | R1；activation-relative service由 R4 |
| WebSocket send、serverStream、DbQuery同步例外 | R1；activation serverStream由 R4 |
| ValueBlock、ordinary catch/rethrow、Actor单续体、stream invocation | R2/R3/R4各自 focused，R5组合回归 |

R1 focused matrix至少包含：

1. scripted CPU loop；
2. scripted long array/map/object或 compiler-generated chunk；
3. 三类 Emit send的 Ready/Pending；
4. ordinary native Ready/Pending与 WebSocket send同步例外；
5. `createFromStream` pending/drop/paired finalize；
6. legacy outbound unary Pending与 serverStream Ready；
7. remote interface Ready/Pending；
8. callback Ready/Pending；
9. Actor dispatch Ready/Pending；
10. `DbQuery`同步不切 segment。

R2 的八个函数可用小矩阵覆盖 nested三种顺序、0/`u64::MAX`、throw/return/drop，但必须分别有
statement与expression真实入口。R3 的九个函数必须分别留下 dependency/tail/winner/late/Actor
计数证据。R4 的八个函数必须让 End与每类非 End terminal产生不同 cleanup断言，不能用一个
“任一错误都会 drop”断言合并。

### 5.2 避免重复 O6R13/J1 gate

叶子节点不得单独重跑：

- `program_db::tests::`
- `db_actor_`
- native `dispatch`
- `service_dispatch`
- `actor_dispatch`
- `callback_native`
- `prepared_runtime`
- J1 八个 owner selector
- 完整 `skiff-runtime-eval` suite

R1/R4 的 evaluator selector可消费这些 owner，但测试断言必须在 evaluator组合层。
R5 的完整 eval suite会自然再次包含部分既有 tests；这是最终 combined code state 的唯一完整
gate，不是重复 owner acceptance。不得再运行完整 native/service-db/capability-context suite。

## 6. Checkpoint 精确边界

R1 只使用 E1 已有 kind，不新增公共/内部 enum variant：

- `FunctionEntry`：executable 与 block entry；
- `LoopCondition`：while/for condition或下一 item前；
- `LoopBackedge`：继续下一 iteration前；
- `GeneratedChunk`：array/map/object/construct及 compiler-generated有界 chunk；
- E2 已拥有 `LaneStart`、`LaneEnd`、`TailStart`，R1/R3 不重复计数；
- actual-Pending/stream await进入与恢复使用同一个窄 helper调用现有 checkpoint，
  不新增调度 yield。

迁移必须替换相应 `add_instruction_units`、`check_cancelled`、
`poll_execution_budget`组合，不能在同一路径叠加两次 units。R1 result需给出反向搜索和至少一个
明确的 instruction-count assertion。

## 7. 执行性与停止条件

### 7.1 五分钟安全修改

| 节点 | 当前 seam 是否齐全 | 首次安全修改能否在五分钟内开始 |
| --- | --- | --- |
| R1 | 是；J1 prepared owners、E1 checkpoint、E3 await均存在 | 是；先写 `f445h_e4r_spine` RED与 test-only scoped control |
| R2 | 是；R1预分配 arm/helper，E1 owner API齐全 | 是；先在 `evaluator_timeout.rs` 把 stable fail-closed变成真实 RED |
| R3 | 是；E2/E3 crate-private API已由 E23验收 | 是；先在预分配 real-evaluator test file写 statement/value RED |
| R4 | 是；current `ExecutionScope`公开完整 signals/deadline，stream cleanup RAII已存在 | 是；先写 child-scope cancel与 activation Ready RED |
| R5 | 是；只消费三叶 commits | 是；先在 R1-only integration base提交 combined RED tests |

test-only `ordinary/test_runtime` 的 scope缺口不构成 E1 production seam缺失：它可通过现有
`ExecutionScope::request/derive` 在 test child module闭合，不需要改变 E1或 I6。

### 7.2 节点停止条件

R1 立即返回 `TASK_SCOPE_EXPANDED`，若：

- 任一 J1 标为 heap/env-free 的 wait 实际捕获 caller heap/env/EvalContext；
- callback接线需要修改 callback owner core而不是 crate-private consumer/re-export；
- checkpoint需要新增 E1 kind、改变 instruction accounting公共语义；
- test scope只能通过 production fallback伪造。

R2 立即返回 `TASK_SCOPE_EXPANDED`，若：

- `ScopeTerminalCarrier::is_owned_by` 无法区分当前 wrapper；
- 正确物化需要修改 `error.rs`、`exceptions.rs`、capability-context或公共 error类型；
- inherited terminal必须由 I6才能在 evaluator内部保持。

R3 立即返回 `TASK_SCOPE_EXPANDED`，若：

- E2无法同时保留 lane state与完成 E3 commit，且现有 `RequestHeap::clone`不能保持语义；
- `ConcurrentLaneFuture + Send` 只能通过 `?Send`、outer mutable borrow或 E2 public API修改实现；
- E3 bridge不能在 scheduler error后恢复 parent或在 drop时收束 child。

R4 立即返回 `TASK_SCOPE_EXPANDED`，若：

- 完整 current cancellation/deadline无法用
  `ExecutionScope::{cancellation_signals,effective_deadline,terminal_at}` 组合；
- exactly-once cleanup必须修改 capability-context `StreamConsumerCleanup` /
  supervised lease core；
- provider stream正确性要求 host/I6或 request-root fallback。

R5 不扩写 production。若 combined失败，只报告并退回责任节点；若问题横跨公共 owner，则
`TASK_SCOPE_EXPANDED`。

任何节点发现需要 O1–O6 core、E1/E2/E3 public API、host/native adapter、公共契约或语言语义
修改，均不得继续局部实现。

## 8. 集成顺序、gate owner 与风险

### 8.1 集成顺序

1. R1 先提交 implementation与focused result，记录精确 commit。
2. R2/R3/R4 从该 commit创建三个 task worktree并并行。
3. R5 worktree从 R1 commit创建；先提交/运行 combined RED。
4. R5 按 R2、R3、R4 顺序接入三个叶子 implementation commit。production写集不重叠，
   此顺序只用于审计稳定性。
5. focused combined GREEN 后只在该最终代码状态运行一次昂贵 gate。
6. 独立 acceptance PASS 后才解除 I6；本 DAG 本身不 merge `main`、不 push。

### 8.2 每节点 selector

命令形态固定为：

```text
cargo test -p skiff-runtime-eval --locked <selector> -- --list
cargo test -p skiff-runtime-eval --locked <selector> -- --nocapture
git diff --check
```

每个 task使用自己的独立 `CARGO_TARGET_DIR`。叶子 result记录 listing与execution实际非零数。

R5 独占：

```text
cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture
cargo test -p skiff-runtime-eval --locked --no-fail-fast
cargo check -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

R5 还必须反向搜索：

- 四个 `F445H-E4 evaluator integration is required` diagnostic为零；
- `native_call_suspends` 为零；
- production `suspend_actor_segment` / `resume_actor_segment` pre-suspend helper为零；
- production `maySuspend` / `may_suspend` 不参与 segment释放；
- 没有 sequential concurrent fallback、`yield_now`、request-only stream fallback；
- DB O6 adapter保持唯一，`DbQuery`仍无 external wait。

### 8.3 主要风险

| 风险 | 控制 |
| --- | --- |
| R1 root改动面积最大 | 单 owner；先冻结 child route；10+真实 tests；R2/R3/R4不再碰 root |
| ordinary test control升级 current scope影响既有测试 | test-only child抽取；保持原 cancellation/deadline输入；完整 eval只由 R5检查一次 |
| callback private module边界 | 只允许 `assembly_execution/mod.rs` 窄 crate-private consumer接线；不改 callback prepared core |
| Actor lane heap同时服务 E2 handoff与E3 commit | 使用现有 heap clone作 commit snapshot；真实 alias/field/store tests；不新增共享 lease slot |
| activation unary与provider stream同居大文件 | 两者统一归 R4；R1只预留 child route，不并行编辑 O3 |
| stream deadline/cancel同刻竞态 | biased ancestor-cancel检查 + `terminal_at` owner恢复；每类 terminal独立 cleanup计数 |
| final gate重复或证据过期 | R5在三叶最终 commit上唯一运行；任何 leaf修复只失效相关 focused与R5最终 gate |

## 9. 最终结论

`READY_FOR_E4R_DAG`。

当前 production 已具备全部实现 seam；唯一新增前置是 R1 内部的 root/module checkpoint和
test-only scope fixture修正，不是公共 API或语义扩张。可以按本结果直接签发 R1，随后从其精确
implementation commit并行签发 R2/R3/R4，最后由 R5 独占组合测试、完整 eval gate与 acceptance。

需要用户决策的问题：**无**。
