# P5-F445F Scoped execution control checkpoint result

状态：`IMPLEMENTATION_COMPLETE / FULL_REQUEST_GATE_INPUT_BASELINE_BLOCKED`。

F445B-I4 的 request/capability-context 通用 primitive 与 I5/I6 consumer contract 已完成；
新增聚焦测试全部 GREEN。任务要求的完整 `skiff-runtime-request` 命令不能报告 PASS：它在本节点
零 diff 的既有 WebSocket connect fixture 上稳定保留一个 input baseline failure。本 result
分别记录 scoped-control 验证和该独立 fixture closure，不把 40/41 伪报为 full crate GREEN。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| 任务指定 integration production input | `128129ff` |
| task worktree 初始 HEAD | `356f66f8` |
| implementation | `fbfdd147` |

`128129ff..356f66f8` 只新增 F445E/F445F 两份 task 文档，production 零 diff。

implementation 写集只有：

- `runtime/request/**`
- `runtime/capability-context/**`

implementation 提交后才新增本 result。没有修改 eval、host、native、WebSocket/HTTP
capability、wire、artifact schema 或 source AST；没有派子 Agent，没有 merge、rebase、push、
stable、live 或 network 操作。所有 Cargo 命令均使用本任务独立 target：

```text
/Users/geek/workspace/skiff-p5-f445f-scoped-control/build/cargo-target
```

## 2. Test-first 证据

测试先于各自 production contract 写入，形成了三次可归因 RED：

1. capability-context paused-clock tests 写入后首次运行 full crate，exit `101`，
   `E0583` 明确报告缺少 `scoped_execution` production module。
2. capability primitive 可编译、request production 尚未修改时运行 request full crate，
   exit `101`，10 个 `E0599` 明确报告缺少 `derive_scope`、
   `add_instruction_units_at`、`poll_execution_budget_at`、`effective_deadline`、
   `scope_terminal_at` 等派生控制 API。
3. consumer façade 编译型测试写入、contract 尚未加入时再次运行 capability-context，
   exit `101`，报告缺少 `ExecutionScopeAccessError` 及 façade 的
   `derive_scope` / `execution_scope`。

随后才实现对应 production，并将这些 scoped-control tests 全部转为 GREEN。异步 deadline、
cancel、lease 用例使用 Tokio paused clock；request poll 用例显式传入 fake monotonic
`Instant`，没有 wall-clock sleep 或真实 I/O。

## 3. 冻结的 scoped execution 模型

### 3.1 Effective deadline

`EffectiveDeadline` 显式持有：

- monotonic absolute `Instant`；
- `ExecutionDeadlineSource::{Request, Scope { site }}`；
- `nesting: u32`。

`ExecutionScope::derive(local_deadline, site)` 只在 parent/request effective deadline 与 local
candidate 中选择最早值。比较使用 `parent.at <= local.at`，因此相同 absolute deadline 固定保留
outer source、outer site 和 outer nesting；child 不能延长 parent/request。

scope nesting 使用 checked increment，overflow fail closed。local timeout 的
`TimeoutError` details 固定包含：

```json
{
  "reason": "deadlineExceeded",
  "deadlineSource": "scope",
  "deadlineNesting": 1,
  "deadlineSite": {
    "kind": "synthetic",
    "reason": "runtimeControlFlow"
  }
}
```

实际 `deadlineSite` 保留 compiler/linker 提供的完整 `InstructionSourceSite`；示例只展示测试
synthetic site。

### 3.2 Terminal ownership 与 cancellation 分源

`ExecutionScopeTerminal` 明确分成：

- `AncestorCancelled`
- `LocalDeadlineExceeded(EffectiveDeadline)`
- `InheritedDeadlineExceeded(EffectiveDeadline)`

只有 `LocalDeadlineExceeded` 提供 ordinary `TimeoutError` payload/catch projection。
`AncestorCancelled` 永远没有 ordinary projection；`InheritedDeadlineExceeded` 也不能在 child
scope 被 catch，必须传播给拥有 source/nesting 的 outer/request owner。

