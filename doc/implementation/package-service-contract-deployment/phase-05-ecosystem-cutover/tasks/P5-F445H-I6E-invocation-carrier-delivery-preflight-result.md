# P5-F445H-I6E invocation carrier delivery seam preflight result

状态：

```text
PREFLIGHT_COMPLETE
DECISION_REQUIRED = NO
TASK_SCOPE_EXPANDED = NO
READY_FOR_I6_RESUME_DAG = YES
```

I6-A 已经在每次 native projection 捕获一次 current
`OwnedExecutionControl`，但 HTTP、WebSocket request、time、file 与 Actor/spawn 的真实
suspending consumer 仍没有收到它。本 result 冻结一个可单独编译的 shared delivery checkpoint；
checkpoint 后五个互斥 consumer 可以完全并行。本节点没有修改 production、tests、fixture、
Cargo/lockfile、父任务或权威设计。

## 1. 冻结输入与只读边界

| 项 | commit / tree |
| --- | --- |
| 合同固定 production base | `1000d290ce9ebc3cd5a792cf01f27b5835496a2a` |
| 合同固定 production tree | `90c69b694fb38c7ec544149aec3b87a3b632496c` |
| 本 task 发布 HEAD | `505ff6a2f22301b21b0b0f204df623044bc4ef48` |
| 本 task 发布 tree | `25efa284f3baff5c9426c2297c4b0860e76ea4e5` |

`1000d290..505ff6a2` 只新增本 task 合同；production/tests 与固定 base 相同。源码结论因此锚定
`1000d290` / `90c69b69`，任务编排锚定 `505ff6a2` / `25efa284`。

本预检只使用 `git`、`rg` 与源码/文档阅读。没有运行 Cargo、build、test、完整 gate，没有访问
stable/live/network/MongoDB，也没有 merge、rebase 或 push。三个只读分片分别核对 HTTP +
WebSocket、time + file、Actor + spawn；分片没有继续委派，最终接口、写集、DAG 与 verdict 由本
result 统一。

当前树还纠正了合同中的两个路径：

- `runtime/host/src/host/file_stream.rs` 不存在；Host file adapter 是
  `runtime/host/src/eval_capability_adapter/file_stream.rs`，真实 channel/pull waiter 位于
  `runtime/host/src/capability_context/stream_runtime.rs`。
- `runtime/eval/src/actor/dispatch.rs` 不存在；实际 owner 是
  `runtime/eval/src/actor_dispatch.rs` 与
  `runtime/eval/src/actor_dispatch/prepared_operation.rs`。

## 2. 共同内部接口

### 2.1 唯一 carrier

所有链统一传递现有
`skiff_runtime_capability_context::OwnedExecutionControl`，不新增第二种 carrier，也不传裸
deadline/token。

```text
RuntimeNativeCapabilityProjectionSource::new
  -> context.execution().owned()                         # I6-A 唯一 current read
  -> RuntimeNativeInvocationExecutionControl
  -> execution_control().clone()                         # cheap owned clone
  -> internal operation trait/context method argument
  -> Host/native concrete operation start
  -> execution_scope() exactly once
  -> real pending owner acquires the only scope lease
```

内部 operation 方法按值接收 `OwnedExecutionControl`。它本身是 `Arc` façade；clone 只延长同一
invocation control 的 owned lifetime，不创建 lease、timer 或 waiter。下层 `execution_scope()`
取得的 scope clone保留同一 ancestor/local signals、absolute `EffectiveDeadline`、deadline
source/site/nesting 与 lifecycle。consumer 的 `acquire_lease()` 才登记 waiter/timer，并由该
consumer 的 completion owner在 normal/terminal/drop路径归零。

不选择其它表示的原因：

- Eval-private `RuntimeNativeInvocationExecutionControl` 不能被 Host crate命名，向外公开它只会制造
  第二个公共抽象。
- 在 Eval/native projection提前取 `ExecutionScope` 会把真正的 operation-start read挪到错误层，
  并鼓励提前把 absolute deadline重冻为 remaining milliseconds。
