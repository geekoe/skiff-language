# P5-F445H-I6R current-scope refresh preflight result

状态：`DECISION_REQUIRED`。

```text
READY_FOR_I6_DAG = PARTIAL
TASK_SCOPE_EXPANDED = NO
TASK_NOT_EXECUTABLE = NO
I6_UNBLOCKED_BY_E4R = YES
```

E4R 已完成 evaluator actual-Pending、timeout/catch、concurrent、Actor continuation、
activation、program stream、response/source stream consumer 与 canonical service wait 的 owner。
当前剩余 Host/native delta 可以形成有界 DAG；唯一不能由 repository 事实唯一推出的语义是
`consumer dependency timeout` 与 `callee operation timeout` 的 authoring、持久化、粒度和优先级。
因此不能宣称完整 `READY_FOR_I6_DAG`，但不依赖该决策的共享 checkpoint 与三个 consumer 节点可以继续。

## 1. 冻结输入与只读边界

| 输入 | commit | tree |
| --- | --- | --- |
| integration | `1c042d207c278f0f69d27e12ffee671898dc8985` | `348a85bf8a2a9d5323efe68313209f14fa81504e` |
| production/tests | `bf55ede018526751a2db101a42900c4e07fe08a8` | `61323e4772061c3b50abc189712767bde716ea24` |
| 本 task 发布 HEAD | `838b0c2d4c9c06e8c2042ebc7a02190b270b1215` | `3fb42c19bc2d1bcbdb067d9bdafd019e6152b024` |

`bf55ede0..838b0c2d` 只新增 E4R task/result 与本 task；`1c042d2..838b0c2d`
只新增本 task。production/tests 在当前 worktree 与冻结 tree 精确相同，所以以下源码结论全部锚定
`bf55ede0` / `61323e47`，父结果与任务编排锚定 `1c042d2` / `348a85bf`。

本节点只使用 `git`、`rg` 和源码/文档阅读。没有运行 Cargo、测试、完整 gate、network、stable
instance、live service、MongoDB 或其它昂贵检查；没有修改 production/tests、权威设计、父结果、
manifest 或 lockfile，也没有派子 Agent。

## 2. E4R 后的最小 delta

### 2.1 旧 I6 §11 逐项分类

| 项 | 分类 | 当前事实 |
| --- | --- | --- |
| capability façade borrowed/owned execution control | `STILL_MISSING_IN_I6` | `RuntimeExecutionControl` 与 `RuntimeOwnedExecutionControl` 仍只转发 root token/deadline/budget；full scope API走 `Unavailable` 默认。 |
| native invocation projection | `STILL_MISSING_IN_I6` | projection 确实每次调用重新执行，但 Actor/file/time/HTTP/WebSocket getter仍 clone request-construction snapshot；只有 HTTP response sink显式取 `context.execution()`。 |
| HTTP unary/open stream/open SSE | `STILL_MISSING_IN_I6` | lower仍收到 request-start relative `deadline_ms` 与 root token；E4 actual-Pending只做前后 checkpoint，不在 pending期间建立 current-scope race。 |
| HTTP body/SSE 已创建 source 的 ordinary consumer wait/cleanup | `SATISFIED_BY_E4R` | `program_stream/current_scope.rs` 与 `program_invocation/current_scope.rs` 在每次 `next` 读取 current scope；`StreamConsumerCleanup` 只在真实 End disarm，其它退出 cleanup。open 操作本身仍缺。 |
| HTTP response/source sink wait | `STILL_MISSING_IN_I6` | response sink虽按调用时构造 context，但只给 sink 一个 root `cancellation_token()`，没有 current deadline timer或全部signals。 |
| WebSocket 四个普通 send | `SATISFIED_BY_E4R` | native dispatch返回 `PreparedNativeCall::Ready`，底层是同步 unbounded channel send；表达式入口已有 current checkpoint，不存在 pending、timer或合法 suspension point。 |
| `requestJsonToConnection` | `STILL_MISSING_IN_I6` | Skiff surface仍正确保持三参数；内部 request transport在request构造时冻结 root token/deadline，registry不知道 derived current scope。 |
| canonical in-process service unary/stream | `SATISFIED_BY_E4R` | `async_stream_cancel/current_scope.rs` 在调用/发布时读取 current scope、按 cancel→deadline→provider顺序等待并隔离 late result；但两个额外 service timeout candidate没有数据模型，见 §6。 |
| Actor create/get/replace/find/remove、spawn control | `STILL_MISSING_IN_I6` | Host `ActorClientContext`只持 request root cancellation，无current deadline；E4 wrapper不能在future Pending期间唤醒它。 |
| Actor method Host path | `STILL_MISSING_IN_I6` | eval从 retired legacy timeout context得到 `None`后固定 30s；Host只等root token、30s timer或response，current scope未进入wire或pending owner。 |
| `std.time.sleep` | `STILL_MISSING_IN_I6` | projection返回冻结 `TimeCapabilityContext`；10ms poll读的是root control。同步 `Date.now`/date helper只需既有checkpoint，已满足。 |
| file source与其它 request-local external source | `STILL_MISSING_IN_I6`（Host file/composite）；`SATISFIED_BY_E4R`（ordinary source consumer） | `createFromStream` 内部 file-source context仍冻结root control；普通 `for`/invocation source wait已由E4 current-scope owner覆盖。 |
| response/source stream wait与cleanup | `SATISFIED_BY_E4R`（ordinary/canonical service consumer）；`STILL_MISSING_IN_I6`（Host response sink、file composite/open） | E4已经拥有 consumer natural End与非End cleanup；I6不得复制 stream supervisor，只补 Host-created wait。 |
| request/root最终 timeout owner | `SATISFIED_BY_E4R` | HTTP/WebSocket connect outer select与inbound WebSocket JSON-RPC finalizer仍是唯一root owner；E4 internal carrier没有被 ordinary catch/wire导出。 |
| 通用 Host lifecycle metadata / M0 | `OBSOLETE_BY_D1_D2` | D1明确第一版不要求 cancel-safety、commit point、cleanup action或cleanup grace；不得恢复旧 M0。 |
| peer `$/cancelRequest` / `-32800` | `OBSOLETE_BY_D1_D2` | D2已删除peer cancel profile/broker路径；现存 `request.cancel` / `connection.request.cancel` 只能是internal best-effort stop hint。 |
| 真实 source→artifact→Router/consumer receipt | `I7_ONLY` | `.skiff` 编译、golden、真实Router wire、Agine/codex-relay、stable/live/chat smoke均不进入I6。 |

