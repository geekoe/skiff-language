# P5-F445H-E3R Heap-borrowing actual-Pending preflight result

状态：`READY_FOR_CORRECTION_DAG`。

结论是：

1. E3 的 actual-Pending Actor 状态机本身没有发现 production 缺陷；它在 future 不借用 caller
   heap/env 时已经满足“第一次 poll 为 `Ready` 不释放、真实 `Pending` 才提交”的合同。
2. E4 result 提议的 detached field snapshot 不可采用。Actor field 中的 array/map/object 是
   caller heap handle；合法用户代码能绕过 `ActorExecutionFrame::write_field`，直接经 nested
   field 或 receiver method 修改该 handle 可达节点。只在显式 Actor field write 刷新 snapshot
   会丢失同步 segment 的真实终态。
3. 安全修正不应伪装成一个“小 E3 correction”。需要由 DB、native、service、callback 和 Actor
   dispatch 各 owner 把同步 prepare、heap-free owned wait、resume 后 finalize 分开；transaction
   和 lease claim 必须保留专门状态机。完成这些 owner 节点并做组合审查后，重新发 E4，只让 E4
   接 evaluator call site、timeout/concurrent/catch/stream closure。
4. 当前没有需要用户决定的语言或产品语义。若后续实现只能靠改变 Actor field alias 语义、
   RequestHeap 公共模型、unsafe 别名或恢复 pre-suspend，必须停止，不能自行选择。

## 1. 输入、范围与判定