- root cancellation token + relative deadline不能恢复 deadline owner、nested signals或 lifecycle，
  明确禁止作为伪 current scope。

### 2.2 current read 与 waiter owner

| 链 | 第一次读取 full current scope | 唯一 pending / lease owner |
| --- | --- | --- |
| HTTP unary/body-open/SSE-open | `HttpEffectRequest::new`，从本次 dispatch传入的 owned control读取 | `http_client_runtime.rs` 的共享 scoped lower-future helper；它完成或drop lower request/open future |
| WebSocket request | Host `RuntimeWebsocketRequestCapabilityContext::request_json_to_connection` | `ConnectionRequestRegistry::install` / `PendingConnectionRequest`; registry CAS settle、清 pending/timer/lease后再发 best-effort hint |
| `std.time.sleep` | `TimeNativeDispatch::prepare` 取得 owned control并读取 scope | `sleep_for_millis` 的 scope lease与 Tokio sleep future；零时长保持首 poll Ready且不 acquire lease |
| file direct/provider/source | Host `RuntimeFileCapabilityContext` / `RuntimeOwnedFileSourceStreamContext` 每次 operation开始读取 | file adapter 的 scoped lower-future owner；source lower仍是现有 channel/pull waiter，adapter只拥有外围 scope lease |
| Actor control/spawn | Host Actor adapter收到每次 call carrier后读取 | `await_control_response` 同时拥有 `OutboundRequestLease` 与 receiver |
| Actor method | carrier随 `PreparedActorMethodInvocation` owned wait进入 Host后读取 | `invoke_actor_method` 同时拥有 scope lease、`ActorMethodOutboundLease`、timer与router sender |

native projection不调用 `execution_scope()`、不 `acquire_lease()`、不读取 clock、不计算 remaining
milliseconds。HTTP primitive timeout、Actor 30s primitive与 WebSocket/Actor wire deadline只在真实
operation start基于 current absolute deadline计算；不改变 I6R 已冻结的 winner/error语义。

### 2.3 内部 trait 形态

shared checkpoint只改变 workspace 内 Rust seam：

- `HttpClientCapabilityApi` / `HttpClientCapabilityContext` 的三个 suspending dispatch接收
  `OwnedExecutionControl`；
- `FileCapabilityApi` / `FileCapabilityContext` 的六个 operation，以及
  `FileSourceStreamApi` / `FileSourceStreamContext::next_file_source_stream_item` 接收它；
- `ActorCapabilityApi` / `ActorCapabilityContext` / `ActorClient` 的
  get-or-create、replace、find、remove、spawn、method六个 suspending operation接收它；
- Eval `WebsocketRequestCapabilityApi` 与
  `WebsocketCapabilityContext::request_json_to_connection` 接收它；
- `NativeTimeCapability` 增加同步 owned-control getter；现有同步
  `poll_execution_budget()` 不改成 async，非 sleep time helper不增加 suspension。

HTTP/WebSocket/Actor/file 的 `runtime/native/src/capability.rs` trait方法不需要增加业务参数：
其 `self` 已经是 I6-A 带 carrier 的 Eval wrapper。只有 time dispatch自身位于 native crate，因而
`NativeTimeCapability` 需要内部 getter。

## 3. 五条真实调用链

### 3.1 HTTP unary、body-stream open、SSE open

```text
EvalContext::eval_native_prepared_call
-> project_runtime_execution_native_capability_context
-> RuntimeNativeCapabilityProjectionSource::new
-> RuntimeNativeCapabilityProjectionSource::http_client
-> RuntimeNativeHttpClientCapabilityContext::new(lower context, doubles, carrier)
-> NativeDispatch / HttpNativeDispatch::prepare
-> NativeHttpClientCapability::{dispatch_http_request,dispatch_http_stream,dispatch_http_sse}
-> eval RuntimeNativeHttpClientCapabilityContext impl
-> capability-context HttpClientCapabilityContext / HttpClientCapabilityApi
-> Host RuntimeHttpClientCapabilityContext adapter
-> concrete HttpClientCapabilityContext methods in http_client_runtime.rs
-> HttpEffectRequest::new(current owned control)
-> request_with_cancellation_and_options
   | open_body_stream_with_cancellation_and_options
   | open_sse_with_cancellation_and_options
-> HttpCallContext -> transport::send_request
```