### 2.2 E4R 已拥有、I6 禁止重做的 owner

- `runtime/eval/src/eval_context/actual_pending.rs` 的 first-poll/Actor segment与前后checkpoint；
- `runtime/eval/src/eval_context/timeout.rs` 的local owner materialization和catch边界；
- concurrent scheduler与Actor continuation；
- `program_stream/current_scope.rs`、`program_invocation/current_scope.rs`；
- `assembly_execution/async_stream_cancel.rs` 及其 `current_scope.rs` 的 canonical service
  unary/stream/provider publication。

I6 consumer可以调用这些已有 owner，不能把它们复制到每个 Host operation，也不能修改
actual-Pending来重新定义E4语义。

## 3. Façade 与 invocation-time carrier

### 3.1 Façade 事实仍成立

生产调用链是：

```text
request::ExecutionControl(root ExecutionScope)
  -> host/eval_capability_adapter/factory.rs::execution_control
  -> RuntimeExecutionControl(concrete OwnedExecutionControl)
  -> capability-context ExecutionControl / OwnedExecutionControl
  -> ProgramExecutionContext.current execution
```

`runtime/host/src/eval_capability_adapter/execution.rs` 中：

- borrowed adapter没有实现 `execution_scope()` 或 `derive_scope(...)`；
- owned adapter也没有显式实现两者；其trait默认经 `borrow()` 转发，最终仍得到
  `ExecutionScopeAccessError::Unavailable`；
- `deadline()`只保留 `Instant`，`cancellation_token()`只返回request root token，无法重建
  `EffectiveDeadline::{source,site,nesting}`、全部ancestor/local signals或scope lifecycle。

最小修复是 borrowed/owned 两条路径都显式转发 concrete full scope与derive，并使用已有
`From<ExecutionScopeDeriveError> for ExecutionScopeAccessError` 保留 `Derive` variant。不得从
deadline/token猜scope，也不得把derive overflow降级成 `Unavailable`。

### 3.2 capability context 仍冻结 root

`ProgramExecutionContext::with_execution_control` 只替换 `execution`。`file_source_stream`、`time`、
`websocket`、`http_client`、`actor`、`spawn` 和 retired `outbound` 都在
`assembly_execution_context.rs::program_execution_context` 构造时保存root snapshot。

`RuntimeNativeCapabilityProjectionSource` 每次native invocation都会新建，但其 getter当前为：

```text
actor -> context.actor_context()
file -> context.file_context() + context.file_source_stream_context()
time -> context.time_context()
http_client -> context.http_client_context()
websocket -> context.websocket_context()
http_response_stream -> context.execution()       # 唯一调用时读取
```

所以仓库尚无“每次 invocation 读取一次current control，并由所有suspending capability共享”的统一
carrier；各 capability仍保存自己的snapshot。

### 3.3 最小共享 checkpoint

不需要新的公开语义或metadata。I6-A应只建立以下Rust内部合同：

1. façade完整转发已有 `OwnedExecutionControl` / `ExecutionScope`；
2. `RuntimeNativeCapabilityProjectionSource::new` / `new_supervised` 在native调用时精确读取一次
   `context.execution().owned()`，所有本次projection consumer共享该clone；
3. suspending consumer从该control取得 `ExecutionScope`，使用现有
   `ExecutionScope::acquire_lease()`；pending waiter等待全部current signals与absolute deadline，
   lower需要主动观察时接收 `lease.child_execution_scope().cancellation_signals()`；
4. lower成功后只由completion owner `complete()`；scope winner先固定本地结果并drop lower future，
   child cancellation只用于best-effort收束。late value/error不得进入finalize或caller heap；
