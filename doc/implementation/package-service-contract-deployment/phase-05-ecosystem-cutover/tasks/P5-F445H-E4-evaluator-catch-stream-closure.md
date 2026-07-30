# P5-F445H-E4 Evaluator, catch and stream closure

状态：Ready。F445H 的最终 evaluator 合流节点；完成后只解除 I6 host/native scope propagation
前置，不代表整个 F445H/F445 已收尾。

## 直接父节点

- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-E2-lane-local-DAG-scheduler-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`
- `P5-F445H-E23-concurrency-branches-combined-result.md`

本任务文件完整描述本节点需求。直接父节点提供已经实现的接口、证据和唯一上游引用；不得重新
解释 timeout、concurrent、错误 owner 或 Actor 语义，也不得直接把更高层权威设计当作额外需求。

## 当前检查点与边界

production prerequisite 为 Skiff integration `7a69b7e3`：

- E1 已提供 current owned execution control、保留 scripted clock 的 owned round-trip、
  `derive_timeout_child`、owner-aware `checkpoint` 与 internal `ScopeTerminalCarrier`；
- E2 已提供 linked plan projection、lane-local env/heap/current scope、DAG scheduler、确定性
  winner与失败原子的 heap handoff；
- E3 已提供每 lane 独立 Actor continuation、actual-Pending suspend、complete/abandon/drop和
  outer gate；
- E23 已在相同 production tree上独立通过完整 eval suite；
- `eval_context.rs` 的四个新 IR arm仍明确 fail closed；
- I6 尚未让 production host/native adapter从 invocation-time current scope读取完整
  `execution_scope` / `derive_scope`。本任务不得用 request-root fallback伪造 I6。

语言尚未发布，不保留旧 `maySuspend`、顺序 concurrent或 request-only兼容路径。

## 生产目标

### 1. Timeout statement、expression 与 owner

替换两个 timeout fail-closed arm：

- `LinkedStmtIr::Timeout { duration_ms, body, site }` 在 parent context的 clone上
  `derive_timeout_child`，使用 child current control执行 body，并原样返回 body的 `Flow`；
- `LinkedExprIr::Timeout { duration_ms, value, site }` 同样执行 value并返回 carrier；
- parent context/env作用域恢复依赖值所有权，不允许跨 `await` 原地替换共享 control；
- duration为 0、最大合法 `u64`、normal、throw、return、cancel和future drop都不得泄漏 child
  scope或改变 parent control。

child执行得到 `RuntimeError::ScopeTerminal` 时：

1. 只有 carrier的 local source/nesting与本 timeout child current scope精确匹配，wrapper才物化
   一个 `RuntimeError::UserException`，其 payload identity为
   `std.error.TimeoutError` / `PlatformBuiltinErrorIdentity::Timeout`；
2. payload details固定保留 `reason=deadlineExceeded`、`deadlineSource=scope`、
   `deadlineNesting`与完整 `deadlineSite`；
3. source site、exception correlation和 stack使用当前 timeout wrapper位置，不得借用内层
   ordinary catch或重建不相关 site；
4. inherited outer/request deadline保持 internal terminal，穿过当前 timeout和所有 ordinary
   catch；ancestor cancel保持不可 catch；
5. nested内层早于外层时由内层物化；外层早于内层时穿过内层后由外层物化；绝对 deadline相同
   时固定只由最外层 owner物化一次；
6. instruction limit等非 scope budget继续沿既有 `ExecutionBudgetExceeded` 行为。

不得通过 `ordinary_catch_projection()` 猜 terminal owner。不得让
`ScopeTerminalCarrier`进入普通 payload、wire error或 request heap。

### 2. Catch closure

ordinary catch只处理 ordinary error和已由正确 timeout wrapper物化的
`UserException<TimeoutError>`：

- internal local/inherited terminal在 owner wrapper之前不可 catch；
- cancellation不可 catch；
-外层 `catch<TimeoutError>` 能捕获 owner wrapper产生的 exception；
- catch后 parent execution仍可继续；
- existing nominal/union/package-schema catch identity和 rethrow行为不回归。

只在现有 catch owner中做最小接线，不新增错误接口、错误签名或公共 error类型。

### 3. Concurrent statement/value evaluator接线

替换两个 concurrent fail-closed arm，只消费 E2 seam：

- `project_concurrent_plan`
- `ConcurrentLaneExecutor` / `LaneExecutionState` / `LaneCompletion`
- `run_concurrent_scheduler`
- `ConcurrentSchedulerResult`

真实 lane executor必须：

- `Statement` lane执行 plan指定的唯一 direct statement body；
- `Serial` lane按 body中的既有顺序执行整个 block；
- `Tail` lane求值其 expression；
- 使用 `LaneExecutionState::program_context`、lane env与lane heap，不借用 outer
  `&mut Env` / `&mut RequestHeap`；
- normal、flow、error与 carrier精确映射到 E2 completion；不复制 ready queue、winner、scope
  lease、dependency import或 heap handoff状态机；
- statement concurrent全部 normal后返回 `Flow::Continue`；不允许 lane的 return/break/park等
  非 continue flow静默丢失，若 linked contract禁止则稳定 fail closed；
- concurrent value只有 tail normal结果返回 parent carrier；
- malformed shape保持 E2的 `InvalidArtifact`，不能顺序 fallback。

### 4. Actor 与 concurrent组合

当 outer context持有 Actor execution frame时，只消费 E3 seam：

1. concurrent入口调用 `begin_concurrent(&outer_heap, lane_count)`；
2. 每个真实 lane按 source order claim唯一 bridge lane；
3. 启动 lane evaluator前 `lane.resume`，并把该 child frame安装到
   `LaneExecutionState::program_context` 的 clone；
4. lane normal completion调用 `lane.complete(lane_heap)`；error/cancel/winner/drop调用
   `lane.abandon()` 或依赖 fail-safe drop；
5. 所有 lane终结后才 `bridge.resume_parent`，并恢复 outer frame；
6. 同一 Actor同步 segment仍串行，多个 lane的真实 Pending外部 future可以重叠；Ready future
   不释放 segment；
7. winner、outer cancel、timeout、malformed completion和future drop均不能泄漏 lease slot、
   scheduler guard或 active-child gate。

不得 clone旧单 frame来共享 lease slot，也不得在 E4复制 store acquire、identity fence、
field codec或 gate状态机。

### 5. Actual-Pending：移除预释放

删除 `native_call_suspends` 及所有根据 `maySuspend`、binding name、effect summary或调用种类在
await前无条件 `suspend_actor_segment` 的 production路径。service、native、DB、callback、
Actor/interface dispatch和其它 Actor-sensitive外部 future统一使用 E3既有 poll-once语义：

- 第一次 poll为 `Ready`：不 commit、不释放、不 reacquire Actor segment；
- 第一次 poll为 `Pending`：才 commit并释放，future ready后经过 current checkpoint、
  scheduler acquire与 identity fence再继续；
- error/cancel/drop沿同一 RAII路径收束；
- `connection.send` 或其它通常 ready的调用不因静态 effect声明让出执行权；若底层 future真实
  Pending，则按统一规则处理。

不得加入语言级 `yield`、Tokio主动 `yield_now`或新的 `nosuspend` 关键字。checkpoint是同步
终止检查，不是调度让出点。

### 6. 统一 checkpoint覆盖

把本写集内散落的 `add_instruction_units`、`check_cancelled`、
`poll_execution_budget`组合迁移到 E1 owner-aware `ProgramExecutionContext::checkpoint`，
至少覆盖：

- function/block entry；
- loop condition求值前与 loop backedge；
- lane start、lane result接纳后与 value tail start由 E2 seam保持；
- array/map/object等长 literal或 compiler-generated chunk的有界粒度检查；
- Actor resume/acquire等待前后；
- stream next、service/native/DB/interface await进入和恢复。

纯 CPU deadline测试必须使用 scripted clock，在有界 checkpoint次数内结束；不得依赖 Tokio
timer得到调度机会。instruction accounting不能被重复或漏记；既有 request共享预算语义不变。

### 7. Stream current scope与 exactly-once cleanup

`program_stream.rs`、`program_invocation.rs` 与
`assembly_execution/async_stream_cancel.rs` 的 stream wait必须读取调用时
`ProgramExecutionContext::execution_scope()`：

- 使用完整 `cancellation_signals()`，包括 request、ancestor/local与 E2 lease child取消；
- 使用 effective deadline及其 owner，不再从 generic budget error或 request-start token猜测；
- ancestor cancel优先于同刻 deadline；
- local/inherited deadline保持 internal carrier，交回相应 timeout/request owner；
- Actor frame下的 buffered/ready `next()`不释放 segment，只有真实 Pending才释放；
- natural End只 `reached_end()`；break、return、error、timeout、cancel和future drop都通过 RAII
  cancel source；
- source cleanup、producer terminal、waiter、timer与lease各自 exactly once；winner后的 late
  item/error/heap write不可进入 caller；
- cleanup不能无限等待不配合取消的producer才决定用户可见 terminal。

I6仍负责 request/root boundary对 inherited request deadline的最终协议映射。本任务若发现必须
修改 host/native adapter才能证明 production closure，按停止规则上报，不得加 fallback。

### 8. 结构

`eval_context.rs`、`program_stream.rs`、`program_invocation.rs` 与
`async_stream_cancel.rs` 均已很长。新增 timeout/concurrent/Actor wait/stream-scope职责应放入
对应 owner的窄 child module；root保留 dispatch、module声明和薄转发。不得继续堆叠一个新的
数百行 helper区，也不得复制 E1/E2/E3 production状态机。

## Test-first 与验收

先新增真实 RED，再实现。至少覆盖：

- T05：statement timeout normal flow、expression timeout value、child current scope可见、
  parent恢复；
- T06：local timeout在准确 wrapper物化，外层 catch成功，内部 catch不能提前截获，catch后
  parent继续；
- T07：nested inner-earlier、outer-earlier、equal-deadline outer-only；
- T08：request-like inherited deadline早于 local，不延长、不物化、不被普通 catch；
- T09：ancestor cancel与deadline同一 poll ready时cancel优先，生命周期归零；
- T10：scripted clock终止长纯 CPU loop与长 generated/literal chunk，无 scheduler yield依赖；
- T11：真实 evaluator concurrent两个无依赖 lane同时 Pending、dependency gating、tail fence、
  同 turn错误 source-order、outer terminal优先；
- T11 Actor variant：同步 segment不重叠、两个 external Pending重叠、Ready不切换；
- T12：winner阻止未启动 lane、取消 running child、late result隔离、Actor outer在全部 child
  关闭后恢复；
- T12 stream variant：break/return/error/timeout/cancel/drop与natural End的 cleanup exactly
  once，current child scope取消可见；
- actual-Pending：至少覆盖 native/service类 Ready与Pending各一条，不再使用静态
  `maySuspend`决定；
- existing `ValueBlock`、ordinary catch/rethrow、Actor单续体和stream invocation不回归。

允许复用 E1 scripted clock、E2 fake lane executor和 E3 real-store fixture；但至少要有穿过真实
四个 evaluator arm及真实 stream wait的集成测试，不能只重复 leaf unit tests。

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure/build/cargo-target \
  cargo test -p skiff-runtime-eval f445h_e4 -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果文档必须记录 focused和完整 suite实际测试数；零测试 filter不算证据。反向搜索必须确认：

- 四个 `F445H-E4 evaluator integration is required` arm已不存在；
- `native_call_suspends` 与 production `maySuspend`预释放判断已不存在；
- 没有顺序 concurrent fallback、主动 yield或 request-only stream fallback。

不运行 stable、live、network或其它仓库测试。

## 写集、非目标与停止规则

只允许：

- `runtime/eval/src/lib.rs`
- `runtime/eval/src/eval_context.rs`
- `runtime/eval/src/eval_context/**`
- `runtime/eval/src/program_stream.rs`
- `runtime/eval/src/program_stream/**`
- `runtime/eval/src/program_invocation.rs`
- `runtime/eval/src/program_invocation/**`
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/**`
- 本 result

child目录可包含窄 production helper与对应 tests。不得修改 E1/E2/E3 owner、capability-context、
request、host/native adapter、artifact/compiler/linker、Router、Cargo manifests或 lockfile。

非目标：

- 不实现 I6 request/host boundary；
- 不改变语言/IR/source语义；
- 不引入 yield/nosuspend；
- 不修改 service/HTTP/WebSocket/publication设计；
- 不保留历史兼容；
- 不运行 stable/live。

若 E2/E3 seam不足、需要公共 API或写集外 production owner、I6是当前正确性必要条件，或实际
任务仍有多个未决设计问题，立即停止并精确报告 `TASK_SCOPE_EXPANDED`；不得吞并 I6或自行修改
上游。发现一个直接小缺陷时提供最小 RED和建议 owner，不要绕过。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure
branch   codex/p5-f445h-e4-evaluator-closure
```

先提交 implementation，再只新增并提交
`P5-F445H-E4-evaluator-catch-stream-closure-result.md`。最终 clean；不得 merge/rebase/push。

任务合同已可执行，不应为填槽派子 Agent。只有出现一个会阻止正确实现的具体未知量时，最多派
一个只读有界子 Agent；该子 Agent不得再派 Agent。探查后范围超出预期或依然存在多个不明确问题
时，立即结束任务并如实上报。