当前丢失点是 `runtime/eval/src/capabilities.rs` 的三个 native dispatch：它们只 clone lower
`context`，从不传 `invocation_execution`。当前 `HttpEffectRequest::new` 因而只能读取
request-construction `HttpEffectContext.deadline_ms` 与 root cancellation。

body/SSE handle建立后的 `next`、natural End与非End cleanup继续属于 E4，I6 HTTP只拥有 open
future。

### 3.2 WebSocket request

```text
projection source -> RuntimeNativeWebsocketCapabilityContext::new(context, carrier)
-> WebsocketNativeDispatch::prepare_request
-> NativeWebsocketCapability::request_json_to_connection
-> eval RuntimeNativeWebsocketCapabilityContext impl
-> eval WebsocketCapabilityContext
-> WebsocketRequestCapabilityApi
-> Host RuntimeWebsocketRequestCapabilityContext
-> concrete WebsocketCapabilityContext::request_json_to_connection
-> ConnectionRequestRegistry::install(scope)
-> PendingConnectionRequest::wait
```

当前丢失点是 Eval native impl和三参数内部 request trait；Host adapter仍从
`RuntimeConnectionRequestParts` 读取 request-root token/deadline。普通四个 send不经过新增
carrier参数，继续是同步 Ready。

### 3.3 `std.time.sleep`

```text
projection source -> RuntimeNativeTimeCapabilityContext(lower TimeCapabilityContext, carrier)
-> TimeNativeDispatch::prepare
-> NativeTimeCapability owned-control getter
-> sleep_for_millis(scope, duration)
-> race scope lease with one Tokio sleep future
```

当前 `NativeTimeCapability` 只有 `poll_execution_budget()`，`sleep_for_millis` 每 10ms poll旧
`TimeCapabilityContext`。新 seam只给 sleep取本次 invocation owned control；duration决定 normal
wake，不成为第二个 scope deadline。decode、clamp、零时长与其它同步 time helper不改成 Pending。

### 3.4 file direct、provider、source stream

```text
projection source -> RuntimeNativeFileCapabilityContext
-> RuntimeNativeFileCapability / RuntimeNativeFileSourceStreamCapability (same carrier)
-> FileNativeDispatch
-> eval NativeFileCapability / NativeFileSourceStreamCapability impl
-> capability-context FileCapabilityContext / FileSourceStreamContext
-> Host RuntimeFileCapabilityContext / RuntimeOwnedFileSourceStreamContext
-> concrete FileCapabilityContext or FileSourceStreamContext
-> FileRuntime / StreamRuntime channel-or-pull waiter
```

当前六个 direct/provider delegation与 source `next`都只使用旧 lower context，忽略各自已持有的
carrier。shared seam在 Host adapter接收 carrier；file consumer在这里拥有唯一外围 scope lease并
drop lower future。已进入 blob/DB/spawn-blocking effect后结果可以 unknown，不宣称撤销；但
`FileIngest` 的临时路径必须具有 drop cleanup，避免 scope winner在 ingest中途留下 temp file。

### 3.5 Actor control、method、spawn

Actor control：

```text
projection source -> RuntimeNativeActorCapabilityContext(context, carrier)
-> ActorNativeDispatch
-> eval NativeActorCapability impl
-> capability-context ActorClient / ActorCapabilityApi
-> Host RuntimeActorCapabilityContext
-> concrete ActorClient::{get_or_create,replace,find,remove}
-> send_raw_control_request
-> OutboundRequestRegistry::insert_with_lease
-> await_control_response
```

Actor method不经过 native projection：

```text
EvalContext::eval_actor_dispatch
-> actor_dispatch::prepare_actor_method(current execution owned)
-> PreparedActorMethodInvocation
-> OwnedActorCapabilityContext::invoke_actor(carrier, request)
-> Host invoke_actor_method
-> ActorMethodOutboundRegistry::register
-> send Invoke frame
-> race scope/current+30s primitive with ActorMethodOutboundLease::receive
```