5. 普通WebSocket send等 `Ready` operation只经过既有调用前/后checkpoint，不创建lease、timer或yield。

唯一共享API owner是 I6-A；HTTP/WebSocket/time/file/Actor分支不得各自发明第二种scope投影。Skiff
业务函数参数不变化，特别是 `requestJsonToConnection` 不增加第四参数。

projection source的 `new` / `new_supervised` 与两个实例化点都在
`runtime/eval/src/native_capability.rs`；production caller分别位于同文件的legacy wrapper、
`runtime/eval/src/eval_context/actual_pending.rs` 与 `runtime/eval/src/eval_context.rs`。
若内部trait需要机械增加carrier，fixture follow-through至少包括：

```text
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/tests/f445h_e4r_combined/capability_harness.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/actor_dispatch.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/host/src/eval_capability_adapter/{http,file_stream,websocket,actor}.rs
```

这些只是constructor/trait机械跟随，不把E4测试语义重新分配给I6。共享测试必须证明borrowed、owned、
owned.borrow和native projection都保留同一deadline owner、全部signals与derive错误；只测
`deadline()`或root token不构成receipt。

## 4. Current operation owner

| operation | 当前真实调用链 | current scope读取时点 / lower实际收到 | 当前结论与最小production owner |
| --- | --- | --- | --- |
| HTTP unary | native `dispatch/http.rs` → eval `RuntimeNativeHttpClientCapabilityContext` → Host adapter → `http_client_runtime::HttpEffectRequest` → `http_runtime::{request,transport}` | `HttpEffectContext`在request构造时把wall-clock remaining budget冻结为`deadline_ms`并保存root token；transport只取 `min(input.timeoutMs, frozen deadline_ms)` | `STILL_MISSING_IN_I6`。I6-A提供invocation control；I6-B让pending waiter/transport使用current absolute deadline与signals。 |
| HTTP stream / SSE open | 同上，进入 `open_body_stream...` / `open_sse...`，然后注册 `HttpBodyPullSource` / `HttpEventPullSource` | open阶段仍是root snapshot；source另有stream-local token | open为I6-B；handle创建后的ordinary `next`与非End cleanup已由E4满足，不复制。 |
| raw HTTP response emit | native `std.http.stream.emitResponse` → eval response context → capability-context `HttpResponseStreamCapabilityContext::send_response_event` → `StreamSink` | projection在调用时取current control，但sink只得到 `execution.cancellation_token()`（request root），没有deadline waiter | I6-D只补scope lease/signals；E4继续拥有actual-Pending和stream supervisor。 |
| WebSocket四send | native `dispatch/websocket.rs` → eval/shared websocket context → Host concrete `send_connection_frame` → unbounded router writer | invocation前表达式checkpoint读取current scope；lower是同步send，无wait数据 | 已满足。I6-C只加“无虚假suspension”的回归receipt，不改生产语义。 |
| WebSocket request | native三参数dispatch → eval `WebsocketRequestCapabilityApi` → Host `RuntimeWebsocketRequestCapabilityContext` → concrete context → `ConnectionRequestRegistry` | `RuntimeConnectionRequestParts`在request构造时保存root token/deadline；registry只收单token与`Option<Instant>` | I6-C改为current signals/absolute deadline（或等价child scope），保留三参数。 |
| canonical service unary | `assembly_execution::execute_service_call` → `prepare/execute_provider_unary` → `await_provider_unary` → in-process provider evaluator | call时从 `context.execution` 取full scope；`current_scope::wait`按signals/deadline包provider future | E4已满足caller/current scope与late隔离；只剩§6没有typed candidate。禁止改legacy relay。 |
| canonical service stream | `start_provider_stream` → provider task → `await_provider_stream_terminal` / publication wait → shared stream runtime | producer保存调用时owned control；provider、publication与item wait均重新取full scope | E4已满足；natural End/non-End cleanup也已覆盖。 |
| Actor get/create/replace/find/remove、spawn | native actor dispatch → eval `ActorClient` → Host adapter/concrete `ActorClient` → `OutboundRequestLease` / router writer | Actor context构造时保存root cancellation；没有current deadline | I6-D用shared carrier包control RPC wait并把child stop交给lease/drop owner。 |
| Actor method | `eval/actor_dispatch.rs` → prepared operation → shared Actor API → Host `invoke_actor_method` → `ActorMethodOutboundLease` | eval用legacy `effective_timeout_ms(None).unwrap_or(30_000)`；Host只收root token与该30s；wire同样写30s | I6-D取 `min(current effective deadline, 30s primitive)`，区分current owner与primitive owner。 |
| `std.time.sleep` | native `dispatch/time.rs::sleep_for_millis` → `NativeTimeCapability::poll_execution_budget` | 每10ms poll冻结的 `TimeCapabilityContext`；sleep duration是等待时长，不是额外deadline | I6-A/D按invocation current control重建；paused clock证明scope winner，不增加语言yield。 |
| file direct operations | native file dispatch → eval/shared file context → Host adapter → `FileRuntime` / DB/blob provider | file context完全没有execution carrier；E4只在future返回前后checkpoint | I6-D在Host file operation边界安装scope waiter；已发blob/DB effect可能unknown，不能伪装撤销。 |
| `createFromStream` | native composite → Host file context + file-source context → `StreamRuntime::next_with_cancellation` → `FileRuntime::create_from_chunks` | source context冻结root control；cleanup guard正确区分natural End，但pending wait看不到derived scope | I6-D用invocation carrier；保留E4 composite supervision和现有End规则。 |
| ordinary/source stream `next` | program stream / program invocation / canonical service stream current-scope modules | 每次wait读取current scope，额外root token iterator为空 | E4已满足，不进入I6 production写集。 |
| request/root terminal | HTTP/WebSocket connect Host outer select；inbound JSON-RPC `finalize_execution_terminal` | ingress owner持root cancellation/deadline并在接纳ready result前重查cancel→deadline | 已满足且保持唯一root telemetry owner；I6 ordinary operation不写request-wide deadline telemetry。 |

