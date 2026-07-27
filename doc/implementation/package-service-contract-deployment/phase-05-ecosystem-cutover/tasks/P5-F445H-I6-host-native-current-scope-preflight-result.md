# P5-F445H-I6 Host/native current-scope propagation preflight result

状态：`DECISION_REQUIRED`。

I6 不能按“只补 Host execution adapter，再改几处 native timeout”直接实现。只读追踪确认了三类
不同问题：

1. `RuntimeExecutionControl` / `RuntimeOwnedExecutionControl` 确实漏掉
   `execution_scope()` / `derive_scope(...)`；这部分合同完整、可以直接实现。
2. HTTP、WebSocket、time、file source、Actor 等 capability 多数在 request adapter 构造时
   冻结 request-start deadline / 单 token。要让每次 invocation 读取 current scope，至少会
   修改 `runtime/eval/**`、`runtime/capability-context/**` 和 `runtime/host/**`，已经超出原先
   “disjoint host/native owner”的边界。
3. canonical in-process service call 没有 consumer dependency timeout 或 callee operation
   timeout 的运行时数据；当前 Host operation metadata 也没有 reference 要求的 commit point、
   cancel-safety、idempotency、cleanup action、lower-cancel capability 和 bounded cleanup
   policy。T14 的完整 deadline 候选和 T16 的 unsupported-lower-cancel receipt 因此无法只靠
   I6 接线得到。

第 3 项不是代码位置没找到，而是当前 artifact / assembly / activation / native semantics 中
不存在所需事实。继续实现前需要决定：本轮是否扩展这些数据模型，还是明确缩小 I6/T14/T16
验收范围。第 13 节给出精确问题和建议。

另外，I6 production 的固定前置 E4 result 当前尚不存在；即使用户决策完成，也必须先等待
E4 集成结果。

## 1. 输入与只读边界

任务固定 integration：

```text
6d324555
```

preflight worktree：

```text
/Users/geek/workspace/skiff-p5-f445h-i6-preflight
branch codex/p5-f445h-i6-preflight
HEAD   010dc6869097830a6d869279f372a74d4c9fce91
```

`6d324555..010dc686` 只新增本 preflight task 文档；production 和 tests 为零 diff。当前没有
`P5-F445H-E4-...-result.md`，`eval_context.rs` 中四个 F445H evaluator arm仍保持 E4
fail-closed 状态。

本节点没有运行 crate gate、stable instance、live service或真实网络测试，也没有修改
production、tests、父文档、Cargo manifest或 lockfile。

## 2. Façade adapter：可直接实现的确定部分

### 2.1 当前包装路径

```text
host request entry
  -> skiff_runtime_request::ExecutionControl::new(root cancellation, ExecutionBudget)
  -> runtime/host/src/eval_capability_adapter/factory.rs::execution_control
  -> RuntimeExecutionControl(concrete owned control)
  -> capability_context::ExecutionControl
  -> EvalRequestExecutionCapabilities / ProgramExecutionInput
  -> ProgramExecutionContext 保存 capability OwnedExecutionControl
  -> program_execution::borrow_owned_execution_control
  -> 每次 context.execution() 暴露 current borrowed control
```

request concrete control 已经保存完整 `ExecutionScope`。丢失发生在
`runtime/host/src/eval_capability_adapter/execution.rs`：borrowed/owned adapter只转发
`deadline()` 和单个 `cancellation_token()`，scope API走 capability trait默认
`ExecutionScopeAccessError::Unavailable`。

### 2.2 精确实现

`RuntimeExecutionControl`：

- `execution_scope()` 返回 `Ok(self.0.execution_scope().clone())`；
- `derive_scope(local_deadline, site)` 调用 concrete
  `self.0.derive_scope(local_deadline, site)`，再包装为 capability
  `OwnedExecutionControl::new(RuntimeOwnedExecutionControl(derived))`；
- concrete error使用 `map_err(ExecutionScopeAccessError::from)`。

`RuntimeOwnedExecutionControl`：

- 显式实现同样的 `execution_scope()`；
- 显式实现同样的 `derive_scope(...)`。

owned trait默认经 `borrow()` 转发理论上也能工作，但显式实现可以锁定 owned path不因未来
borrow adapter变化退回 `Unavailable`。

当前 concrete derive只有 `ExecutionScopeDeriveError` 一个失败类型，语义是 `u32` nesting
overflow；capability 已有无损映射：