spawn：

```text
EvalContext Spawn statement
-> submit_spawn_statement(current execution owned)
-> ActorClient::submit_spawn(carrier, request, payload)
-> Host submit_spawn_and_wake
-> concrete ActorClient::submit_spawn
-> await_control_response
-> only valid receipt wakes SpawnWorkerRegistry
```

当前 native control在 Eval wrapper丢 carrier；method的 prepared object没有 carrier；spawn只 clone
request-construction context。Actor control/spawn的真实 response owner是 `OutboundRequestLease`，
method的真实 response owner是 `ActorMethodOutboundLease`; 两者 Drop都已提供 late response fence。

## 4. 精确写集

### 4.1 E1 shared delivery checkpoint

唯一共同 production owner与接口：

```text
runtime/eval/src/capabilities.rs
  RuntimeNativeInvocationExecutionControl::execution_control
  WebsocketRequestCapabilityApi / WebsocketCapabilityContext request method
  RuntimeNative{HttpClient,Websocket,Time,File,FileSourceStream,Actor} impls

runtime/capability-context/src/http.rs
  HttpClientCapabilityApi + HttpClientCapabilityContext three dispatch methods

runtime/capability-context/src/file.rs
  FileCapabilityApi/FileCapabilityContext six operations
  FileSourceStreamApi/FileSourceStreamContext next operation

runtime/capability-context/src/actor.rs
  ActorCapabilityApi/ActorCapabilityContext/ActorClient six suspending operations

runtime/native/src/capability.rs
  NativeTimeCapability owned-control getter only
```

真实 adapter接收与 production caller delivery：

```text
runtime/host/src/eval_capability_adapter/http.rs
  RuntimeHttpClientCapabilityContext three carrier parameters

runtime/host/src/eval_capability_adapter/file_stream.rs
  RuntimeFileCapabilityContext + RuntimeOwnedFileSourceStreamContext parameters

runtime/host/src/eval_capability_adapter/websocket.rs
  RuntimeWebsocketRequestCapabilityContext parameter

runtime/host/src/eval_capability_adapter/actor.rs
  borrowed/owned Actor impls, submit_spawn_and_wake, invoke_actor_method parameters

runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/eval/src/spawn_ops.rs
  current owned capture/storage/forwarding only; no winner or error behavior

runtime/host/src/capability_context/native_projection.rs
  alternate NativeTimeCapability impl getter, compile-only follow-through
```

共享 receipt与纯机械 fixture跟随：

```text
runtime/eval/src/program_execution/execution_scope_tests.rs
  f445h_i6_carrier_delivery_receipt

runtime/native/src/dispatch/prepared_tests.rs
  CountingTimeContext getter

runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/tests/f445h_e4r_combined/capability_harness.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/actor_dispatch.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/support.rs
runtime/host/src/eval_capability_adapter/factory.rs
```

这些 fixture只允许补 `OwnedExecutionControl` 参数或记录 receipt，不修改 E4断言、Pending结构或
业务结果。`runtime/eval/src/capabilities.rs` 是五条链的唯一共同冲突文件，必须由 E1 的一个 owner
一次修改；五个 consumer不得各自再写它。

### 4.2 E2 HTTP consumer

Production：

```text
runtime/host/src/capability_context/http.rs
  scoped concrete dispatch entrypoints
runtime/host/src/host/http_client_runtime.rs
  HttpEffectRequest::new + three dispatch + shared scope/lower-future owner
runtime/host/src/host/http_runtime/transport.rs
  current-vs-primitive timeout and ordinary primitive TimeoutError
```

Tests：

```text
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs              # new
runtime/host/src/host/http_runtime/tests/request.rs
runtime/host/src/host/http_client_runtime.rs                           # private fake lower seam
```