## 5. Operation deadline / internal stop 合同

### 5.1 现有唯一错误 owner

- `ProgramExecutionContext::checkpoint` 从current scope产生内部 `ScopeTerminalCarrier`；
- `eval_context/timeout.rs::materialize_owned_timeout` 只把自己拥有的local deadline物化为可catch的
  `TimeoutError`；inherited deadline和ancestor stop继续内部传播；
- primitive operation自己的更早timeout是该lane的ordinary `TimeoutError` owner；
- HTTP/WebSocket connect的outer Host race与inbound WebSocket JSON-RPC finalizer是request/root最终owner；
- ancestor internal stop没有业务payload、`CancelError`或catch projection。

consumer不能先把scope terminal折成generic capability error再猜owner。若lower先以cancel/timeout唤醒，
E4的post-await checkpoint必须在finalize/caller heap写入前恢复精确current terminal。

### 5.2 每个仍属I6的operation

| operation | 最早deadline | stop、local settlement与late隔离 | 副作用/cleanup owner |
| --- | --- | --- | --- |
| HTTP unary/open | current effective deadline（已含request/outer）与显式`HttpClientRequest.timeoutMs`中更早者；不存在字段时不得发明authoring值 | scope lease/pending waiter先决定terminal并drop request/open future；fake lower必须证明late response不进入native finalize | reqwest response/future与HTTP stream owner尽力关闭；已发HTTP write outcome可unknown，不宣称撤销 |
| HTTP body/SSE pull | handle创建时继承operation owner；每次consumer wait使用调用点current scope | E4 consumer guard在break/return/error/timeout/stop时触发stream cancel；late chunk/event被丢弃 | `HttpBodyPullSource` / `HttpEventPullSource`、stream-local token与response drop |
| response emit | current effective deadline，无额外primitive | scope lease先settle，sink late capacity wake不能继续写response | `StreamSink`与existing `StreamConsumerCleanup`; natural End与异常cleanup仍由E4 |
| WebSocket request | current effective deadline，无第四业务timeout | registry先CAS settle、删除pending、清timer/lease，再发可丢失internal stop hint；late `complete=false`且session fence不变 | `PendingConnectionRequest`；不等待hint ack，不发送peer cancellation |
| Actor control | current effective deadline，无现有primitive | scope winner drop/wake `OutboundRequestLease`，registry先本地移除；late response不能命中 | `OutboundRequestLease` best-effort `request.cancel`；命令已发送后outcome可unknown |
| Actor method | current effective deadline与现有30s primitive更早者 | `ActorMethodOutboundLease`先移除本地entry；internal actor cancel frame只是best-effort hint；late outcome不能命中 | Actor outbound registry/lease；不等待owner acknowledgement |
| sleep | current effective deadline；requested sleep duration只决定normal wake | current waiter或current control poll终止sleep，post-checkpoint保留owner | Tokio timer，drop即本地清理 |
| file direct/composite | current effective deadline；当前无file primitive timeout | local scope winner先drop provider future，caller finalize/heap不可接纳late结果；source cleanup非End只执行现有幂等request | `FileIngest`/`StagedFile`、DB/blob driver/session；commit/put开始后可能完成或unknown |

HTTP显式 primitive timeout与Actor 30s primitive若先于current scope，必须成为ordinary
`TimeoutError`；current/request/outer deadline则由scope owner决定local materialization或root结果。
现有HTTP reqwest timeout映射为 `ProviderUnavailable("request timeout")`，不能作为最终I6 receipt；
I6-B必须用RED锁定正确timeout owner。其它 Host operation没有可证明的通用lower physical cancel，
但D1只要求本地立即收束、late隔离和具体资源owner best-effort cleanup。

## 6. Service timeout 条款审计

### 6.1 逐层事实

