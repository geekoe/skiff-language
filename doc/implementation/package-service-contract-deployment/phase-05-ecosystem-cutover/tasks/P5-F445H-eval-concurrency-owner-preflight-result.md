# P5-F445H Eval and concurrency owner preflight result

状态：`TASK_SCOPE_EXPANDED`。

I5 不能作为一个 bounded implementation leaf。linked IR 已经充分表达 timeout / concurrent
源码语义，不需要回退 I3，也没有新的权威设计问题需要用户决定；扩大的原因来自三个实现边界：

1. I4 lease 的 child cancellation token 尚不能组成 child current scope，lane 被取消后，嵌套
   host/native/stream 调用无法从 `execution_scope()` 观察该取消；
2. eval 当前把 local / inherited deadline 都折叠为同一个可 catch 的
   `ExecutionBudgetExceeded`，丢失 F445F 已冻结的 terminal owner；
3. actor 当前只有单续体 execution frame，并且若干调用在 future 真正返回 `Pending` 前就释放
   executor。它既不满足 actor 权威合同，也不能承载同一 actor 方法内的多个 concurrent lane。

因此必须先做一个 I4 correction，再把 eval 拆为 scope/checkpoint、lane scheduler、actor
continuation 和最终 evaluator/stream 集成四个互斥写集节点。下文给出证据、最小 DAG 和
T05–T12 owner。

## 1. 输入与只读验证

固定 production 输入：

```text
/Users/geek/workspace/skiff-phase-05-integration @ d5812c27
```

preflight worktree：

```text
/Users/geek/workspace/skiff-p5-f445h-eval-preflight
branch codex/p5-f445h-eval-preflight
```

只读编译命令：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-eval-preflight/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
```

结果为预期 RED，exit `101`，只有两个与本任务直接相关的 `E0004`：

- `runtime/eval/src/eval_context.rs::exec_program_statement` 未覆盖
  `LinkedStmtIr::{Timeout, Concurrent}`；
- `runtime/eval/src/eval_context.rs::eval_program_expr` 未覆盖
  `LinkedExprIr::{Timeout, ConcurrentValue}`。

另有一个既有 unreachable-pattern warning。没有通过 wildcard 隐藏新 IR，也没有在 preflight
中修改 production 或测试。

## 2. Evaluator exhaustive dispatch

linked model 已包含唯一执行 shape：

| surface | linked shape |
| --- | --- |
| statement timeout | `LinkedStmtIr::Timeout { duration_ms, body, site }` |
| statement concurrent | `LinkedStmtIr::Concurrent { plan }` |
| expression timeout | `LinkedExprIr::Timeout { duration_ms, value, site }` |
| sequential value block | `LinkedExprIr::ValueBlock { block, result }` |
| value concurrent | `LinkedExprIr::ConcurrentValue { plan }` |

真正执行 statement / expression 的 exhaustive dispatch 只有：

- `runtime/eval/src/eval_context.rs::exec_program_statement`；
- `runtime/eval/src/eval_context.rs::eval_program_expr`。

`ValueBlock` 已在 expression dispatch 中按词法 block 后求 result 的顺序执行。新增的四个
timeout / concurrent arm 正是编译 RED 指出的缺口。其它扫描 `Call`、`Emit`、return 或 suspend
summary 的 match 不是 evaluator semantic dispatch；I5 不应在这些辅助扫描里重新推导执行计划。

最终集成必须做到：

- statement timeout 执行 body 并只返回 body 的 `Flow`；
- expression timeout 执行 value 并返回 carrier；
- statement concurrent normal completion 返回 `Flow::Continue`；
- concurrent value 只有 tail lane 产生结果；
- malformed linked plan 一律 `InvalidArtifact`，不能退化为顺序执行。

## 3. Current execution scope 的保存、克隆与恢复

### 3.1 当前结构

`ProgramExecutionInput<'a>` 和 `ProgramExecutionContext<'a>` 当前都保存借用的
`ExecutionControl<'a>`。`ProgramExecutionContext::clone` 克隆这个借用 view。

`OwnedProgramExecutionContext` 才保存 `OwnedExecutionControl`：