每个 derived scope 有独立 local cancellation source。child 的 ancestor signal set 包含 request
token 与所有 parent local tokens，因此 outer timeout 从 child 观察仍是 ancestor terminal。
local winner 只 cancel 当前 scope signal 与 lease child work，不 cancel shared request/parent
token。退出或 drop derived control 不修改 parent control，outer catch 后可恢复 parent execution。

所有同步检查与 async wait 均保持 cancel-first：

1. ancestor cancellation；
2. current local cancellation；
3. effective deadline；
4. normal completion。

因此 cancel 与 deadline 同 ready 时固定得到 `AncestorCancelled`。同 deadline 的 inner scope
只能得到 non-projectable inherited terminal；outer owner 才能投影 timeout。

### 3.3 Request accounting 与 telemetry isolation

request `ExecutionControl` 的 root/derived/owned/borrowed view 都保留同一个
`Arc<ExecutionBudget>`：

- instruction count/limit 共享；
- poll count 共享；
- `execution_budget_trace_attrs` 后续读取的 stats/first-failure facts 共享。

local/inherited scope deadline poll 只调用 `record_scoped_poll` 增加共享 poll accounting，再从
shared stats 构造当前 control failure；它不调用 `record_deadline_exceeded`，也不写
`budget_reason`。request deadline 仍走原 request budget poll 并记录
`DeadlineExceeded` first failure；ancestor cancel 仍记录 request-wide `Cancelled`。

### 3.4 Bounded lease lifecycle

`ExecutionScope::acquire_lease` 返回：

- `ExecutionScopeLease`：wait owner 与 child cancellation token；
- `ExecutionScopeLeaseCompletion`：one-shot completion fence。

lease 用 atomic terminal settlement 与 RAII 管理 active lease/waiter/timer counters，不 spawn
detached timer task。normal、local timeout、ancestor cancel 和 drop 都在 terminal settlement
时归零；非-normal terminal/drop cancel child work。deadline/cancel 后的
`completion.complete()` 返回 `false`，不能重新打开已结束 lease。多个同 scope lease 共享 local
signal，但不共享 request token 的 mutation authority。

## 4. I5/I6 consumer contract

capability façade现已冻结两个显式方法：

- `ExecutionControl::{derive_scope, execution_scope}`
- `OwnedExecutionControl::{derive_scope, execution_scope}`

底层 `ExecutionControlApi` / `OwnedExecutionControlApi` 同步冻结相同 contract。
未保存 full scope 的 adapter 返回 `ExecutionScopeAccessError::Unavailable`；默认实现不会从旧
`deadline()` + `cancellation_token()` 猜回 request-only scope，也没有 silent fallback。

concrete request control 同时提供：

- `derive_scope(local_deadline, site)`；
- `effective_deadline()` / `scope_nesting()` / `execution_scope()`；
- `scope_terminal_at(now)`；
- `cancellation_signals()`；
- fake-clock friendly `add_instruction_units_at` /
  `poll_execution_budget_at`。

### 4.1 I5 eval handoff

I5 必须按以下边界接线：

1. 在 `runtime/eval/src/program_execution.rs` 的 current execution context 中保存 parent
   `OwnedExecutionControl`；进入 linked timeout wrapper 时调用 façade
   `derive_scope(absolute_deadline, instruction_site)`，安装返回的 owned control，并用 frame/guard
   在 normal、throw、timeout、ancestor cancel、drop 的所有出口恢复 parent。
2. CPU/function/loop/lane checkpoint 先通过
   `execution_scope()?.terminal_at(now)` 读取 owner 分类，再处理 shared instruction budget。
   `AncestorCancelled` 直接走内部 cancellation；
   `InheritedDeadlineExceeded` 越过当前 catch boundary；只有
   `LocalDeadlineExceeded` 在当前 owner 使用其 ordinary catch projection。
3. async block/lane work 使用 `acquire_lease`；把 lease child token 传给 child work，winner
   后丢弃 late value/error/write。不能用 request `cancel_flag()` 代替 scope signal。