| 层 | 当前typed事实 | 缺失 |
| --- | --- | --- |
| authoring | `package.yml` service dependency只声明alias/service/version；`service.yml.serviceCalls`只选择callable；`config.<profile>.yml timeout`只投影单个deployment override | dependency edge没有timeout；没有callee operation/default authoring shape |
| compile requirements | `ServiceRequirement { contract_requirement, service_binding_slot, used_operations }` | 无consumer dependency timeout |
| contract | `BoundaryOperationContract`只有parameters/return/stream/callback/effect guarantee | 无callee operation timeout，且设计明确不从provider body推断pending语义 |
| deployment | `DeploymentPolicy { timeout_ms, resources, activation, principal }` | 只有deployment-wide ingress/request override；没有按dependency edge或operation key字段 |
| assembly | `ResolvedServiceBinding` / `ServiceBindingTemplate`只有key、contract、provider、used operations；`ActivationTemplate`复制deployment policy | binding edge没有timeout candidate |
| activation | `ActivationServiceBinding`只有key、provider activation、contract、used operations；`ActivationOwnedBindings`保留global policy | canonical service resolve不返回dependency/callee constraint |
| eval seam | `RuntimeAssemblyServiceCallTarget`只有provider request、contract/schema、operation、executable addr | call target没有任何额外timeout数据 |
| runtime wait | E4 `current_scope::wait`只消费caller current scope | 无从聚合dependency/callee deadline |
| ingress Host | `assembly_wire::effective_request_deadline`把wire expiry与provider deployment `policy.timeoutMs`取min；WebSocket connect outer race也收紧同一policy | 这证明该字段当前是external ingress/request policy，不证明它是service callee operation timeout |
| legacy | `ServiceTimeoutConfig { default_ms, methods }`、legacy `ServiceDependencyConstraint`和outbound relay仍存在 | canonical assembly构造 `RuntimeActivation.timeout = Default` 且outbound为retired；不得把legacy字段搬回canonical路径 |

### 6.2 必答结论

1. 两者**没有**“数据模型完整、只差current-scope接线”的状态。
2. 现有 `DeploymentPolicy.timeout_ms` 被权威文档和Host代码明确用于deployment external ingress/request
   override；现有service dependency声明与operation contract都没有timeout。不能按字段名把它解释成
   dependency或callee timeout。
3. 除 `doc/reference/runtime.md` §8 的候选列表外，没有可唯一推出配置位置、粒度、持久化、缺省值和
   两者相互优先级的public/config语义。

因此完整service timeout条款必须返回 `DECISION_REQUIRED`。最小用户选项是：

1. **补齐typed owner**：先单独更新权威设计，明确consumer edge owner和callee
   operation/default owner、authoring位置、正整数/缺省规则、artifact/assembly持久化及min优先级；
   随后重发独立schema/compiler/loader checkpoint和I6 service consumer任务。
2. **第一版缩小条款**：权威文档明确service call当前只聚合caller request、outer timeout和已存在的
   primitive candidate；把dependency/callee candidate延后。E4 canonical service current-scope receipt
   可直接复用，I6无需service production节点。
3. **明确复用 `policy.timeoutMs`**：只有用户/设计明确它同时是callee default、说明是否按operation覆盖及
   service-call应用规则后才可实现；当前事实不能自行选择此解释。

在决策前不得新增artifact字段、把legacy `ServiceTimeoutConfig`接回assembly、或把global deployment
timeout偷偷当作两种candidate。

## 7. 已删除的旧要求

- 旧M0 Host lifecycle metadata前置已删除：第一版不增加通用cancel-safety、commit point、cleanup
  action或cleanup grace；具体resource owner负责normal terminal与异常best-effort收束。
- 没有公开request cancellation、`CancelError`、按request id取消或stop inspection API。
- 第一版不收发peer `$/cancelRequest`，没有 `-32800`。内部
  `request.cancel` / `connection.request.cancel` 名称可以保留为幂等、可丢失stop hint，但本地pending
  correctness不得依赖peer接收或ack。
- 普通WebSocket send是non-suspending；语言没有显式yield，不得人为制造suspension point。
- `requestJsonToConnection` Skiff surface严格保持 `(connectionId, method, value)` 三参数；scope是内部
  invocation carrier。
- legacy service relay、旧DTO、dual-read/dual-write/fallback均是禁止写集。

## 8. 最小 I6 DAG 与精确写集

```text
I6-A shared invocation-scope checkpoint
  ├─ I6-B HTTP current scope
  ├─ I6-C WebSocket request current scope
  └─ I6-D time / file / Actor / Host response-source wait

service-timeout user decision
  ├─ typed-owner design + new schema/compiler/loader checkpoint
  └─ first-version scope reduction (no service production delta)

I6-B + I6-C + I6-D + service-decision receipt
  -> I6-J hermetic combined integration probe
  -> independent I6 acceptance
  -> I7 handoff
```

当前 ready queue只有 I6-A；A冻结并验收后，B/C/D写集互斥，可以并行。service decision不阻塞
A/B/C/D，但阻塞I6-J最终合同与独立I6 acceptance。

### 8.1 I6-A shared invocation-scope checkpoint

- **直接父节点**：本 result、`P5-F445H-E4R5C-combined-reacceptance-result.md`、
  D1 result、D2 result。
- **production owner / 允许写集**：

```text
runtime/host/src/eval_capability_adapter/execution.rs
runtime/eval/src/native_capability.rs
runtime/eval/src/capabilities.rs
runtime/capability-context/src/{http,file,actor,stream}.rs
runtime/host/src/eval_capability_adapter/{http,file_stream,websocket,actor}.rs
```

  实际diff应是该集合子集。共享trait签名若不需要变化，就不得机械修改相应文件。