- `capture` 对当前 borrowed control 调用 `owned()`；
- `borrow` 再构造 borrowed `ProgramExecutionContext`；
- stream producer 因此能跨 `tokio::spawn` 保留 request-start control。

当前没有安装 derived current scope 的 API，也没有 scope guard。`file_source_stream`、`time`
等 capability context 还会在 context 构造时捕获当时的 control。

### 3.2 bounded 改法

I5 不改变公开的 `ProgramExecutionInput` 形状。`ProgramExecutionContext` 内部改为保存
`OwnedExecutionControl`，构造时立即对 input control 调用 `owned()`，并提供：

- `execution()`：每次从 owned current control 生成 borrowed view；
- `with_execution_control(OwnedExecutionControl)`：在 context clone 上安装 child current
  scope；
- eval-owned monotonic clock view，production 默认使用 Tokio/std monotonic instant，测试可
  使用 scripted clock。

timeout 和 lane 不在共享 context 上原地替换 control，而是创建 child context：

```text
parent context
  -> derive owned scope
  -> cloned child context.with_execution_control(derived)
  -> execute child
  -> drop child context
  -> caller continues with unchanged parent context
```

因此 normal、throw、timeout、cancel、future drop 都由值所有权自然恢复 parent scope，不需要
跨 `await` 持有可错配的可变 guard。

`OwnedProgramExecutionContext::capture` 继续捕获调用时的 current owned control；所有
invocation-time capability projection 必须从 `ProgramExecutionContext::execution()` 读取，
不能复用 request-start snapshot。production host adapter 对 `execution_scope` /
`derive_scope` 的实现属于 I6；I5 使用 scope-aware fake 锁定接口。

最大合法 `duration_ms` 远大于常见平台 `Instant::checked_add` 范围。eval helper 必须对合法
duration fail safe 地钳到该 `Instant` 实现可表示的最远将来，再与 parent deadline 取较早者；
不得把 I3 已接纳的合法 IR 在 runtime 当作 artifact error。

## 4. Terminal carrier 与 catch

当前路径如下：

| 结果 | 当前 carrier | 当前 catch |
| --- | --- | --- |
| normal | `Flow` / `RuntimeValueCarrier` | 不适用 |
| 用户 throw | `RuntimeError::UserException(RequestException)` | 按 nominal catch identity |
| ancestor cancel | `RuntimeError::Cancelled` | 不可 catch |
| local deadline | `ExecutionControlError::BudgetExceeded(DeadlineExceeded)` → `RuntimeError::ExecutionBudgetExceeded` | `TimeoutError` |
| inherited deadline | 同上 | 同样被投影为 `TimeoutError` |

F445F 的 `ExecutionScopeTerminal` 已能区分：

- `AncestorCancelled`；
- `LocalDeadlineExceeded`；
- `InheritedDeadlineExceeded`。

并且只允许 local terminal 有普通 Timeout catch projection。但是
`request/src/execution_control.rs::poll_execution_budget` 把 local / inherited 两者都转换成
同一个 `ExecutionControlError::BudgetExceeded`，eval 的 `From<ExecutionControlError>` 随后
再次折叠，owner 到这里已经丢失。

I5 必须增加 eval-internal、不可普通 catch 的 scope terminal carrier。统一 checkpoint 先读取
`execution_scope().terminal_at(clock.now())`，再检查共享 instruction/request budget：

- ancestor cancel 转成 `RuntimeError::Cancelled`；
- inherited deadline 以 internal terminal 穿过当前 timeout 和 catch；
- local deadline 也先保持 internal，只有拥有该 local source / nesting 的 timeout wrapper
  才把它物化成带当前 source site、correlation 和 stack 的
  `RuntimeError::UserException(TimeoutError)`；
- instruction limit 等非 scope budget 继续走现有 `ExecutionBudgetExceeded`；
- timeout wrapper 不得依赖 `ordinary_catch_projection()` 猜 owner。

这也锁定嵌套规则：内层只物化自己拥有的 deadline；相同到达时间时 inherited terminal 穿过
内层，由最外层 owner 产生唯一可见 Timeout；request deadline 穿过所有 local timeout，最后由
request/host boundary 映射，不能被内部 catch 截获。

## 5. Checkpoint 审计

### 5.1 当前已有