```text
ExecutionScopeDeriveError
  -> ExecutionScopeAccessError::Derive(the_same_error)
```

`Unavailable` 只表示 adapter没有保存 scope，不能拿 `deadline()` / token猜回 scope；不得把
derive error stringify、改成 `Unavailable` 或普通 timeout。

### 2.3 最小 tests

在 Host adapter附近用 concrete request `ExecutionBudget`、request cancellation source和两个
不同 source site构造 parent / child / grandchild：

- borrowed、owned、owned.borrow 三条路径都返回同一个 cloned
  `EffectiveDeadline::{at,source,site,nesting}`；
- inner earlier、outer earlier和 equal deadline（equal保留 outer owner）不变；
- request token取消后，三条 clone均观察 cancellation；
- 令 parent owner deadline到达并调用 parent `terminal_at(now)`，child clone观察
  parent-local cancellation为 ancestor cancellation；
- child自己的 local cancellation只影响 child，不污染 parent；
- 直接断言 `ExecutionScopeAccessError::from(ExecutionScopeDeriveError)` 保留
  `Derive` variant；无需执行 `u32::MAX` 次 derive。

capability-context现有 `scoped_execution_tests.rs` 已证明默认 `Unavailable`、deadline tie、
全部 signal和 lease lifecycle；request现有 `execution_control/tests.rs` 已证明 concrete nested
scope。Host新增测试必须证明的是两层 adapter没有丢字段或信号，不能只重复 leaf test。

## 3. 共同的 invocation-time 缺口

`ProgramExecutionContext::with_execution_control` 只替换字段 `execution`。其它 capability
context仍是 request construction时的 clone：

- `time`
- `file_source_stream`
- `http_client`
- `websocket`
- `actor`
- legacy `outbound`

`runtime/eval/src/native_capability.rs` 当前每次 native invocation虽然重新执行 projection，但
多数 projection getter又返回这些冻结 context：

```text
time()       -> context.time_context()                // frozen
file()       -> context.file_source_stream_context()  // frozen
http_client()-> context.http_client_context()         // frozen
websocket()  -> context.websocket_context()           // frozen
actor()      -> context.actor_context()                // frozen
```

只有 `http_response_stream()` 已经用 `context.execution()` 创建新 context，但其
`send_response_event` 又降级成单 request token。

所以 Host-only 修改无法满足 current child scope。E4 完成后，I6仍需一个 eval-owned
invocation execution carrier：projection在每次调用时携带 current control/scope，Host adapter
再把它交给 lower operation。这个 carrier是 Rust内部 capability contract，不改变 Skiff
业务参数。

## 4. Operation-by-operation owner表

### 4.1 HTTP、WebSocket与外部 transport

| operation | 当前完整路径 | 当前 scope读取 | lower接收与当前 settlement | 正确 owner / 最小变化 |
| --- | --- | --- | --- | --- |
| HTTP unary `std.http.client.request` | `native_capability.rs` → eval `RuntimeNativeHttpClientCapabilityContext` → Host `RuntimeHttpClientCapabilityContext` → `http_client_runtime.rs::HttpEffectRequest` → `http_runtime::transport::send_request` | `assembly_execution_context.rs` 构造 `HttpEffectContext` 时冻结 request剩余 `deadline_ms`和一个 root token | lower取 `min(input.timeoutMs, frame deadline ms)`，reqwest设置相对 timeout，并用单 token race response；winner靠 drop request future丢弃 late response，但没有 hermetic late-settlement probe | invocation carrier必须提供 full current `EffectiveDeadline`、signals/lease；Host HTTP call context保留 absolute deadline直到 operation start，再与 primitive timeout取 min；`transport.rs`需 fake driver seam证明 late response被丢弃 |
| HTTP body stream / SSE source | 同上，分别到 `open_body_stream...` / `open_sse...`，再成为 `StreamPullSource` | 同一个 request-start snapshot | lower stream保存 root token + stream-local token；每次 chunk/event race这些 signals；drop stream关闭连接的方向已有 | 创建 source时保存 current scope signals/lease，而不是 root token；自然 End与非自然 drop继续由 stream cleanup区分 |
| raw HTTP response `std.http.stream.emitResponse` | `native_capability.rs::http_response_stream` → shared `HttpResponseStreamCapabilityContext::send_response_event` → stream sink | projection已取 current control，但 shared context只调用 `execution.cancellation_token()` | sink接一个 stream cancel signal和一个 request token | `runtime/capability-context/src/stream.rs` 改用 current `execution_scope()?.cancellation_signals()`；actual-Pending与terminal owner仍由E4 |
| WebSocket request `requestJsonToConnection` | native三参数 dispatch → eval `WebsocketRequestCapabilityApi` → Host `RuntimeWebsocketRequestCapabilityContext` → concrete `WebsocketCapabilityContext` → `ConnectionRequestRegistry` | factory/rebinder构造时冻结 root token与root deadline | registry只接一个 token和`Option<Instant>`；已有 pending先安装、biased cancel-before-deadline、CAS settle、单 cancel callback、timer/lease归零、late `complete=false`和session fence | Skiff API保持三参数；eval内部request API携带 current execution；registry接 owned `CancellationSignals`和current absolute deadline（或等价 child lease scope），不得新增第4个业务参数 |
| 四个 WebSocket send | native → eval/Host synchronous `send_connection_*` → router channel `send` | 无 async wait；context只提供路由身份 | 同步返回，无 pending/timer/lease | E4 invocation前后checkpoint即可；不得为了 static `maySuspend` 或“可能发送”主动让出执行权 |