- **test/fixture owner / 允许写集**：

```text
runtime/host/src/eval_capability_adapter/execution.rs                 # inline tests
runtime/eval/src/program_execution/execution_scope_tests.rs
runtime/eval/src/program_execution/execution_scope_tests/evaluator_checkpoint.rs
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/tests/f445h_e4r_combined/{capability_harness,imports}.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/{actor_dispatch,file_create_from_stream}.rs
```

  后八项只允许constructor/trait机械跟随，不允许改E4断言或降低覆盖。
- **禁止写集**：E4 actual-Pending/timeout/concurrent/stream/service owner、native public/std
  signature、artifact/native metadata、request root boundary、router、legacy outbound、Cargo/lockfile。
- **第一处预期修改**：
  `RuntimeExecutionControl` / `RuntimeOwnedExecutionControl` 显式转发
  `execution_scope()` / `derive_scope(...)`；随后才把同一invocation control接入native projection。
- **test-first RED**：
  Host façade borrowed/owned/owned.borrow对derived scope返回 `Unavailable`；derived native projection
  中HTTP/time/file/WebSocket/Actor fake观察到root而不是child。
- **聚焦命令**：

```bash
cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --list
cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --list
cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --nocapture
```

  listing必须非零；测试覆盖inner-earlier、outer-earlier、equal保留outer owner、ancestor/local
  signals、derive error variant与lifecycle归零。
- **反向搜索**：

```bash
rg -n "context\\.(time_context|file_source_stream_context|http_client_context|websocket_context|actor_context)\\(\\)" runtime/eval/src/native_capability.rs
rg -n "fn (execution_scope|derive_scope)" runtime/host/src/eval_capability_adapter/execution.rs
```

  第一条不得再表示suspending consumer直接采用冻结snapshot；第二条必须同时覆盖borrowed/owned。
- **解除节点**：I6-B、I6-C、I6-D。
- **证据失效条件**：`ExecutionControl`/`ExecutionScope` API、native projection constructor、
  E4 actual-Pending边界或任何上述机械fixture在checkpoint后变化。

### 8.2 I6-B HTTP current scope

- **直接父节点**：I6-A result/commit。
- **production owner / 允许写集**：

```text
runtime/host/src/capability_context/effect_context.rs
runtime/host/src/host/http_client_runtime.rs
runtime/host/src/host/http_runtime/{call_context,request,stream,sse,transport}.rs
```

- **test owner / 允许写集**：

```text
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs             # new
runtime/host/src/host/http_runtime/tests/{request,stream,sse}.rs
runtime/host/src/host/http_client_runtime.rs                          # fake-driver inline tests if private seam
```

- **禁止写集**：真实TCP/network test作为current-scope receipt、E4 stream consumer、HTTP ingress
  root owner、proxy/egress policy、public std shape、router、Cargo/lockfile。
- **第一处预期修改**：
  `HttpEffectRequest` 停止把request-start relative `deadline_ms` / root token当作operation owner；
  `HttpCallContext`保留current absolute deadline与child signals到transport开始。
- **test-first RED**：paused clock + scripted fake transport进入Pending；derived child到期时旧future不醒，
  fake只看到root budget；primitive timeout旧路径返回ProviderUnavailable；scope winner后fake late
  response仍能尝试完成。
- **GREEN**：pending waiter取 `min(current, input.timeoutMs)`，ancestor stop同刻优先，primitive
  timeout为ordinary `TimeoutError`，current owner由post-checkpoint恢复；future/drop/stream cancel各自
  使用现有幂等owner收束且active counter归零，不等待cleanup acknowledgement，也不承诺cleanup恰好
  一次；late result不finalize。
- **聚焦命令**：

```bash
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
```

- **反向搜索**：

```bash
rg -n "CancellationSignals::from_tokens\\(\\[request\\.cancellation" runtime/host/src/host/http_client_runtime.rs
rg -n "frame_deadline_ms|deadline_ms" runtime/host/src/host/http_runtime runtime/host/src/host/http_client_runtime.rs
```

  第一条目标为0；第二条每个剩余命中必须分类为primitive/current-at-operation-start，而不是
  request-construction snapshot。
- **解除节点**：I6-J HTTP case。
- **证据失效条件**：I6-A carrier、HTTP native input shape、transport seam或stream runtime owner变化。

### 8.3 I6-C WebSocket request current scope

- **直接父节点**：I6-A result/commit、D2 result。
- **production owner / 允许写集**：

```text
runtime/capability-context/src/connection_request.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
```

- **test owner / 允许写集**：

```text
runtime/capability-context/src/connection_request_tests.rs
runtime/host/src/eval_capability_adapter/{websocket,factory}.rs       # inline tests
runtime/host/src/capability_context/websocket.rs                      # inline tests
```

- **禁止写集**：router/profile/broker、peer cancel、transport schema、Skiff function signature、
  business identity fan-out request、E4 actual-Pending、Cargo/lockfile。
- **第一处预期修改**：
  `RuntimeConnectionRequestParts` / registry install接current child scope的全部signals与absolute
  deadline，不再只保存root `CancellationToken`。