- block entry：instruction + cancel；
- block 每条 statement 前：budget poll；
- statement / expression entry：instruction + cancel；
- function call entry、若干 return/materialization 后：instruction / budget poll；
- array / map literal 每项：instruction + cancel；
- `for-in` 与 stream consumption 每轮：instruction + cancel；
- match arm 选择过程中：budget poll；
- actor resume 等待中：budget tick；
- stream consumer 有 RAII cleanup guard。

这些位置分散调用 `add_instruction_units`、`check_cancelled` 和
`poll_execution_budget`，尚不能保留 scope terminal owner。

### 5.2 必须补齐

所有 eval 路径改走一个 owner-aware checkpoint helper，并明确覆盖：

- function entry；
- loop condition 求值前；
- loop backedge；
- lane start；
- lane normal/error completion 后；
- concurrent value tail start 前；
- 长 array/map/record 或 compiler-generated 片段的固定粒度 checkpoint；
- actor segment resume / acquire 等待；
- stream next、service/native/host await 的进入和恢复。

纯 CPU 测试不能依赖 Tokio timer 在同一个始终 ready 的 future 之外获得调度机会，所以需要
eval-owned clock seam。scripted clock 在第 N 次 checkpoint 越过 deadline，即可证明长纯 CPU
循环在有界 checkpoint 数内结束。

## 6. Env、slot 与 heap 隔离

`Env` 和内部 `SlotStore` 都可 clone；slot value 是
`Vec<Option<RuntimeValueCarrier>>`。`RequestHeap` 可 clone，并已有
`deep_clone_runtime_value_carrier_between_heaps(source, destination, carrier)`。

不能让多个 lane 共享 `&mut Env` 或 `&mut RequestHeap`。正确模型是：

1. concurrent plan 入口冻结 baseline `Env + RequestHeap`；
2. 每个 ready lane 从 baseline 创建自己的 env / heap；
3. 只把该 lane 显式 dependency 的 export carrier 按 source order 深拷贝进 lane heap，并放入
   对应 slot；
4. lane normal completion 才保留 export；error、cancel、future drop 或 winner 之后到达的
   value/error/heap 全部丢弃；
5. statement concurrent normal completion不把 lane 局部 env / heap 整体合回 parent；
6. value tail 在独立 lane heap 中导入依赖，normal result 再深拷贝回 parent heap。

不能从“已经完成的全部 lane heap”克隆下一个 lane，因为这会把未声明 dependency、无关临时值
和 source-order timing 泄漏进 sibling。

当前 `Env` 缺少按 slot 导出 / 导入的窄 API；E2 只增加 eval-private snapshot/import helper，
不开放任意 SlotStore mutation。

### 6.1 IR 是否充分

`LinkedConcurrentPlanIr` 已有：

- plan site；
- 每个 lane 的 strict kind；
- 连续 `source_order`；
- sorted prior `dependencies`；
- statement/serial body label或 tail expression ref；
- lane site。

I3 admission 还保证 value tail 依赖闭包所有前序 lane。runtime 不缺调度字段。

IR 没有显式 `export_slot`，但现有 lowering shape 可无歧义消费：

- `Statement` lane body 必须恰好含一个 source 直属 statement；
- 只有该唯一 statement 是直接 `LinkedStmtIr::Let { slot, .. }` 时才产生 sibling-visible
  export；
- `Serial` 永不 export；
- `Tail` 只 export plan result。

这不是从 runtime 重算 source dependency：dependencies 仍完全来自 I3，runtime 只从已链接的
statement body 读取已有 destination slot。若 statement lane body 不是精确单 statement，或
出现其它不可能 shape，必须 fail closed。无需回退 I3。

## 7. 真实 async overlap、DAG 与 winner

每个 lane future 必须拥有：

- lane-local env / heap；
- child current scope；
- scope lease / completion；
- completion/drop probe；
- source order 和可能的 const export。

ready queue 只在全部 dependencies normal 后入队，并按 source order 启动。无依赖且到达真实
async wait 的 lane 必须同时存活，不能串行 await 每个 lane。可使用 Tokio task set 或同一 task
内的 unordered future set；关键不是线程并行，而是多个 pending async operation 确实重叠。