HTTP lower已经能接 `CancellationSignals`，但上层人为缩成一个 token；WebSocket registry则需要
共享 API扩展。HTTP现有 unit tests通过本机 TCP server，不满足 T13 的 hermetic要求；
test-effect double又立即返回，不能证明 pending cancellation和late discard，因此必须增加 fake
transport seam，不能把真实网络测试冒充 T13。

### 4.2 Service、Actor与其它 async operation

| operation | 当前完整路径 | 当前 scope读取 | lower settlement | 结论 |
| --- | --- | --- | --- | --- |
| canonical service unary | `assembly_execution::dispatch_service_call` → `RuntimeAssemblyServiceCallTarget` → `async_stream_cancel::execute_service_call` → provider in-process evaluator | `provider_execution_context` clone caller context，因此 current control本身可保留；但 `await_provider_unary` 仍只读旧 `deadline()`和一个 token | timeout/cancel调用 `provider_request.cancel()`；E4拥有改成full scope/lease、winner/late隔离 | 不经过 Host outbound relay；I6不得修改 retired relay来制造绿色测试 |
| canonical service server stream | 同上，provider producer + shared stream runtime | current owned control被保存，但 publication/item wait只用旧 deadline/token | request stream lease与producer cleanup已有，full-scope/owner closure属于E4 | T14必须基于canonical in-process fixture，不能基于旧 router frame |
| legacy service relay | `runtime/eval/src/service_dispatch.rs` → Host `outbound_service.rs` | request extra snapshot + `ServiceTimeoutConfig` | `OutboundRequestLease`可单 cancel frame和late隔离 | canonical assembly安装的是 `RetiredAssemblyOutboundServiceContext`；此路径只剩legacy/test consumer，列入禁止写集 |
| Actor get/replace/find/remove与spawn control | native/eval Actor API → Host actor adapter → `capability_context/actor.rs` control RPC | Actor context构造时冻结 root cancellation | `OutboundRequestLease` drop可删除pending并best-effort cancel一次；没有current absolute deadline | E4 current operation guard可以在winner后drop future并利用lease cleanup；若要求lower自己直接读full scope，则Actor capability API还需 invocation execution参数 |
| Actor method invoke | `eval/actor_dispatch.rs` → Actor API → Host `invoke_actor_method` | canonical assembly的 retired outbound返回None，因此 eval固定使用30s；Host只读request-start root token | Host select root cancel / 30s / response；custom `ActorMethodOutboundLease::drop`只删registry，不发cancel frame | current request/local deadline可能早于30s却不进wire；E4 drop future也不会发送Actor cancel frame。I6至少需把 current effective deadline/signals带到Host并加 exactly-once cancel guard |
| DB / interface / callback await | explicit eval IR或assembly callback path，不是 native required context | 当前由eval await site拥有 | lower API未统一接scope | E4任务已明确拥有这些 actual-Pending/checkpoint；I6只在E4 result留下Host gap时处理，不能抢写 |