- **test-first RED**：root仍active而derived child到期/ancestor scope stop时pending不醒；wire
  deadline仍是root；late completion能否重开由现有negative assertion锁定。
- **GREEN**：三参数native不变；cancel→deadline→response priority；CAS先移除pending并把
  lease/timer归零，再尝试internal stop hint；late `complete=false`、wrong session/generation不命中；
  internal hint失败不改变本地terminal。
- **聚焦命令**：

```bash
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture
```

- **反向搜索**：

```bash
rg -n '\\$/cancelRequest|-32800|Request cancelled' runtime router
rg -n "requestJsonToConnection" std doc/reference/std-surface.md
rg -n "RuntimeConnectionRequestParts|registry\\.install" runtime/host/src runtime/capability-context/src
```

  第一条production/profile必须保持0；第二条确认Skiff三参数；第三条所有production install都必须
  分类为current scope或无request API fixture。
- **解除节点**：I6-J WebSocket case。
- **证据失效条件**：D2 profile/broker、connection registry settlement、I6-A carrier或
  WebSocket generation/session identity变化。

### 8.4 I6-D time / file / Actor / Host response-source wait

- **直接父节点**：I6-A result/commit、D1 result。
- **production owner / 允许写集**：

```text
runtime/native/src/dispatch/time.rs
runtime/capability-context/src/stream.rs
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/capability_context/store.rs
runtime/host/src/host/file_runtime.rs
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

- **test owner / 允许写集**：

```text
runtime/native/src/dispatch/time.rs
runtime/native/src/dispatch/file/tests.rs
runtime/capability-context/src/stream.rs
runtime/host/src/host/file_runtime/tests.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
```

- **禁止写集**：DB E4/O6 state machine、canonical service/ordinary stream owner、通用lifecycle
  metadata/grace、public cancel/error、Actor/router wire schema、legacy outbound、Cargo/lockfile。
- **第一处预期修改**：
  Host response sink使用invocation scope lease；file/Actor consumer复用同一carrier并各自保留resource
  owner，不新增全局cleanup supervisor。
- **test-first RED**：
  derived child下sleep仍poll root；response sink capacity wait不醒；file provider与Actor control/method
  pending仍只认root/30s；drop中的file temp staging与late Actor response需有明确negative probe。
- **GREEN**：
  paused sleep按current scope收束；response sink current deadline/ancestor stop不晚写；file direct与
  `createFromStream`先本地settle、non-End cleanup且temp staging不泄漏；Actor control/current deadline
  与method `min(current,30s)`生效，registry先移除、internal hint best-effort、late outcome不命中。
- **聚焦命令**：

```bash
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --nocapture
```

- **反向搜索**：

```bash
rg -n "parts\\.cancellation\\.wait_cancelled|context\\.cancellation_token\\(\\)\\.wait_cancelled" runtime/host/src/{eval_capability_adapter,capability_context}
rg -n "send_with_cancellation\\(.*cancellation_token" runtime/capability-context/src/stream.rs
rg -n "FileIngest|StagedFile" runtime/host/src/host/file_runtime.rs
rg -n "30_000" runtime/eval/src/actor_dispatch.rs runtime/host/src/eval_capability_adapter/actor.rs
```

  root-only waits目标为0；30s可保留为primitive，但所有callsite必须显示与current deadline取min而非替代。
- **解除节点**：I6-J time/file/Actor/response case。
- **证据失效条件**：D1 stop/cleanup合同、I6-A carrier、file staging、Actor registry/wire或stream sink
  owner变化。

### 8.5 Service decision gate（当前不是可执行leaf）

该节点没有允许写集，也不能分派实现。用户选择§6.2后：

- 选择typed owner：先单独更新权威设计并重新preflight schema/compiler/loader写集；当前result不能替它
  发明精确文件或字段。
- 选择第一版缩小：以单独设计提交删除/延期两个candidate，复用E4 service current-scope receipt；
  I6不新增service production节点。
- 选择复用deployment policy：先在权威设计中冻结语义，再按最新tree重发任务。

任何一个决策receipt都会解除I6-J的service合同输入；在此之前不能把service case写成“跳过也绿色”。

### 8.6 I6-J combined integration probe

- **直接父节点**：I6-B、I6-C、I6-D result/commit，加service decision receipt。
- **production owner**：无。
- **test owner / 唯一允许写集**：

```text
runtime/host/tests/f445h_i6_current_scope.rs
```

- **禁止写集**：所有production、现有E4 combined binary、network/server、Mongo、stable/live、
  Cargo/lockfile。
- **第一处预期修改**：先在三个consumer merge之前写combined test并记录至少一个真实RED，再冻结同一
  integration commit运行。
- **combined cases**：
  1. 一个derived child依次驱动fake HTTP pending、WebSocket pending、response sink；
  2. 一个ancestor stop同时到达，三个owner都先本地settle，late value/error/response均不能finalize；
  3. paused time、file staging与Actor pending归零；
  4. canonical service current scope复用E4 owner，并按用户决策加入typed candidate或明确不断言它。
- **命令**：

```bash
cargo test -p skiff-runtime-host --test f445h_i6_current_scope -- --list
cargo test -p skiff-runtime-host --test f445h_i6_current_scope -- --nocapture
```

  listing与execution数量必须精确相同且非零。
- **反向搜索**：

```bash
rg -n "F445H I6|f445h_i6" runtime/host/tests runtime/{eval,capability-context,native}/src
rg -n '\\$/cancelRequest|-32800|CancelError' runtime router std
rg -n "service_dispatch|outbound_service" runtime/host/tests/f445h_i6_current_scope.rs
```

  combined不得依赖legacy relay或peer cancel。
- **解除节点**：independent I6 acceptance。
- **证据失效条件**：combined冻结后任一production/test/fixture commit、service decision或parent
  evidence变化。

### 8.7 Independent I6 acceptance

- **直接父节点**：I6-J result/commit。
- **production/tests owner**：只读；只新增独立acceptance result。
- **唯一完整crate gate owner**：在同一最终tree上各执行一次
  `skiff-runtime-capability-context`、`skiff-runtime-native`、`skiff-runtime-eval`、
  `skiff-runtime-host` 的 `--locked --no-fail-fast`，再执行locked check、fmt、diff与本result冻结的
  反向搜索。不得机械重跑E4旧tree的 `411/411` 作为I6证据，也不得运行live/stable/network/Mongo。
- **解除节点**：I7。
- **证据失效条件**：acceptance开始后任何候选写入。

## 9. Test-first 矩阵与 combined probe

所有timer使用 paused Tokio/scripted monotonic clock；所有lower使用barrier、oneshot、drop counter或
fake driver。零test listing不算成功。

| receipt | 首个RED | GREEN必须证明 | owner |
| --- | --- | --- | --- |
| A façade/projection | derived scope经Host adapter返回Unavailable；native fake看root | borrowed/owned/owned.borrow同一scope；每次native invocation只读一次current carrier；signals/owner/derive error无损 | I6-A |
| HTTP | child到期不醒、lower只见root；primitive timeout为ProviderUnavailable | current/primitive最早、owner正确、late response不finalize、stream lifecycle counter归零且不等待cleanup ack | I6-B |
| WebSocket | child到期pending仍存活、wire/root deadline过宽 | 三参数、CAS先settle、timer/lease/pending归零、late/session fence、hint失败不影响terminal | I6-C |
| time/file/response | sleep/root poll、sink/file pending不醒 | current deadline/stop、natural End与非End区分、staging与late heap隔离 | I6-D |
| Actor | control只认root、method固定30s | `min(current,30s)`、local registry先移除、late outcome不命中、hint无需ack | I6-D |
| service | current scope已绿；dependency/callee数据不存在 | 只按用户决策增加typed receipt，或明确复用E4且删除两项断言 | decision gate + I6-J |
| combined | 至少一个consumer在parent merge前真实失败 | 同一child/ancestor stop跨三个Host consumer一致收束，所有counter归零 | I6-J |

建议独立acceptance命令：

```bash
cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
cargo test -p skiff-runtime-native --locked --no-fail-fast
cargo test -p skiff-runtime-eval --locked --no-fail-fast
cargo test -p skiff-runtime-host --locked --no-fail-fast
cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