现有 `call_context.rs`、`request.rs`、`stream.rs`、`sse.rs` 可以继续接收 `None` 作为旧 frame deadline，
由 scoped outer helper拥有 current scope；当前事实不要求修改它们。若实现选择删除旧
frame-deadline plumbing，则这四个文件必须加入同一个 E2 owner，不能作为并行 cleanup。
`effect_context.rs` 的 request-root字段清理会机械波及多个 request constructor，不属于最短行为闭环。

### 4.3 E3 WebSocket request consumer

Production：

```text
runtime/capability-context/src/connection_request.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/eval_capability_adapter/assembly_execution_context.rs
runtime/host/src/capability_context/websocket.rs
```

Tests：

```text
runtime/capability-context/src/connection_request_tests.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
runtime/host/src/eval_capability_adapter/carrier_delivery_tests.rs     # new vertical receipt
runtime/host/src/eval_capability_adapter/mod.rs                        # test module declaration
runtime/host/src/host/router_session/tests.rs                          # direct install mechanical caller
```

`RuntimeConnectionRequestParts` 只保留 registry/session；factory不再冻结 root token/deadline。
`assembly_execution_context.rs` 的两个 production constructor caller是合同漏列的机械 owner。

### 4.4 E4 time consumer

```text
runtime/native/src/dispatch/time.rs
  TimeNativeDispatch::prepare / sleep_for_millis + inline tests
runtime/eval/src/program_execution/execution_scope_tests.rs
  f445h_i6_time_projection_to_pending vertical integration receipt
```

`runtime/capability-context/src/time.rs` 不变；它已有 owned/borrowed execution façade。同步 decode/clamp与
非 sleep helper不增加 lease、timer或 yield。

### 4.5 E5 file consumer

Production：

```text
runtime/host/src/eval_capability_adapter/file_stream.rs
  one shared scoped lower-future owner for six direct/provider operations and source next
runtime/host/src/host/file_runtime.rs
  FileIngest drop cleanup; existing StagedFile drop owner retained
```

Tests：

```text
runtime/host/src/eval_capability_adapter/file_stream_tests.rs          # new
runtime/host/src/host/file_runtime/tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/support.rs
```

已核对但不写：

- `runtime/host/src/capability_context/store.rs` 继续做 provider/store选择；
- `runtime/host/src/capability_context/stream_runtime.rs` 继续拥有 channel/pull waiter；
- `runtime/host/src/host/file_stream.rs` 不存在。

若实现把 scope lease下沉到 `store.rs`，它必须替代 adapter owner，而不是两层同时 acquire；这会改变本
result 冻结的最短写集并触发停止。

### 4.6 E6 Actor control/method/spawn consumer

Production：

```text
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/eval/src/spawn_ops.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

Tests：

```text
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

`runtime/native/src/dispatch/actor.rs` 与 `runtime/native/src/capability.rs::NativeActorCapability`
不改；带 carrier 的 Eval wrapper已经是交付点。不存在单独 Actor `get` / `create` native目标，当前
surface是 get-or-create、replace、find、remove。

## 5. 最短可并行实现 DAG

```text
E1 shared OwnedExecutionControl delivery checkpoint
├── E2 HTTP unary/body-open/SSE-open current-scope consumer
├── E3 WebSocket request registry current-scope consumer
├── E4 time sleep current-scope consumer
├── E5 file direct/provider/source current-scope consumer
└── E6 Actor control/method/spawn current-scope consumer
      |
      +-- all five results -> existing I6-J combined probe -> independent I6 acceptance
```

可以先完成一个单独可编译的 E1。原因是 carrier类型已经位于所有相关 crate共同依赖的
`runtime/capability-context`，不需要新增依赖边或公开 type。E1把 method参数与所有仓内 impl/caller
机械跟随一次完成，Host/native lower consumer暂不 acquire lease、不改变 winner；receipt只证明
owned carrier确实离开 wrapper并到达内部 lower API/adapter。

E1 后 E2–E6 production写集互斥，可以完全并行。它们不会再争用
`runtime/eval/src/capabilities.rs`、`runtime/capability-context/src/{http,file,actor}.rs` 或
`runtime/native/src/capability.rs`。不存在第二个串行 owner checkpoint。

