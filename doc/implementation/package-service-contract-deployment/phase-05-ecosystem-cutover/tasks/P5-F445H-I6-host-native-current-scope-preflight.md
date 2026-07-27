# P5-F445H-I6 Host/native current-scope propagation preflight

状态：Ready。I6 实现前的有界只读 owner 探查；与 E4 并行，只产出可执行合同，不修改
production或 tests。

## 直接父节点

- `P5-F445F-scoped-execution-control-checkpoint-result.md`
- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-E4-evaluator-catch-stream-closure.md`

前两份结果冻结 capability façade与 eval current-scope合同；E4 任务冻结 evaluator/stream与 I6的
边界。本探查不得重新解释 timeout、cancel、Actor或 concurrent语义。

## 固定输入

Skiff integration `6d324555`。E4只修改 `runtime/eval/**`，因此本探查可以只读检查 disjoint的
host/native owner；I6 implementation仍必须等待 E4 result并以其集成 commit为生产前置。

已知第一个缺口：

`runtime/host/src/eval_capability_adapter/execution.rs::RuntimeExecutionControl` 与
`RuntimeOwnedExecutionControl` 尚未覆盖 capability contract的 `execution_scope()` /
`derive_scope(...)`，默认行为只能 `Unavailable`，且禁止从 `deadline()` /
`cancellation_token()`猜回 request-only scope。

## 必答问题

### 1. Façade adapter

精确说明：

- concrete `skiff_runtime_request::{ExecutionControl, OwnedExecutionControl}` 到
  capability-context borrowed/owned control的包装路径；
- borrowed与owned API各自应怎样实现 `execution_scope`、`derive_scope`；
- nesting overflow、scope unavailable和其它 derive失败如何无损映射为
  `ExecutionScopeAccessError`；
- 哪些 tests能证明 cloned current `EffectiveDeadline`、source、site、nesting与全部
  cancellation signals均保留。

### 2. Invocation-time owner清单

逐项追踪 HTTP client、service outbound、WebSocket request、time/sleep、file、source stream、
普通 stream及其它实际 native operation：

```text
Eval ProgramExecutionContext current control
  -> native capability projection
  -> host/eval capability adapter
  -> native dispatch/context
  -> lower transport/provider operation
```

对每项记录：

- 当前 control是在 adapter/context构造时冻结，还是每次 operation invocation读取；
- 当前读取的是 full `execution_scope()`，还是旧 `deadline()`、单 token、cancel flag或
  request-start snapshot；
- lower operation接收 absolute deadline、wire timeout、cancellation signals或 child lease的
  哪种组合；
- timeout/cancel winner后如何取消一次、清理一次、拒绝 late response/value/error；
- 正确 owner文件、函数和最小 production写集。

不得仅凭 binding name或静态 effects列表猜 operation行为。

### 3. Deadline、cancel和lease合同

I6 implementation必须能够证明：

- operation invocation读取 full `EffectiveDeadline`，取 request/local/primitive约束的最早值；
- inherited deadline不在 child adapter变成普通 `TimeoutError`；
- ancestor cancel优先于同刻 deadline；
-需要 pending/cleanup fencing的 operation使用 current scope的
  `cancellation_signals()`，必要时使用 `acquire_lease()`及 child scope/token；
- local deadline settlement不污染 request-wide first-failure telemetry；
- WebSocket request保持既有三参数 Skiff API；timeout不是第四个业务参数；
- cancel frame / lower cancel / source cleanup exactly once，late response不能恢复已删除或其它
  pending call。

探查应判断这些合同是否可由现有 capability/native/host API实现；若需要公共 API或 request /
transport production写集，精确标记范围扩张，不自行设计。

### 4. Request/root boundary

找出 inherited request deadline从 eval internal terminal到最终协议/host error的唯一现有 owner。
说明 I6是否只需保留/转发，还是必须在某个 host request boundary做最终映射；禁止在普通 native
operation或 inner catch中提前物化。

### 5. Hermetic tests与 I7 handoff

给出 T13–T16 的最小 fake fixture、现有可复用 test support、真实 RED断言和命令：

- HTTP：`min(request, local, primitive)` 与 late response discard；
- service outbound：tighter caller scope、callee/dependency earlier winner、single cancel；
- WebSocket：三参数 request、pending原子移除、单 `$/cancelRequest`、late response false；
- time/file/stream：current child scope、break/return/timeout/cancel cleanup与 unsupported lower
  cancel的 bounded cleanup。

同时列出只属于 I7 的 cross-layer receipt，不把 compiler/artifact/Agine source编译吞进 I6。
不使用真实网络、wall-clock长 sleep、stable instance或 live service。

## 输出要求

新增
`P5-F445H-I6-host-native-current-scope-preflight-result.md`，必须包含：

- `READY_FOR_IMPLEMENTATION`、`TASK_SCOPE_EXPANDED` 或 `DECISION_REQUIRED`；
- operation-by-operation owner表；
- 精确 implementation写集与禁止写集；
- 建议 task拆分/DAG。若写集确实互斥且可并行，明确 join gate；不要为了填槽拆分；
- test-first矩阵、命令与预计完整 crate gates；
- E4 result进入后需要重新确认的最小 delta；
- 当前不需要用户决策时明确写出。

只允许新增该 result。不得修改 production、tests、父文档、Cargo manifests或 lockfile。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i6-preflight
branch   codex/p5-f445h-i6-preflight
```

最终 clean；不得 merge/rebase/push。不得派子 Agent。若探查后范围超出预期或仍有多个不明确
问题，应如实以 `TASK_SCOPE_EXPANDED` / `DECISION_REQUIRED` 结束，不得假装 ready。