canonical service的 `RuntimeAssemblyServiceCallTarget` 当前只有 provider request、contract、schema、
operation和 executable address；`ActivationServiceBinding`也只有 provider、contract和used
operations。这里没有 consumer dependency timeout。

provider `ActivationOwnedBindings`只有一个 deployment-wide `DeploymentPolicy.timeout_ms`，contract
operation descriptor没有 operation timeout。旧 `ServiceTimeoutConfig { default_ms, methods }`
仅存在于 legacy `ServiceUnit/RuntimeActivation`，canonical assembly context把它构造成
`Default::default()`；旧 service dispatch甚至把每个 operation的 `timeout_ms` 固定为 `None`。
因此不能把这些遗留字段称为已实现的 T14 dependency/callee constraints。

### 4.3 Time、file、stream与同步 native

| operation | 当前读取 | lower/cleanup | 正确边界 |
| --- | --- | --- | --- |
| `std.time.sleep` | projection返回request-start `TimeCapabilityContext`；native每10ms调用generic `poll_execution_budget()` | Tokio sleep可被drop，但generic error丢失local/inherited owner | projection每次用 `TimeCapabilityContext::new(context.execution())`；E4 operation guard/checkpoint决定owner。测试用paused/fake clock，不做长wall sleep |
| `core.date.now` | required context是Time，但实际走同步 registry handler | 无await/cleanup | invocation checkpoint足够，不需要lease |
| file create/read/readText/info/delete | file context不含execution | lower DB/file future没有统一cancel API | E4 current operation lease/race负责即时settlement与drop；是否能承诺unsupported-lower-cancel bounded cleanup取决于尚不存在的operation metadata/policy |
| `std.file.createFromStream` | file context同上；file-source context冻结request-start execution | `StreamConsumerCleanup`只在自然End后disarm，其它出口drop会cancel source；Host source next只用单 token | 每次projection用current execution重建 `FileSourceStreamContext`；Host `store.rs`用full cancellation signals；保留现有natural End / drop区分 |
| 普通 language stream `next()` | eval `program_stream.rs` / `program_invocation.rs` / service stream wait | 当前旧路径只用deadline/token；source cleanup RAII已有 | 全部属于E4；I6只修Host-created source与response sink，不复制E4 stream supervisor |
| config、resource、telemetry、JSON/crypto/date helpers等同步native | 不等待lower future | 无pending/lease；telemetry send也是同步context调用 | invocation前后checkpoint；无需给每个同步native制造scope adapter或调度让出点 |

`artifact-model/src/native_signature.rs::NativeCallableSemantics` 当前只有
`CallableMayEffects`和return provenance；`may_suspend`不是 lifecycle metadata。仓库没有
commit point、cleanup action、lower-cancel capability、grace/platform cleanup limit的typed
owner。因此 T16 中“lower不支持cancel时进入bounded cleanup”现在不能被完整实现或测试。

## 5. Deadline、cancel与lease合同

后继实现必须保持以下唯一组合：

1. invocation先读取 current `ExecutionScope`，保留完整 `EffectiveDeadline`，不能只留
   `Instant`后再猜source。
2. operation candidate是current effective deadline与该 operation已编码的 primitive constraint
   的最早 absolute deadline。service dependency/callee candidate必须等第13节的数据owner决定。
3. pending operation取得scope lease；lower收到 child scope/token或其完整 signals。normal
   completion只完成一次，winner/drop取消child一次。
4. race顺序固定为 ancestor cancellation、deadline、lower result；接纳一个ready lower
   result前再次检查scope terminal，防止scheduler先poll到late success。
5. inherited deadline保持internal `ScopeTerminalCarrier`；普通 native error conversion、
   inner catch和provider error channel都不得把它变成普通 `TimeoutError`。
6. 只有拥有local source/nesting的timeout wrapper物化 ordinary `TimeoutError`。local
   settlement不得调用 request `record_deadline_exceeded()`或取消request root token。
7. lower pending registry先原子删除/fence，再发送best-effort cancel；late
   value/error/response不能恢复pending、写caller heap或改变winner。
8. 不支持lower cancel时，用户可见terminal仍立即固定；后台清理是否满足bounded grace必须由
   typed metadata/policy证明，不能靠“drop通常会停止”声称完整。

## 6. Request/root boundary

inherited request deadline的最终物化已有按 ingress protocol划分的root owner，不应在普通
operation增加第二个owner：