consumer自己的行为文件与 E1 adapter文件存在有意的串行重用：
HTTP adapter、WebSocket adapter、file adapter、Actor adapter只能先由 E1完成签名，再由各自唯一
consumer继续行为；不能把同一文件的 E1/E2–E6部分伪装成并行提交。

### 5.1 节点合同

| 节点 | 直接父文档 | base | 唯一结果文档 |
| --- | --- | --- | --- |
| E1 | 本 result、`P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`、`P5-F445H-I6B-http-current-scope-result.md`、`P5-F445H-I6C-websocket-request-current-scope-result.md`、`P5-F445H-I6D-host-operation-current-scope-result.md` | production `1000d290ce9ebc3cd5a792cf01f27b5835496a2a` / `90c69b694fb38c7ec544149aec3b87a3b632496c`; task/result docs可叠加但不得改变production tree | `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md` |
| E2 | E1 result、`P5-F445H-I6B-http-current-scope-result.md` | E1 implementation commit/tree（必须由 E1 result给出精确 hash，预检不伪造） | `P5-F445H-I6E2-http-current-scope-resume-result.md` |
| E3 | E1 result、`P5-F445H-I6C-websocket-request-current-scope-result.md`、`P5-F445H-D2-websocket-peer-cancel-hard-cut-result.md` | E1 implementation commit/tree | `P5-F445H-I6E3-websocket-current-scope-resume-result.md` |
| E4 | E1 result、`P5-F445H-I6D-host-operation-current-scope-result.md` | E1 implementation commit/tree | `P5-F445H-I6E4-time-current-scope-resume-result.md` |
| E5 | E1 result、`P5-F445H-I6D-host-operation-current-scope-result.md` | E1 implementation commit/tree | `P5-F445H-I6E5-file-current-scope-resume-result.md` |
| E6 | E1 result、`P5-F445H-I6D-host-operation-current-scope-result.md` | E1 implementation commit/tree | `P5-F445H-I6E6-actor-current-scope-resume-result.md` |

每个下游 task发行时必须写入 E1 的真实 implementation commit/tree；本预检不能提前发明未知 hash。

### 5.2 最小非零测试与停止条件

| 节点 | 最小非零测试 | 节点停止条件 |
| --- | --- | --- |
| E1 | Eval `f445h_i6_carrier_delivery_receipt` list/run；四 crate locked check | 任一 carrier需要公开 Skiff/native参数、artifact/wire、dependency cycle，或无法在不 acquire lease下单独编译 |
| E2 | Host `f445h_i6_http_current_scope` list/run | 需要 E4 stream owner、HTTP ingress、Router、真实 network，或新公开 timeout/error surface |
| E3 | capability-context `f445h_i6_connection_request_scope` 与 Host `f445h_i6_websocket_scope` list/run | 需要 Router/wire、peer cancel、业务第四参数，或 local correctness依赖 hint ack |
| E4 | native `f445h_i6_time_scope` 与 Eval `f445h_i6_time_projection_to_pending` list/run | 需要修改 `RuntimeNativeInvocation`/artifact，或同步 helper被迫变 Pending/yield |
| E5 | Host `f445h_i6_file_scope` 与 Eval create-from-stream receipt list/run | 需要 DB/blob rollback承诺、全局 cleanup supervisor，或 scope lease必须移出冻结 adapter owner |
| E6 | Eval + Host `f445h_i6_actor_scope` list/run | 需要 Actor/Router wire schema、新公开 cancel/lifecycle metadata，或 existing post-await checkpoint不能恢复 current owner而必须新增跨层错误契约 |

所有 selector listing必须非零且与 execution数量一致；consumer测试使用 paused Tokio clock、
barrier/oneshot/drop counter/fake registry/provider，不访问 network、stable/live或 MongoDB。

## 6. 真实 receipt

### 6.1 E1 delivery receipt

E1 在 `runtime/eval/src/program_execution/execution_scope_tests.rs` 扩展现有
`ScopeAwareControl` / nested derived context fixture：

1. 构造 root仍 active、inner child有更早 absolute deadline的 current execution；
2. 从 `project_runtime_native_capability_context(...Websocket)` 得到真实
   `RuntimeNativeWebsocketCapabilityContext`；