确定性 winner 顺序：

1. 每次选择 lane result 前先做 outer owner-aware checkpoint，因此 outer timeout / request
   deadline / ancestor cancel 优先；
2. 一个 poll turn 中收集所有已经 ready 的 lane error candidate；
3. 若没有 outer terminal，选择 `source_order` 最小的 error；
4. winner 确定后停止启动尚未启动 lane，触发所有 running lane child scope cancellation；
5. 等到每个 active lane 的 execution segment / cleanup 已有界退出后再恢复 outer execution；
6. winner 后的 late value、late error 和 lane heap mutation一律不导入；
7. 所有前序 lane normal 后，tail 仍须经过独立 checkpoint 才能启动。

外部 effect 已经提交的事实不回滚；runtime 只保证 Skiff heap/env 不泄漏，并依赖现有 effect
admission 和 cancellation metadata。

## 8. I4 child-scope correction

`ExecutionScope::acquire_lease` 当前返回：

- `ExecutionScopeLease`；
- `ExecutionScopeLeaseCompletion`；
- lease 上单独的 `child_cancellation_token()`。

lease terminal/drop 会 cancel 这个 token，但该 token 不在原
`ExecutionScope::cancellation_signals()` 中；`ExecutionScope` 字段私有，也没有公共方法构造
“原 scope + lease child token”的 child scope。只把 token单独传给一处 wait 不够：I6 的
invocation-time 合同明确要求 host/native 从 current `execution_scope()` 读取完整 deadline /
cancel，stream producer 也会捕获 current context。

因此先做 I4 correction：

```text
ExecutionScopeLease::child_execution_scope()
```

它返回保留原 effective deadline、nesting、local cancellation 和 lifecycle，但把 lease child
token 追加为 ancestor cancellation 的 scope。lease drop/control terminal 后 child scope
观察为 `AncestorCancelled`；parent scope 不受影响。normal completion 不制造虚假 cancel。

没有这个 API，E2 即使 cancel lane future，也无法证明嵌套 host work 和后台 stream cleanup
收到结构化取消。

## 9. Stream cancellation 与 bounded cleanup

当前 `StreamConsumerCleanup` 的 RAII 方向正确：

- 只有观察到自然 `End` 才调用 `reached_end()`；
- `break`、`return`、error、timeout、cancel 和 future drop 都会 drop guard，从而 cancel source；
- producer lease / sink cancel 防止 producer继续无限发送。

缺口在 current scope：

- `program_stream.rs` 把 `execution.cancellation_token()` 作为单个 request-start token传入；
- `program_invocation.rs` 的消费路径也只轮询 generic execution control；
- `assembly_execution/async_stream_cancel.rs::deadline_error` 从 generic budget error重新猜
  deadline owner。

E4 必须让 stream wait 消费 current `ExecutionScope::cancellation_signals()` 和 effective
deadline，使用 internal terminal carrier，不再由 `deadline_error` 重建 owner。winner 后 drop
lane future必须：

- child scope cancel；
- source cleanup exactly once；
- producer / waiter / timer / lease counter归零；
- 不等待无法配合取消的 late producer 才决定用户可见 winner，但其 heap/value/error 永不导入。

对有 actor frame 的 stream next，继续使用 poll-once 语义：buffered item立即返回且不释放
executor，只有实际 `Pending` 才提交同步 segment。

## 10. Actor composition 缺口

权威 `actor-model.md` 明确规定：`maySuspend` 只是保守静态 summary；只有 future 实际返回
`Pending` 才释放 actor executor。

现状只有 stream next 使用 `ActorExecutionFrame::await_if_pending`。service、DB、callback、
actor/interface 和若干 native 调用在 await 前直接调用
`eval_context.rs::suspend_actor_segment()`；native 甚至根据 `native_call_suspends` summary
提前决定释放。这与权威合同不符。

更直接的并发阻塞是：`ActorExecutionFrame` 内只有一个
`Mutex<Option<ActorInstanceExecutionLease>>`。两个 clone frame 共享这个 slot；第一条 lane
suspend 后，第二次 suspend 会得到：

```text
Actor continuation attempted to suspend without an execution token
```