- HTTP gateway与WebSocket connect：
  `runtime/host/src/host/request_entry/assembly.rs` 的outer `tokio::select!`；
  `prefer_cancel_then_deadline` 在接纳ready eval result前再次按 cancel→deadline顺序检查，
  并且只有这里记录 request-wide deadline telemetry。
- inbound WebSocket JSON-RPC：
  `runtime/request/src/websocket_jsonrpc_execution.rs::finalize_execution_terminal` 与
  `record_terminal_budget`；同样先cancel后deadline，再产生协议 outcome。

I6 ordinary operation只需保留/转发 internal terminal。E4 result进入后必须确认
`ScopeTerminalCarrier`能到达上述outer race，而不会先被
`RequestError::ordinary_payload()`或 native conversion吞掉。若outer timer/recheck已截获，
不改request/root boundary；只有存在可复现的terminal leak时才允许增加窄映射。

“唯一owner”是每种 ingress协议一个root owner，不是要求HTTP和WebSocket共用同一个函数。

## 7. 精确写集

### 7.1 不依赖第13节决策的 base I6 写集

在本 preflight 固定输入上，base I6 的 production 精确允许集是：

```text
runtime/host/src/eval_capability_adapter/execution.rs
runtime/eval/src/native_capability.rs
runtime/eval/src/capabilities.rs
runtime/eval/src/actor_dispatch.rs
runtime/capability-context/src/actor.rs
runtime/capability-context/src/actor_invocation.rs
runtime/capability-context/src/http.rs
runtime/capability-context/src/connection_request.rs
runtime/capability-context/src/stream.rs
runtime/host/src/eval_capability_adapter/effects.rs
runtime/host/src/eval_capability_adapter/http.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/eval_capability_adapter/assembly_execution_context.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/effect_context.rs
runtime/host/src/capability_context/store.rs
runtime/host/src/capability_context/websocket.rs
runtime/host/src/capability_context/actor_method_outbound.rs
runtime/host/src/host/http_client_runtime.rs
runtime/host/src/host/http_runtime/mod.rs
runtime/host/src/host/http_runtime/call_context.rs
runtime/host/src/host/http_runtime/transport.rs
runtime/host/src/host/http_runtime/request.rs
runtime/host/src/host/http_runtime/stream.rs
runtime/host/src/host/http_runtime/sse.rs
```

对应 tests 精确允许集：

```text
runtime/capability-context/src/connection_request_tests.rs
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs
runtime/host/src/eval_capability_adapter/execution.rs        # adapter unit tests可同文件
runtime/host/src/eval_capability_adapter/factory.rs          # 复用现有WebSocket fixture
runtime/eval/src/program_execution/execution_scope_tests.rs  # 仅在E4未已有跨projection receipt时
runtime/eval/src/assembly_execution/ordinary/tests.rs
runtime/eval/src/assembly_execution/ordinary/tests/current_scope_service.rs
```

实际 diff 可以是上述允许集的子集，不要求无条件改完每个文件。E4若已提供统一 invocation
guard或 full-scope projection，I6必须删掉重复项并只做最小delta；若 E4 result 证明必须写入
允许集之外的新文件，则应先更新 preflight，而不是静默扩张。

### 7.2 尚不能冻结的扩张写集

若要求 T14 完整覆盖 dependency/callee constraint，至少会新增/修改：

```text
artifact-model/src/runtime_assembly.rs
artifact-model/src/deployment.rs 或新的typed timeout-policy owner
compiler/driver的deployment/runtime-assembly projection
runtime/activation/src/context.rs
runtime/activation/src/request_context.rs
runtime/eval/src/assembly_seam.rs
runtime/eval/src/assembly_execution/async_stream_cancel.rs
runtime/loader与runtime/host loader admission对应owner/tests
```

具体字段不能在用户决定“dependency timeout与callee operation timeout由谁配置/持久化”之前
假装精确。

若要求 T16 完整覆盖 unsupported lower cancel的bounded cleanup，还至少涉及：

```text
artifact-model/src/native_signature.rs 或新的typed HostOperationLifecycleMetadata
runtime/native-contract/**
compiler/source/src/execution_semantics/**
runtime/eval的operation supervisor
对应Host provider cleanup executor与grace/platform-limit配置owner
```

这不是 base I6 的合理顺手修改，应先成为独立设计/production前置节点。