3. 真正调用 `NativeWebsocketCapability::request_json_to_connection`，而不是读取
   `invocation_execution()` accessor；
4. recording `WebsocketRequestCapabilityApi` 从收到的 `OwnedExecutionControl` 调
   `execution_scope()`，断言 nesting、effective deadline owner、signals与 lifecycle等于 inner
   expected；
5. 返回 ready terminal并断言没有 lease/timer/waiter被 E1制造。

这证明 carrier穿过 native wrapper、新内部 method seam到达 lower API object，满足 shared
checkpoint receipt；现有只断言 wrapper字段的 I6-A测试不能替代它。

### 6.2 第一条 projection 到真实 pending consumer闭环

E3 是 canonical 第一条纵向闭环 owner。其新
`runtime/host/src/eval_capability_adapter/carrier_delivery_tests.rs` 必须在一个测试内走：

```text
Host-backed ProgramExecutionContext
-> native WebSocket projection
-> Eval wrapper + internal request trait
-> RuntimeWebsocketRequestCapabilityContext
-> concrete WebsocketCapabilityContext
-> real ConnectionRequestRegistry pending
-> derived child deadline or ancestor stop
-> local CAS settlement + pending/timer/lease == 0
-> late complete == false
```

这条 receipt完全内存，不依赖 Router server/network。它是 I6-A 之后第一条被指定为
projection-to-real-pending 的证据 owner；E1在此之前仍只是内部 delivery checkpoint。E2、E4、E5、
E6不依赖 E3，可以并行实现，但各自仍须建立自己的真实 consumer receipt。

其余 node 至少覆盖：

- HTTP：unary/body-open/SSE-open 的 current deadline、ancestor/internal stop、normal、late lower
  drop，primitive/current owner与所有 counter归零；
- time：zero Ready无 lease，normal sleep、current deadline、ancestor stop及 timer/lease归零；
- file：direct/provider/source Pending、normal、current winner、late finalize隔离、natural
  End/non-End cleanup与 temp staging归零；
- Actor：control与spawn共用 response waiter；method `min(current,30s)`；normal、current winner、
  best-effort hint、late receipt/outcome fence及 registry/scope counters归零。

## 7. public surface、wire 与额外 production shape

以下都不修改：

- `std.http.*`、`std.time.sleep`、`std.file.*`、Actor与 spawn语言表面；
- `requestJsonToConnection(connectionId, method, value)` 三参数与四个同步 send；
- native binding、`RuntimeNativeInvocation`、artifact/schema/compiler；
- Router protocol、ConnectionRequest/Actor frame schema、peer cancellation规则；
- Cargo manifest与 lockfile。

carrier不序列化，只在同一 Runtime进程内传到 local pending owner。WebSocket/Actor继续使用既有 wire
deadline与 internal hint字段，只改变 operation-start值与本地 winner；HTTP/file/time不新增 wire。
workspace 内 `pub trait` 的仓内签名跟随是内部 Rust compile checkpoint，不是公开 Skiff/native API
变化。

`runtime/host/src/capability_context/native_projection.rs` 还导出一组 Host-side promoted native
contexts。全仓搜索只有本文件定义与 `capability_context/mod.rs` re-export，没有 constructor/use
callsite；它不在本 result列出的 Eval production路径。E1只为 `NativeTimeCapability` 新 getter做
机械编译跟随，不用 request-root token伪装 HTTP/file/Actor current scope。若后续证明该导出仍是可达
production入口，应单独返回 `TASK_SCOPE_EXPANDED` 并决定退休或建立真实 carrier owner，不能在五个
consumer中暗建第二路径。

## 8. 结论

五条链没有互相冲突的公共语义；共同类型、依赖方向、current read点与 pending owner均可精确冻结。
唯一共同 conflict file由 E1独占，E1可单独编译并以真实 method receipt结束；随后五个 consumer写集
互斥、无需固定波次数或伪并行。

```text
READY_FOR_I6_RESUME_DAG = YES
```