4. 当前 `runtime/eval/src/assembly_execution/async_stream_cancel.rs::deadline_error` 只从
   `poll_execution_budget` 重建 deadline error；I5 必须改为消费 scope terminal ownership，
   否则 inherited outer/request deadline 会在 child 被错误投影。
5. 如果 runtime adapter 尚未提供 scope，timeout execution 必须对
   `ExecutionScopeAccessError::Unavailable` fail closed；不得退回 request-start deadline。

### 4.2 I6 host/native handoff

I6 必须在 `runtime/host/src/eval_capability_adapter/execution.rs` 为
`RuntimeExecutionControl` 覆盖：

1. `execution_scope()`：clone concrete request control 的 current `ExecutionScope`；
2. `derive_scope(...)`：调用 concrete `OwnedExecutionControl::derive_scope`，将结果重新包装成
   capability `OwnedExecutionControl`，并把 nesting overflow 映射为
   `ExecutionScopeAccessError::Derive`。

随后每个 operation 在 invocation 时从 façade `execution_scope()` 读取：

- full `EffectiveDeadline`，而不是只读 `deadline()` absolute instant；
- `cancellation_signals()`，而不是只读 request `cancellation_token()` / `cancel_flag()`；
- 需要 pending/cleanup fencing 时使用 `acquire_lease()` 与 child token。

HTTP、service outbound、WebSocket、time/file/stream adapters 必须在各次 operation invocation
读取 current scope；不能继续使用 request adapter construction 时保存的 deadline/token 快照。
local deadline settlement 不得调用 request `record_deadline_exceeded`，ancestor terminal 不得转成
`TimeoutError`。本节点没有越界修改这些 consumer。

## 5. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context --no-fail-fast` | PASS：48 unit tests；2 doc-tests |
| `cargo test -p skiff-runtime-request execution_control::tests -- --nocapture` | PASS：4 scoped request tests |
| `cargo test -p skiff-runtime-request --no-fail-fast` | **不是 PASS**：40/41 unit tests PASS，1 个 input fixture FAIL；1 doc-test PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

完整 request 命令唯一失败：

```text
websocket_connect_target::tests::websocket_connect_target_requires_real_handler_and_exact_plan
InvalidGatewayEntryProtocolSurface:
WebSocket connect surface must expose exactly the jsonrpc-2.0-text profile
```

## 6. Input baseline fixture closure handoff

该失败已在本任务实现之前的 task worktree 初始 HEAD `356f66f8` 上用独立本地 clone/独立
Cargo target 单测复现：同一测试 0/1，完全相同 panic。由于 `128129ff..356f66f8` 只有两份
task 文档，该复现同样证明任务指定 production input 已带该失败。

原因精确为：

- `runtime/request/src/websocket_connect_target.rs:345-349` 没有 import
  `GatewayWebSocketRpcProfile`；
- 同文件 `surface()` 在 line 366 使用 `rpc_profiles: Vec::new()`；
- `artifact-identity/src/gateway.rs:404-407` 已要求 exact
  `[GatewayWebSocketRpcProfile::JsonRpc2_0Text]`；
- `git diff 356f66f8..fbfdd147 -- runtime/request/src/websocket_connect_target.rs
  artifact-identity/src/gateway.rs` 为空，本实现没有触碰 failure producer 或 validator。

I7 或独立 request fixture closure 应只把该测试 fixture 改为：

```rust
rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
```

并补 import 后重跑完整 request crate。该 WebSocket fixture 与 scoped execution control 无关，
本节点按写集和用户指示没有越界修复，也没有把 full request gate 标为 GREEN。

## 7. 反向闭包

- implementation commit 只包含 task 允许的两个 runtime crate。
- scoped production/tests 反向搜索没有 WebSocket、HTTP、eval、source AST、wire 或 artifact
  schema 特殊字段。
- 没有新增 public `CancelError`、WebSocket private timeout 参数、legacy dual path、
  request deadline fallback 或 detached cleanup task。
- 本节点交付的是可供 I5/I6 显式消费的通用 execution control primitive；consumer 未接线时
  `Unavailable` fail closed。