## 8. 禁止写集

在 base I6 中明确禁止：

- `syntax/**`、普通 source grammar和 std公开函数签名；
- `std.websocket.requestJsonToConnection` 第四个timeout参数；
- `api.yml`、ServiceContract业务参数或错误合同；
- `runtime/eval/src/service_dispatch.rs` 与
  `runtime/host/src/capability_context/outbound_service.rs` 的legacy relay复活；
- router wire protocol、真实网络server或stable instance；
- request root telemetry被local timeout写入；
- E4 result未集成前修改它的精确owner：
  `eval_context.rs`、`program_stream.rs`、`program_invocation.rs`、
  `assembly_execution/async_stream_cancel.rs`；
- Cargo manifests、lockfile和无关历史fixture。

第7.2节只有在用户明确选择扩张后才能解除相应禁止项。

## 9. 建议 DAG

```text
                     D0 用户冻结第13节两个范围决策
                                  |
                +-----------------+-----------------+
                |                                   |
        S0 service timeout数据owner          M0 Host lifecycle metadata
        （若选择本轮实现）                    （若选择本轮实现）
                |                                   |
                +-----------------+-----------------+
                                  |
                  E4 result + exact integration commit
                                  |
                                  v
                 I6-A shared invocation scope checkpoint
          façade adapter + eval内部carrier + shared capability API
                    /              |               \
                   v               v                v
             I6-B HTTP       I6-C WebSocket    I6-D time/file/Actor
             fake transport   pending registry  source/cleanup
                    \              |               /
                     +-------------+--------------+
                                   v
                    I6-E canonical service constraint
                    （依赖S0；E4 owner的最小delta）
                                   |
                                   v
                     I6-J join gate / T13–T16
```

I6-A 应独占 `runtime/eval/src/{native_capability,capabilities}.rs` 和 shared capability signature；
各consumer branch只在冻结后的API上实现，避免三个agent争写同一projection文件。

I6-B/C/D 的 lower文件可以互斥时并行；如果D仍需改 `actor_dispatch.rs`，由D单独拥有。
I6-E不能与E4并行，也不能在S0未冻结时启动。join gate必须在同一个integration commit上重跑
全部 crate gates；不要为了填槽继续拆分。

如果用户选择暂不实现 S0/M0，则必须新建缩小后的 I6 task，明确 T14只验证current caller
scope、T16只验证已有lower-cancel/RAII路径；不能沿用当前task并把缺失断言写成绿色。

## 10. T13–T16 test-first矩阵

所有 timer使用 paused Tokio或scripted monotonic clock；所有lower用barrier/oneshot/drop probe；
不得使用真实网络、wall-clock长sleep、Mongo、stable instance或live service。

| ID | 最小 fake与复用点 | 必须先出现的 RED | GREEN receipt |
| --- | --- | --- | --- |
| T13 HTTP | Host HTTP fake transport记录收到的current absolute deadline、signals和drop；复用scope fake，不复用本机TCP helper | local child存在时仍只看到request-start `deadline_ms`；late fake response可在winner后尝试settle | fake看到 `min(request,local,primitive)`；ancestor cancel同刻优先；operation lease/drop/cleanup各一次；late response未进入caller |
| T14 service | `assembly_execution::ordinary` canonical two-activation fixture、pending provider future、provider-request cancel probe | current path只能验证caller request/local，target没有dependency/callee candidate；旧单token wait可能丢owner | provider context携带tighter caller scope；冻结后的dependency/callee更早者获胜；provider lifecycle cancel一次；late provider value/error/heap write隔离。不得断言legacy router cancel frame |
| T15 WebSocket | 复用 `ConnectionRequestRegistry`与Host factory现有sender/registry/session fixture | child local scope下仍发送root deadline；registry只等root token | Skiff native仍严格三参数；pending先原子删除；单 `$/cancelRequest`；timer/lease/pending归零；late `complete=false`且不能影响另一session/request |
| T16 time/file/stream | paused sleep future、fake file provider、`StreamConsumerCleanup`/supervision现有drop probes、不合作lower future | derived child后time/file source仍读request-start control；非自然退出cleanup路径不完整 | current child deadline/cancel对sleep、file source、response/source stream可见；End只disarm，break/return/error/timeout/cancel/drop只cleanup一次；若M0落地，再证明不合作lower进入bounded grace且late结果隔离 |