| 项 | 值 |
| --- | --- |
| production checkpoint | `d9b504ee` |
| worktree HEAD | `ca52d8f25424acc5b1245b72ed68fe3c4561406e` |
| direct parent | `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md` |
| direct parent | `P5-F445H-E4-evaluator-catch-stream-closure-result.md` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e3r-preflight` |
| branch | `codex/p5-f445h-e3r-preflight` |

本节点只读 production/IR/model owner，未修改 production、tests、父任务、manifest 或 lockfile，
也未运行 stable、live、network。下面的 DAG 是后继任务合同，不是本节点已经实施的代码。

之所以选择 `READY_FOR_CORRECTION_DAG` 而不是 `TASK_SCOPE_EXPANDED`：

- 工作确实跨多个 owner，不能放回原 E4 或一个 E3 小补丁；
- 但每个 owner 的所有权边界、前后依赖、写集和停止条件已经可以明确冻结；
- 没有剩余的语言语义选择，也没有必须先问用户的产品决策。

## 2. Actor field 与 RequestHeap 是同一份可变状态

### 2.1 lease 如何把 field handle 与 heap 联系起来

`runtime/eval/src/actor_instance.rs` 中：

- `ActorInstanceState` 同时保存 `fields: Vec<ActorFieldValue>` 和 `heap: RequestHeap`；
- `ActorFieldValue.value` 是普通 `RuntimeValue`，heap-backed value 只保存
  `RuntimeValue::Heap(handle)`；
- `ActorInstanceStore::acquire_execution` 分别 clone live `state.fields` 和
  `state.heap`，组成一个 `ActorInstanceExecutionLease`；
- `ActorInstanceExecutionLease::take_heap` 把这份 lease heap 移给
  `actor_executor.rs` 的 evaluator；
- `ActorInstanceStore::commit_execution` 把 lease fields clone 和调用方交回的整份 heap
  一起替换到 live state。

因此 lease fields 中的 handle 必须在同一次 lease clone 出来的 heap 上解释。fields 并不是一份
脱离 heap 的值快照。

`runtime/eval/src/actor_executor/actor_concurrent_continuation.rs` 中：

- `read_field` 只 clone `RuntimeValue`，heap handle 数字不变；
- `write_field` 才会通过 linked field plan 做 wire round-trip，并把 checked value 写回 frame
  fields；
- `suspend` 取走当前 lease，并用 `heap.clone()` 调用 `commit_execution`；
- `resume` 先 acquire 新 lease，再从该 lease 的 source heap 编码每个 field，解码到当前
  continuation heap，并把 field handle 改成当前 heap 中的新 handle；
- `await_if_pending` 第一次 poll 为 `Ready` 时原样返回；第一次真实 `Pending` 时才
  `suspend(heap)`，等待同一个 future，随后 `resume(heap, execution)`。

`runtime/eval/src/actor_executor.rs` 的执行入口也确认 evaluator 使用的是
`lease.take_heap()` 返回的 heap，而不是另一份 detached heap。

### 2.2 nested mutation 不经过 `write_field`

最小合法 source 形状如下：

```text
actor Counter {
  state: {
    count: number,
    labels: Map<string, string>
  }

  update() {
    self.state.count = 2
    self.state.labels.set("phase", "prepared")
    std.time.sleep(1)
  }
}
```

对应代码路径是：

1. `compiler/lowering/src/function_lowering.rs::lower_assign_target` 只把直接
   `self.state = value` 降为 `AssignTargetIr::ActorSelfField`；
2. `self.state.count = value` 降为普通 `AssignTargetIr::Field`，其 object expression 再降为
   `ActorSelfField` read；
3. `eval_context.rs` 的 ActorSelfField read 调用 frame `read_field`，得到指向 caller heap 的
   handle；
4. 普通 `AssignTargetIr::Field` 直接调用
   `RequestHeap::set_object_field_carrier`，不调用 frame `write_field`；
5. `Map.set`、`Array.push/set/pop`、`JsonObject.set/delete` 同样在
   `runtime/eval/src/receiver_methods.rs` 中通过 receiver handle 直接改 RequestHeap；
6. runtime 的 `AssignTargetIr::Index` 也经
   `program_mutation::assign_program_index_target_carrier` 直接改 array/map/object handle。

当前 lowering 不产生直接 index assignment target，但 receiver mutation 已提供不依赖该语法的
production 路径；普通 nested field assignment 也已经足够证明 alias。

所以同步 first poll 可以先改 `self.state` 可达对象，再返回 `Pending`，而 frame fields vector
本身完全没有第二次 write。此时真实终态只存在于当前 caller heap。现有 `suspend(heap)` clone
这份 heap，因而不会丢 mutation；只监听显式 `write_field` 的 detached snapshot 会保留旧对象并
丢状态。

### 2.3 没有可复用的“所有 mutation 后”hook

RequestHeap 的可变入口不只有 Actor assignment：

- array/map/object/index 和 builtin receiver method；
- boundary decode、native return、DB result、callback import 等 allocation；
- alias 可以在 local、argument、Actor field 和多个容器间传播。

frame 不在这些入口上，也没有收到 mutation notification。给每个入口增加 hook 等价于把
RequestHeap 改成事务式、observer-aware 公共模型；这不是 E3 小修正。

`RequestHeap::checkpoint` 也不是撤销日志。它只记录 `nodes.len()` 和 stats；
`rollback_to_checkpoint` 只 truncate 新节点并恢复 stats，不能还原 checkpoint 之前已有
array/map/object 的原地修改。因此不能先改 live heap、失败后再靠 checkpoint 恢复旧 snapshot。

## 3. Detached snapshot 方向判定：拒绝

| snapshot 载体 | alias / cycle | first-Pending 时能否提交真实终态 | 失败原子性 | 判定 |
| --- | --- | --- | --- | --- |
| 每个 field 的 wire JSON | 跨 field 的共享 alias 丢失；不可表示 runtime-only carrier；cycle 不能编码 | 只有每次可达 mutation 后重编码才可，现无 hook | 原 live heap 已被原地修改，checkpoint 不能撤销 | 拒绝 |
| 独立 RequestHeap + field values | 单次整体 clone 可保留一部分 alias；现有 deep clone 对 cycle 明确报错；逐 field clone 仍丢跨 field alias | 仍须拦截所有 allocation/mutation 并同步到独立 heap | 同样缺少原地 mutation undo | 拒绝 |
| 只保存 `Vec<ActorFieldValue>` | heap handle 仍指向 live caller heap，不是 detached | future 持有 `&mut heap` 时 frame 不能安全读取该 heap | 无 | 拒绝 |
| 每次 mutation clone 整份 heap | 理论上可保留整 heap alias | 需要 pervasive hook，且每次 mutation 为 O(heap) | clone 前后的外部副作用和 in-place mutation仍需事务日志 | 拒绝 |

不能通过改变 `read_field` 来规避：若每次 read 都把 field 深拷贝到 continuation-local detached
值，`self.state.count = 2` 或 `self.state.labels.set(...)` 将不再持久化，直接改变现有语言可观察
语义。

状态收束也不能补救 snapshot 缺失：

| 情况 | 正确合同 | detached snapshot 的缺口 |
| --- | --- | --- |
| first poll `Ready` | 不 commit、不 release、不 reacquire | 若提前 clone/commit，已经违反合同 |
| first poll `Pending` | 提交 first poll 后真实 heap 终态一次 | snapshot 不知道 alias mutation |
| pending future 返回 error | 仍先 resume，再把 operation error交给 evaluator | snapshot 仍须包含 first poll mutation |
| pending future drop/cancel | operation guard清外部资源；Actor guard释放 scheduler/gate | 不能靠 drop 重启或读取仍被 future 借用的 heap |
| resume failure / replacement | identity fence fail closed，不安装 stale lease | snapshot 不能替代 fence |
| nested concurrent bridge | 每 lane 提交自己的真实 lane heap；各层 gate独立 | per-frame snapshot还需跟踪每个 lane全部 alias mutation |

现有 E3 bridge 对不借 caller heap 的 future 已正确覆盖这些 Actor 生命周期。问题不是“如何在
future 持有 heap 时给 frame 偷看 heap”，而是 operation owner 不应让外部 wait future 持有
caller heap。

## 4. 所有现有 pre-suspend 调用点分类

分类含义：

- **A 直接接 E3**：future 已不借 caller heap/env；
- **B 一次 prepare/wait/finalize**：边界天然可拆，wait 可完全 owned；
- **C owner 内重构**：当前 async fn 混在一起，但单一 owner 内可安全拆；
- **D 专门状态机**：递归 evaluator、事务或资源生命周期不能压成一次通用 operation；
- **E 非外部 suspension**：不应释放 Actor segment。

| 当前调用点 / operation | 类别 | 证据与正确方向 |
| --- | --- | --- |
| `eval_context.rs` 三条 `Emit` send | A | projection/deep-clone/wire encode 已在 await 前完成；sink、owned item、signals/token 组成的 send future 不借 caller heap |
| `program_stream.rs` stream `next()` | A | `next_with_cancellation` 先构造 heap-free future；当前已经正确调用 `frame.await_if_pending` |
| `DbOperation` | C | `DbIrEvaluator::eval_operation` 递归准备 command，`execute_db_command` 又把 DB wait 与 recoverable encode/decode混合；需 DB owner拆分 |
| `DbQuery` | E | `DbIrEvaluator::eval_query_value` 只求值 query expression 并在 caller heap 物化 query value；没有 DB I/O；嵌套 expression 自己处理 wait |
| `DbTransaction` | D | begin await → 递归 block → commit/abort await；需要阶段状态机与 drop/cancel abort guard |
| `DbLeaseClaim` | D | key prepare → claim await → binding/renew task → 递归 body → lost/release await；需要 lease guard、renew abort 和 exactly-once release |
| `DbLeaseRead` | B | key eval/encode → owned store read → resume 后 decode到 caller heap |
| remote interface outbound | B | 与 legacy outbound共用 service dispatch；payload 可先编码，lease/receiver wait owned，response resume 后 decode |
| callback capability | D | caller→owner materialization、owner heap mutex、递归 evaluator、owner→caller import；可由 callback owner做专门的 owned-lock状态机，不能塞进通用一次 operation |
| activation-relative unary service | C | caller→provider heap materialization后，provider heap/context可由 owned future持有；完成后才 import回 caller |
| activation-relative server stream创建 | E | `start_provider_stream` 同步完成 setup/spawn并返回 stream handle；后续真实等待发生在 consumer `next()` |
| Actor dispatch | B | argument wire encoding → owned `ActorInvocationRequest` wait → response decode/import |
| legacy service dependency unary | B | payload encode / `start_request` → owned lease+receiver wait → response decode/import |
| legacy service dependency server stream | E | request start后同步构造拥有 lease/receiver 的 stream value；后续等待在 `next()` |
| native dispatch总入口 | C | 当前 `dispatch_resolved_native_call(..., heap)` 把 decode/wait/return materialization混在 async fn；需 native owner prepared call |

`eval_context.rs` 现有 pre-suspend 对的完整位置为：

- Emit：三条 internal projected、noncanonical internal 和 wire sink send；
- DB：operation、query、transaction、lease claim、lease read；
- interface：remote 和 callback；
- call target：activation-relative service、Actor dispatch、legacy service dependency、native。

除此之外，`program_stream.rs` 已有一条 actual-Pending `next()`，不应退回 pre-suspend。

## 5. 各 operation 的 prepare / wait / finalize 合同

### 5.1 stream emit 与 stream next

| 阶段 | 合同 |
| --- | --- |
| prepare | emit 先完成 runtime item projection、跨 heap clone或 wire encode；next 只 clone stream runtime/value、signals 和 cancellation token |
| owned wait | `StreamSink::{send_internal_with_cancellation,send_with_cancellation}`；`StreamRuntime::next_with_cancellation` |
| finalize | emit只映射 send error；next在 resume 后把 item/error materialize到 caller heap |
| cancel/drop owner | sink/source、`StreamConsumerCleanup` 和 operation future；Actor frame只管 segment |
| 最小写集 | `runtime/eval/src/eval_context.rs`、`runtime/eval/src/program_stream.rs` 及各自 child tests；属于重发 E4，不需改 sink API |

### 5.2 outbound service 与 remote interface

`runtime/eval/src/service_dispatch.rs` 已经接近正确分层：

- `encode_outbound_request_payload` 和同步 `context.start_request` 是 prepare；
- `await_outbound_response` 只拥有 context/dispatch、`OutboundRequestLease` 和 receiver，不需要
  caller heap/env；
- decode payload、coerce return 和 caller stream cancellation检查是 finalize；
- serverStream 的 `outbound_service_stream_value` 是同步 finalize，stream value接管 lease，
  不形成当前调用的 suspension。

| 项 | 合同 |
| --- | --- |
| cancel/drop owner | unary lease在 response/error/drop时 complete/cancel；stream source Drop取消 lease和 registry |
| 最小 production | `runtime/eval/src/service_dispatch.rs`，必要时窄 child module |
| 最小 tests | 该文件既有 outbound tests + 新 Actor Ready/Pending/lease-drop tests |

remote interface 和 service dependency只消费同一个 prepared outbound owner，不各造状态机。

### 5.3 Actor dispatch

`runtime/eval/src/actor_dispatch.rs::dispatch_actor_method` 可直接切成：

- prepare：验证 receiver/method/arity，构造 linked plans，把 arguments 编成 canonical owned
  `ActorInvocationRequest`；
- wait：clone actor capability context，`invoke_actor(request)`，不持 caller heap；
- finalize：把 returned payload decode到 caller heap，或映射 cancellation/Actor error；
- cancel/drop：`ActorInvocationRequest`/capability wait owner；不得交给 frame模拟。

最小 production/test 写集仅为 `runtime/eval/src/actor_dispatch.rs` 及其 tests。

### 5.4 activation-relative service

`runtime/eval/src/assembly_execution/async_stream_cancel.rs::execute_provider_unary` 当前已经创建
独立 `provider_heap` 和 `OwnedProgramExecutionContext`：

- prepare 在 caller segment 内完成 boundary plan、caller→provider 参数 materialization、
  provider context capture；
- owned wait 应 `async move` 持有 provider heap、owned provider context、provider request和
  provider args，返回 `{ provider_heap, provider_result }`；
- finalize 在 Actor resume 后导出 provider failure或把 provider result materialize回 caller
  heap；
- cancel/drop 由 provider request guard负责，deadline/cancellation继续由 provider owner选择。

serverStream setup仍是同步路径；producer task和 consumer stream分别拥有 cleanup，不能被 unary
prepared operation吞并。

最小 production/test 写集：

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- 新的窄 `async_stream_cancel/prepared_unary.rs`（如实现需要）
- 对应 provider unary/stream tests。

### 5.5 callback capability

`runtime/eval/src/assembly_execution/callback_native.rs::execute_interface_call` 当前同时借
`context.heap`、`context.env`，并持有
`InProcessCallbackAdapter::owner_heap().try_lock()` 得到的 borrowed guard跨递归 await。

可行 owner 方案是：

1. `runtime/native/src/callback_adapter.rs` 暴露 clone 的
   `Arc<tokio::sync::Mutex<RequestHeap>>`，让 callback owner取得 `OwnedMutexGuard`；
2. prepare 校验 carrier/operation并把 caller args materialize到 owner heap；
3. owned wait持有 guard、owner args、owned program context和必要调用信息，递归执行 owner
   executable；
4. finalize 在 wait future及 guard结束或明确交回 outcome后，把 owner result import到 caller
   heap；
5. 参数 materialization失败时保持当前 checkpoint truncate；method error/cancel/drop必须保持
   现有 owner-heap可见性语义并释放 guard exactly once，不能假设 checkpoint能撤销已有节点的
   原地 mutation。

`OwnedProgramExecutionContext::capture` 当前故意不捕获 `actor_execution_frame`，因此 detached
callback/provider evaluator不会意外复用 caller Actor lease。

最小写集：

- `runtime/eval/src/assembly_execution/callback_native.rs` 及 tests；
- `runtime/native/src/callback_adapter.rs` 及 tests。

这是 callback 专用状态机，不进入通用 one-shot operation trait。

### 5.6 DB

`runtime/eval/src/program_db.rs` 需要分两层。

普通 DB operation：

- prepare：`DbIrEvaluator::eval_operation` 完成所有 argument expression和 `DbCommand` 构造；
- owned wait：store/transaction owner执行已拥有的 command，返回 raw/recoverable owned结果；
- finalize：在 caller resume 后把 wire/recoverable outcome decode到 caller heap；
- cancel/drop：DB store operation/transaction guard拥有，不允许重启 command。

`runtime/service-db/src/store.rs`、`runtime/service-db/src/lib.rs` 当前若在 async API 中持
RequestHeap，须先由 service-db owner切成 raw wait与 recoverable codec阶段；不能从 eval 用
unsafe或 heap mutex绕过。

`DbLeaseRead` 是同一形状的简单实例：key encode → read wait → value decode。

`DbTransaction` 必须是 evaluator-aware 多阶段状态机：

```text
begin external wait
  -> transaction body（其中每个 operation各自 actual-Pending）
  -> commit external wait
  -> success