若用户选择typed service owner，其新增artifact/compiler/loader crate gate由新checkpoint冻结，不能由本
result提前猜测。

## 10. I7 handoff

I6只交付hermetic Host/runtime receipt、精确commit/tree、scope owner与零泄漏counter。以下严格属于I7：

- 真实 `.skiff` source中的nested `timeout(...)` + HTTP/WebSocket/file/Actor/service调用；
- compiler output、File IR/artifact/golden、identity/schema receipt；
- Router真实wire、跨进程internal stop hint、connection generation与deployment rollout；
- Agine/codex-relay consumer、legacy DTO负搜索；
- stable instance、live service、MongoDB、chat smoke、浏览器验证。

I7不得把peer cancellation重新加入wire，也不得用live成功掩盖I6 hermetic late-settlement失败。

## 11. 用户决策与 ready queue

当前需要用户决定§6.2的service timeout owner。除此之外没有设计问题，也不需要恢复旧M0。

```text
READY NOW
  I6-A shared invocation-scope checkpoint

READY AFTER I6-A
  I6-B HTTP
  I6-C WebSocket request
  I6-D time/file/Actor/Host response-source wait

BLOCKED ON USER DECISION
  service timeout contract receipt
  I6-J final combined probe
  independent I6 acceptance
```

建议决策是“先补齐typed owner并单独做schema/compiler/loader checkpoint”；若第一版不准备承担两种
timeout，则选择“缩小权威条款”也能让Host/native ready queue继续。无论选择哪项，都必须先落权威设计，
不能由I6 implementation自行解释 `policy.timeoutMs`。

## 12. 交付边界

本result是唯一tracked写入。它没有修改production、tests、fixture、父文档、权威设计、Cargo manifest或
lockfile；没有执行动态gate，也没有派子 Agent、merge、rebase或push。result提交后最终交付消息记录
commit，并再次确认worktree clean。