T14需要一个可观察的 `RequestActivationContext` cancel-count test seam，或等价的一次性 lifecycle
transition receipt；当前 public API只有幂等 `cancel()`，没有次数probe。不得用多次调用“看起来没
报错”替代 exactly-once证据。

建议 focused命令（filter名称由实现task冻结，零测试不算通过）：

```bash
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-host f445h_i6_http -- --nocapture
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-eval f445h_i6_service -- --nocapture
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-capability-context connection_request -- --nocapture
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-host f445h_i6_websocket -- --nocapture
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-eval f445h_i6_time_file_stream -- --nocapture
```

预计完整 gates：

```bash
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-request --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-native --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-activation --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo test -p skiff-runtime-host --locked --no-fail-fast
CARGO_TARGET_DIR=<i6-worktree>/build/cargo-target \
  cargo check -p skiff-runtime-host -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

如果S0/M0落地，再加入其 artifact/compiler/native-contract/loader完整crate gate。

## 11. E4 result进入后的最小复查

不需要重做整份探查，只检查以下 delta：

1. 记录E4精确commit和实际写集，确认四个fail-closed arm已消失。
2. native/service/DB/interface future是否通过一个actual-Pending scope guard；Ready future是否
   不释放Actor segment。
3. guard是否在await前后都做owner-aware checkpoint，并使用
   `execution_scope().cancellation_signals()` / lease child scope。
4. `async_stream_cancel.rs`是否已经让canonical service unary/stream只cancel provider一次、
   丢弃late result并保留local/inherited owner。
5. `program_stream.rs` / `program_invocation.rs`是否已经覆盖ordinary stream cleanup；若已覆盖，
   从I6-D删除重复stream supervisor。
6. native返回generic deadline error时，E4是否先以current scope terminal覆盖该结果；若没有，
   E4尚未满足父合同，I6不得在每个native dispatch重复猜owner。
7. inherited request terminal是否稳定到达Host/request outer recheck；仅有真实RED时才改root
   boundary。

复查后必须缩减第7.1节写集；不能因为preflight列过文件就无条件全部修改。

## 12. 只属于 I7 的 handoff

以下不进入 I6：

- compiler source编译真实 `timeout(...)` + HTTP/WebSocket/file/service调用；
- artifact/File IR/link/identity golden和schema receipt；
- Agine `host_peer_rpc.skiff` 或其它仓库source compile；
- router真实wire、跨进程cancel frame、deployment rollout；
- stable instance、live account、chat smoke和浏览器验证；
-真实HTTP/WebSocket网络可靠性测试。

I6交给I7的是 hermetic runtime receipt、精确commit和零泄漏counter，不是 live成功截图。

## 13. 需要用户决定的问题

### 决策一：service timeout候选是否在本轮补齐

reference和当前T14要求同时存在：

- consumer dependency timeout；
- callee operation timeout。

当前没有两者的canonical authoring/artifact owner。建议选择：

1. **本轮补齐（建议）**：先单独设计/实现typed owner，再开始I6-E。consumer constraint归caller
   dependency edge，callee constraint归provider deployment的operation policy；两者都不进入
   `api.yml`业务签名。
2. **本轮缩小**：I6只实现request/local/current scope与已经存在的primitive constraint；
   T14明确删除dependency/callee断言，另建后续任务。
3. **修改reference**：取消这两个候选。当前没有证据支持这样做，不建议。

还需明确现有 deployment-wide `DeploymentPolicy.timeout_ms` 是整个ingress/request policy，
还是允许同时充当callee operation default；不能由实现者根据字段名自行决定。

### 决策二：Host operation lifecycle metadata是否在本轮补齐

建议选择：

1. **先建独立前置（建议）**：增加typed lifecycle metadata与bounded cleanup policy，然后T16
   完整验收不支持lower cancel的operation。
2. **本轮缩小**：I6只承诺已有lower cancel、future drop和stream RAII路径；T16不宣称已满足
   通用bounded background cleanup。

在这两个问题冻结前，结论保持：

```text
DECISION_REQUIRED
```

不是 timeout/cancel优先级不清楚；优先级已由父文档冻结。阻塞点是所需运行时事实尚无唯一
数据owner，且完整实现会跨越 artifact/compiler/activation/loader，不能在I6中静默发明。