或任意 error/cancel/drop
  -> exactly-once abort + 现有 checkpoint truncate
```

这里不把 `RequestHeap::rollback_to_checkpoint` 称为完整 heap transaction：它只删除 checkpoint
后的 allocation，不能撤销已有节点的原地 mutation。若需要把 transaction body的所有内存
mutation也做原子回滚，那是另一个 owner/语义问题，不能在本 DAG中偷偷补成新语言合同。

`DbLeaseClaim` 必须显式拥有：

```text
claim wait
  -> binding import + renew guard
  -> body（嵌套 actual-Pending）
  -> stop renew
  -> lease_lost/read + release wait
  -> exactly-once terminal
```

不能把 transaction/claim包装成一个借 `&mut EvalContext` 的 future再交给 E3，那会回到原问题。

最小写集分成两个有依赖的 owner：

- service-db owner：
  `runtime/service-db/src/store.rs`、`runtime/service-db/src/lib.rs` 和直接相关
  recoverable/mapping child files及 tests；
- eval DB owner：
  `runtime/eval/src/program_db.rs`、`runtime/eval/src/program_db/**`、
  `runtime/eval/src/db_eval.rs` 及 tests。

### 5.7 native operation矩阵

native owner应提供 prepared call，而不是让 E4按 binding name判断 `maySuspend`。

| native 类别 | prepare | owned wait | finalize / 判定 |
| --- | --- | --- | --- |
| `std.time.sleep` | decode、校验并 clamp milliseconds | cloned time context + millis | `Null`；zero/ready仍由 first poll决定，不按名称释放 |
| 普通 `std.file.*` | 把 string/bytes/options/file ref解成 owned参数 | cloned file capability + owned参数 | 把 wire/file result decode到 caller heap |
| `std.file.createFromStream` | 参数/stream source准备 | 专用 supervised stream状态机 | import file result；source/partial file cleanup由 native owner exactly once |
| `std.http.client.request/stream/sse` | request encode成 owned wire | cloned HTTP context + request | response/stream handle decode到 caller heap |
| `std.http.stream.emitResponse` | event encode | stream response context send | `Null`；send guard拥有取消 |
| HTTP request/header helpers、`stream.start/chunk/end` | 同步 decode/operation | 无 | 类别 E，不释放 |
| WebSocket四个 send | decode connection/text/bytes | 无；capability send本身同步 | `Null`；即 `connection.send` 不让出 |
| `std.websocket.requestJsonToConnection` | 校验 error owner，encode method/params/payload | cloned websocket context + owned payload request | terminal/result decode或类型化错误 |
| actor registry get/replace/find/remove | id/bootstrap encode | cloned actor context + owned request | ActorRef/result decode |
| bytes/json/crypto/resource/telemetry等同步 helper | 同步 | 无 | 类别 E，不释放 |

`artifact-model/src/native_signature.rs` 的 callable semantics仍可描述 detachment/effect，但
`eval_context.rs::native_call_suspends` 不能再用 `may_suspend` 或 binding prefix决定调度。

最小 native production/test 写集：

- `runtime/native/src/dispatch/adapter.rs`
- `runtime/native/src/dispatch/core.rs`
- `runtime/native/src/dispatch/{time,file,http,websocket,actor}.rs`
- 必要时 `runtime/native/src/capability.rs`
- 对应 `runtime/native/src/dispatch/**` tests。

E4 后续只消费 native owner返回的同步 result或 heap-free owned wait，不再知道具体 binding。

## 6. 可统一的只有“owned wait 生命周期”

可以建立一个很窄的 crate-private 形状，但不要求所有 owner都实现一个大 trait：

```text
OperationStep<Ready, Wait> =
  Ready(Ready)
  | ExternalWait(Wait)

PreparedExternalOperation<Wait, Finalize> {
  wait: Wait,          // 不借 caller RequestHeap / Env / EvalContext
  finalize: Finalize,  // 只在 Actor resume 后接收 caller heap/env
}
```

这里必须叫 `ExternalWait`，不能在 first poll 前叫 `Pending`。真实 `Pending` 只能由 E3
`await_if_pending` 的第一次 poll观察得到。

推荐合同：

1. owner在当前同步 segment完成 prepare，且只启动一次外部副作用；
2. 若 operation纯同步，返回 `Ready`，不经过 Actor cut；
3. 若有 future，返回不借 caller heap/env 的 `ExternalWait`；
4. evaluator把 wait交给现有 E3 `await_if_pending`；
5. E3只在观察到真实 `Pending` 后 commit/release/resume；
6. evaluator在 resume 后调用 owner finalize；
7. wait/guard负责 cancel/drop，frame只负责 Actor lease/gate。

不应统一的部分：

- transaction begin/body/commit/abort；
- lease claim/renew/body/release；
- callback owner heap lock和递归 evaluator；
- file createFromStream、provider stream等资源/producer生命周期。

这些 owner可以复用“等待一个 heap-free future”的底层 runner，但必须保留自己的阶段枚举和 RAII
guard。

## 7. 其它方向审查

| 方向 | Rust 所有权 | Actor 语义 | 判定 |
| --- | --- | --- | --- |
| closure/HRTB `FnOnce(&mut heap) -> Future` | 返回 future 后 borrow仍活到 future结束；frame不能同时读 heap | first Pending无法提交同步终态 | 排除 |
| 把 heap move进 future | future存活且返回 Pending时不能把 heap取回 | 无法在 cut point提交；若先取回则必须结束/drop future | 排除 |
| `RefCell` | future可持 `RefMut` 跨 Pending；frame再借会 panic | 用运行时 panic替代所有权错误，且不能证明状态完整 | 排除 |
| `Mutex<RequestHeap>` | future可持 guard跨 Pending；frame commit会死锁或重入失败 | scheduler/heap锁顺序引入新死锁面 | 排除 |
| custom `Future` 在 Pending时归还 heap | 标准 `Future::poll` 的 `Poll::Pending`不携带资源 | 若扩展成自定义 step protocol，本质就是 owner状态机 | 标准 Future排除；显式 step接受 |
| poll 后由外部 hook/callback读取 heap | hook在 future外执行时与 future保留的 `&mut heap` alias | unsafe/未定义行为 | 排除 |
| future owner在返回 Pending前调用提交 callback | 可安全，因为仍在唯一 borrow内部 | callback需复制 Actor store/field/fence/gate，耦合所有 owner | 不采用；应返回 owned wait给中央 E3 |
| `Ready | ExternalWait { owned_wait, finalize }` | wait不借 caller heap/env，所有权闭合 | E3观察真实 Pending并统一 Actor lifecycle | 推荐 |

不得 drop后重建 future：first poll可能已经发 request、写 DB、注册 waiter或启动 renew task，重建会
重复副作用。不得退回 pre-suspend：它会让同步 Ready 和 WebSocket send错误让出。

## 8. 推荐最小 DAG

### 8.1 节点和依赖

```text
E3 existing actual-Pending seam
          |
          +--> O1 native prepared owner ------------------+
          +--> O2 outbound + Actor dispatch owner --------+
          +--> O3 activation-relative service owner ------+
          +--> O4 callback owner -------------------------+--> J1 combined owner review
          +--> O5 service-db raw/recoverable split --> O6 eval DB state machines --+
                                                                                |
E1 + E2 + E3 + E23 -------------------------------------------------------------+
                                                                                v
                                                                     E4R reissued evaluator closure
                                                                                |
                                                                                v
                                                                     I6 host/native propagation
```

O1–O5 可在写集锁定后并行；O6 必须等待 O5。J1 是独立组合审查点。E4R 必须等待全部 owner GREEN
和 J1，不能在中间保留 pre-suspend fallback。I6 若仍按现有上游 DAG位于 E4之后，则在 O1/E4R后
消费新的 native call形状，不能与 O1并发修改同一文件。

### 8.2 互斥写集

| 节点 | 独占 production 写集 | 与其它节点关系 |
| --- | --- | --- |
| O1 native | `runtime/native/src/dispatch/**`，必要时 `runtime/native/src/capability.rs` | 与 O4 的 `callback_adapter.rs` 不重叠；若实现需改该文件，必须先调整写集后串行 |
| O2 outbound/Actor | `runtime/eval/src/service_dispatch.rs`、`runtime/eval/src/actor_dispatch.rs` 及窄 child | 独立 |
| O3 in-process service | `runtime/eval/src/assembly_execution/async_stream_cancel.rs` 及 child | E4R随后可继续改 stream scope；不得并发 |
| O4 callback | `runtime/eval/src/assembly_execution/callback_native.rs`、`runtime/native/src/callback_adapter.rs` | 独立 |
| O5 service-db | `runtime/service-db/src/**` 中任务明确列出的 store/recoverable文件 | O6前置 |
| O6 eval DB | `runtime/eval/src/program_db.rs`、`program_db/**`、`db_eval.rs` | 等 O5 |
| E4R | 原 E4 eval写集，加 owner新 API的 call-site tests；不得重写 O1–O6状态机 | 等 J1，串行处理与 O3重叠的 `async_stream_cancel.rs` |

不应给 O1–O6 同时开放 `eval_context.rs`。所有 call-site 切换集中留给 E4R，避免每个 owner自行复制
Actor frame逻辑。

### 8.3 哪些属于 E3、owner refactor 与 E4

- **E3 correction**：没有发现需要改 E3 production 的事项。后继应先增加一个真实
  heap-backed Actor alias regression，确认 `self.field.child` / receiver mutation在 heap-free
  Pending 前被现有 `suspend(heap)`提交；若该测试失败，才单独打开 E3 correction。
- **operation owner refactor**：O1–O6；负责产生 heap/env-free owned wait、owner-specific
  finalize 和 cleanup。
- **E4R**：删除 `native_call_suspends` 与所有 pre-suspend pair，连接 owner API和 E3 seam，
  并完成原 E4 timeout/concurrent/catch/checkpoint/stream任务。

不能为了制造 test-first RED 而无理由改 E3 API。E3 alias regression是既有合同的 acceptance，
预期在当前 production上 GREEN；它不是一个 production实现节点。

## 9. 每个节点的 RED、focused GREEN 与停止规则

### O1 native prepared owner

- 真实 RED：测试要求 pending sleep/request在 first Pending释放 Actor segment，而 zero/ready
  operation和四个 WebSocket send保持 lease；现 API只能把 `&mut RequestHeap` 借进整个 future，
  无法接 E3。
- focused GREEN：
  `cargo test -p skiff-runtime-native dispatch -- --nocapture`，以及 eval侧 native
  Ready/Pending Actor fixture。
- 停止：任何 prepared wait仍借 caller heap/env；需要按 binding静态预释放；或
  createFromStream无法在 drop时证明 source/partial result cleanup exactly once。

### O2 outbound + Actor dispatch

- 真实 RED：prepared API不存在；测试要求同步 serverStream setup/ready response不释放，pending
  unary/Actor invocation释放一次，lease/cancel/drop不重复 side effect。
- focused GREEN：
  `cargo test -p skiff-runtime-eval service_dispatch -- --nocapture` 和
  `cargo test -p skiff-runtime-eval actor_dispatch -- --nocapture`。
- 停止：response wait仍借 caller heap；server stream setup被强制当 suspension；lease无法在
  drop时 cancel/complete exactly once。

### O3 activation-relative service

- 真实 RED：pending provider unary当前 future借整个 `&mut EvalContext`；测试要求 provider
  Ready不切 segment、Pending切一次、cancel/deadline/drop取消 provider，server stream setup不切。
- focused GREEN：
  `cargo test -p skiff-runtime-eval async_stream_cancel -- --nocapture`。
- 停止：owned wait仍捕获 caller heap/env/Actor frame；provider result在 resume前写 caller heap；
  producer cleanup被 unary protocol吞并。

### O4 callback

- 真实 RED：callback递归 future当前持 borrowed owner heap guard和 caller context；测试要求
  callback Ready不释放、Pending释放、owner heap checkpoint/drop和 capability generation fence
  保持。
- focused GREEN：
  `cargo test -p skiff-runtime-eval callback_native -- --nocapture` 和
  `cargo test -p skiff-runtime-native callback_adapter -- --nocapture`。
- 停止：必须把 caller Actor frame捕获进 owned callback context；owner mutex不能形成 owned
  guard；参数 prepare失败的 checkpoint清理或 method error/drop的既有 owner-heap语义无法保持。

### O5 service-db

- 真实 RED：raw DB wait和 recoverable caller-heap codec尚未分离；新的 owner test要求 wait
  future类型不借 RequestHeap，并保留现有 wire/recoverable结果。
- focused GREEN：
  `cargo test -p skiff-runtime-service-db --locked --no-fail-fast`。
- 停止：Mongo/store wait仍需持 caller heap；分离会改变 recoverable storage/wire语义；或需要
  重放 DB command。

### O6 eval DB

- 真实 RED：`DbQuery`目前会预释放，transaction/claim把递归 body包进单一借 heap future；
  测试要求 query不释放，普通 operation和lease read按真实 Pending释放，transaction/claim每个
  外部阶段分别 actual-Pending且 abort/release exactly once。
- focused GREEN：
  `cargo test -p skiff-runtime-eval program_db -- --nocapture` 和 DB Actor fixture。
- 停止：transaction drop/cancel不能保证 abort；claim不能保证 renew task停止和 lease
  exactly-once release；任何状态机需要复制 Actor scheduler。

### J1 组合审查

这是 read-only/acceptance join，不制造虚假 RED：

- 确认所有 owner wait不借 caller `RequestHeap`、`Env` 或 `EvalContext`；
- 确认外部副作用只启动一次，drop/cancel guard明确；
- 确认 sync/stream-creation路径没有被包装成预释放；
- 运行各 owner focused suite和 `cargo check`，但不提前改 E4 call site。

若任何 owner仍要求 pre-suspend，J1失败，不得启动 E4R。

### E4R 重发

- 真实 RED：先增加/恢复原 T05–T12 与 native/service/DB/interface Ready/Pending集成测试，并反向
  搜索证明 `native_call_suspends`、`suspend_actor_segment` 调用仍存在；测试应在接线前失败。
- focused GREEN：
  原 E4 的 `f445h_e4` filter、完整
  `cargo test -p skiff-runtime-eval --locked --no-fail-fast`、
  `cargo check -p skiff-runtime-eval --locked`、`cargo fmt --check`、`git diff --check`。
- 停止：任何 operation owner API仍借 caller heap/env；E4R需要修改 O1–O6核心状态机；I6
  production是当前正确性前置；或必须保留静态 pre-suspend fallback。

E4R 的任务文件必须重新发，至少：

1. 把 O1–O6 和 J1 result加入直接 prerequisite；
2. 删除“只消费原 E3即可迁移所有调用”的过时假设；
3. 明确 `DbQuery` 非外部 suspension；
4. 明确 serverStream创建和 WebSocket send为同步路径；
5. 把 `async_stream_cancel.rs` 的最新 O3 checkpoint作为输入后串行修改；
6. 保留原 timeout/concurrent/catch/checkpoint/stream cleanup全部验收，不因 owner拆分删减。

## 10. 测试矩阵

| 维度 | 必须覆盖 |
| --- | --- |
| Actor alias | direct field write、nested object mutation、Map/Array receiver mutation；first poll先 mutation再 Pending；提交后下一 segment可见 |
| Ready | native zero/ready、outbound buffered response、provider ready、callback ready、stream buffered next；不 commit/release/reacquire |
| Pending | native sleep/request、DB wait、outbound/Actor/provider/callback unary、stream next/emit；只 commit一次 |
| operation error | first poll error与 Pending后 error；不重启 side effect；需要时先 resume再 finalize error |
| drop/cancel | outbound lease、provider request、callback owner checkpoint、DB transaction/lease、stream source、file createFromStream全部 exactly once |
| Actor fence | pending后 instance replacement/stale epoch；resume fail closed且不安装 lease |
| nested concurrent | 两个 lane外部 wait可同时 Pending；同步 segment仍串行；lane/outer gate无泄漏 |
| DB特殊 | query不释放；transaction begin/body/commit/abort；claim/renew/body/release；rollback不误称 RequestHeap checkpoint能撤销原地 mutation |
| stream | serverStream创建不释放，消费 next按真实 Pending；natural End与异常 cleanup区分 |
| connection | WebSocket send不释放；requestJson的 requestId由协议层管理，业务调用按真实 poll处理 |
| 反向搜索 | `native_call_suspends`不存在；eval production不再有 pre-suspend pair；无 `yield_now`、unsafe heap alias、restart future |

## 11. 用户决策结论

当前不需要用户决定：

- actual-Pending、无显式 yield、Ready不释放、Pending才释放已经冻结；
- Actor field alias 是现有 production事实，不是新产品选择；
- owner prepare/wait/finalize只是实现所有权修正，不新增语言关键字、API语义或 wire格式。

后继只有在命中以下条件时才需要重新请求决策：

- 必须禁止或改变 `self.field.child` / mutable receiver 的持久化语义；
- 必须让 RequestHeap公开 mutation observer/transaction模型；
- transaction或lease在 cancel/drop下的用户可见语义无法沿现有合同保持；
- 为实现调度必须新增语言级 `yield` / `nosuspend`。

在当前证据下，这些都不是必要条件。推荐直接按 O1–O6 → J1 → E4R 的 DAG推进。