actor 与 concurrent 合同可以组合且无需新设计决定：

- concurrent 入口把 parent actor segment提交一次；
- 每个 lane 有独立的 suspended continuation frame，但共享 actor store / handle / incarnation
  fence；
- actor scheduler 保证同一实例同一时刻只有一个 lane同步 segment持有 execution lease；
- lane 的 capability future 首次 `Pending` 时提交该 lane segment，另一 ready lane 才可获得
  lease；
- ready future 不释放 lease；
- lane完成/error/cancel时提交或释放其 segment；
- 所有 active lane 已经释放 execution lease 后，outer continuation 才能重新 acquire；
- CPU checkpoint保证无 await 的 lane也能有界到达 completion/cancel fence。

这需要 actor continuation bridge，而不是让 lane 共享现有 frame。可以在
`actor_executor.rs` 内基于既有 store/handle 构造独立 suspended child frame，不要求改变 actor
公开语义；若实现证明还需修改 `actor_instance.rs`，E3 必须结束并上报 scope expansion，不能把
额外写集偷带进 E4。

E4 同时把现有 preemptive suspend call sites 收口到 poll-once helper，避免用 `maySuspend`
制造并不存在的调度点。

## 11. 最小实现 DAG

```text
R0 I4 child current scope correction
            |
            v
E1 eval current-scope / terminal / checkpoint core
       |                         |
       v                         v
E2 lane-local DAG scheduler   E3 actor continuation bridge
       |                         |
       +------------+------------+
                    v
E4 evaluator dispatch / stream / catch integration
```

E2 与 E3 可在 R0 + E1 后并行；E4 是唯一 join node。各节点不得再拆出会交叉修改其它节点写集的
“顺手修复”。

### R0 — I4 child current scope correction

精确写集：

- `runtime/capability-context/src/scoped_execution.rs`
- `runtime/capability-context/src/scoped_execution/lease.rs`
- `runtime/capability-context/src/scoped_execution_tests.rs`

退出条件：

- lease 提供 child execution scope；
- drop、ancestor terminal、local/inherited terminal 均 cancel child scope；
- child terminal 对 parent 无反向污染；
- normal completion 不产生 cancel；
- active lease/waiter/timer 全部归零；
- capability-context full tests GREEN。

### E1 — Eval current scope、terminal 与 checkpoint core

精确写集：

- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/**`
- `runtime/eval/src/error.rs`

退出条件：

- context 内部拥有 current control，clone child 安装 / drop恢复有测试；
- invocation-time `execution()` 始终读取 current control；
- local/inherited/cancel internal carrier不被 generic conversion 折叠；
- timeout supervisor helper只物化自己拥有的 local terminal；
- production / scripted monotonic clock和最大 duration safe add有测试；
- owner-aware checkpoint core覆盖 function/loop/generated chunk helper；
- 不修改 evaluator dispatch。

### E2 — Lane-local DAG scheduler

精确写集：

- `runtime/eval/src/env.rs`
- `runtime/eval/src/env/**`

其中 scheduler 作为 `env` 的 crate-private child module，避免与 E1/E4 争用 module root。

退出条件：

- baseline clone和dependency-only slot import；
- statement const export shape fail closed；
- 无依赖 lane 用 barrier证明真实 overlap；
- ready queue、tail fence、outer-terminal priority、source-order simultaneous error winner；
- first winner 后停止 launch、cancel running、discard late；
- lane scope lease lifecycle归零；
- 不共享 `&mut Env` / `&mut RequestHeap`；
- 不修改 actor/evaluator/stream 文件。

### E3 — Actor continuation bridge

精确写集：

- `runtime/eval/src/actor_executor.rs`
- `runtime/eval/src/actor_executor/**`

退出条件：

- parent frame 可产生多个独立 suspended child continuation frame；
- 同一 actor 上多个 lane pending 时无共享 lease slot冲突；
- 同步 segment仍串行，pending async operation真实 overlap；
- buffered/ready operation不释放 executor；
- cancel/error lane释放 lease，outer resume前无 active lane segment；
- incarnation replacement和budget/cancel仍 fail closed；
- 如必须触及 `actor_instance.rs`，节点以 `TASK_SCOPE_EXPANDED` 结束，不越界实现。

### E4 — Evaluator、catch 与 stream closure

精确写集：

- `runtime/eval/src/lib.rs`
- `runtime/eval/src/eval_context.rs`
- `runtime/eval/src/program_stream.rs`
- `runtime/eval/src/program_invocation.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- 上述 owner 的既有或新 test child files

退出条件：

- 四个缺失 IR arm和已有 `ValueBlock` 合同全部执行；
- timeout/catch 的 local/inherited owner矩阵 GREEN；
- E2 scheduler与 E3 actor bridge只通过各自 crate-private API接入；
- service/native/DB/interface等 await不再根据 `maySuspend` 提前释放 actor；
- stream使用完整 current scope，source cleanup exactly once；
- `cargo check -p skiff-runtime-eval --locked` GREEN；
- T05–T12 focused、eval full、fmt、diff-check GREEN。

I6 仍负责 production host/native adapter 提供真实 `execution_scope` / `derive_scope`，以及
request/root boundary 对 inherited request deadline 的最终协议映射。E4 不能用 request-only
fallback伪造 I6。

## 12. T05–T12 精确 owner 与 hermetic fixture

| 测试 | 最小 hermetic fixture | 精确断言 | owner |
| --- | --- | --- | --- |
| T05 timeout normal/value | hand-built linked stmt/expr；scope-aware fake control；paused/scripted clock | body/value normal；child current scope可见；返回后 parent scope完全恢复 | E1 helper，E4 IR arm最终接线 |
| T06 local timeout/catch | local derived scope；精确 source site/correlation/stack；内外两层 catch | wrapper site只物化自己的 `TimeoutError`；外部 matching catch可捕获；内部/inherited不可提前捕获；parent后续可执行 | E1 carrier，E4 catch |
| T07 nested timeout | paused clock；内早、外早、同刻三组 | 内早由内层可见；外早穿过内层；同刻只有最外层可见 | E1 |
| T08 request earlier | request-like root deadline早于 local deadline | local timeout不延长、不物化 request deadline；internal inherited terminal穿过 catch；I6 boundary后续负责协议映射 | E1，I6 production closure |
| T09 ancestor cancel | notify/token fake；cancel与deadline同一 poll ready | ancestor cancel优先、不可 catch；所有 lease/waiter/timer归零 | R0 + E1 |
| T10 pure CPU | scripted clock在第 N 次 checkpoint越界；长 for-in和长 literal/generated chunk | function entry、condition、backedge、chunk checkpoint可观察；在有界 count内终止，无 Tokio scheduler机会依赖 | E1，E4 coverage |
| T11 concurrent overlap/DAG | barrier capability记录 lane entered并统一 release；hand-built valid plan | 两个无依赖 lane都 entered 后才 release；dependency等待；tail等待全部前序 normal；同刻 lane errors按 source order；outer terminal优先 | E2，E4接线 |
| T12 cancel/cleanup/late | 可控 lane futures；drop/completion probes；fake stream source；独立 heaps | winner阻止未启动 lane；running child scope被 cancel；late value/error/heap write不导入；stream cancel exactly once；全部 lifecycle counter为零 | E2 + E4 |

T11/T12 另加 actor variant，由 E3 提供 fixture：

- 两个 lane 的 async wait可同时 pending；
- 同步 actor segment 不重叠；
- ready operation不产生额外 executor切换；
- winner/cancel 后 outer actor continuation只在所有 child segment释放后恢复；
- 不出现共享 lease slot 的 `InvalidArtifact`。

## 13. 最终判定

判定为：

```text
TASK_SCOPE_EXPANDED
```

扩展是实现所有权和写集扩展，不是 source / IR 设计缺失：

- I3 plan 已足够，禁止 eval 重算 dependencies；
- F445F local/inherited/ancestor priority 已唯一决定错误行为；
- runtime 与 actor 权威文档已唯一决定真实 overlap、actual-Pending 和 late discard；
- 当前无需用户补充设计决策。

后继 coordinator 应先调度 R0，再调度 E1；只有二者 GREEN 后才能并行 E2/E3，最后由 E4
统一接线。不能把 I4 correction、actor bridge 或 stream owner问题压回一个“大 I5”节点。
