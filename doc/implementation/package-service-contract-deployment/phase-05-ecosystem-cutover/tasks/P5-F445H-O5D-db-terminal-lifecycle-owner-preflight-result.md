# P5-F445H-O5D DB terminal lifecycle owner preflight result

状态：`DECISION_REQUIRED`。

本节点没有修改 production 或 tests。审计已经唯一冻结 request/store owner、transaction/lease
resource owner、preserving-first-poll handoff、request teardown seam、API 草图和条件实现 DAG。
唯一剩余的用户可观察选择是：

> transaction body 已正常完成、commit intent 已被选择，而且同一个 commit provider future
> 已经首次真实返回 `Pending`；此时 request/lane 被取消，持久化终态应继续 commit，还是丢弃
> commit 并切换为 abort？

现有两个权威文本不能唯一回答：

- O6 task 要求 begin 成功后的 cancel/drop 走 abort；
- `doc/reference/runtime.md:185-193` 则规定 host operation 在 commit point 后取消时，外部副作用
  可能已经发生，语言层只丢弃 late result，不能把 cancel 描述成撤销。

两种选择会让后续 request 观察到不同 DB 内容，因此不能当作内部实现细节。本 result 推荐：

**选择 `COMMIT_INTENT_WINS`：`DbTransactionGuard::commit(self)` 原子选择 commit 后，同一个
commit future 必须继续到 provider terminal；取消只终止 evaluator waiter并丢弃 late result。
只有 commit 尚未选择时，guard drop/cancel 才创建唯一 abort。**

推荐理由是：首次 `Pending` 不提供 Mongo commit 是否已经执行的证据；drop commit 后调用 abort
无法证明 rollback，也可能把已提交结果伪装成已 abort。若选择 `ABORT_UNTIL_ACK`，必须先增加
provider-level 可取消 commit / outcome receipt / transaction resolution 合同，范围将超过本
DAG，并应转为 `TASK_SCOPE_EXPANDED`。

用户确认推荐项并把 O6 的 blanket “cancel/drop 均 abort”改为“commit selection 前 abort，
selection 后继续同一 commit”后，可直接执行 §9 的 DAG，无需重新做 owner 审计。

## 1. 审计输入与边界

| 项 | 值 |
| --- | --- |
| 直接父结果 | `P5-F445H-O6-evaluator-db-state-machines-result.md` |
| integration 起点 | `a5920a32` |
| O5C hermetic baseline | `98add404` |
| task / worktree HEAD | `732bcfaa` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5d-lifecycle-preflight` |
| branch | `codex/p5-f445h-o5d-lifecycle-preflight` |
| production / tests 修改 | 无 |
| Cargo / stable / live / network / MongoDB | 均未运行 |

审计只读取 capability-context、service-db、eval、request/host 的真实 owner 路径，DB/reference
语义，以及任务点名的既有 cleanup 模式。没有从更高层设计增加 DB 语言、wire、Actor、timeout
或错误类型。

## 2. 真实 owner 图

### 2.1 Source、request context、store

```text
ActiveAssemblyContextSet（assembly generation owner）
  └─ DbCapabilitySource
       └─ Arc<dyn DbCapabilityFactory>
            └─ context_for_request(owner, request_id)
                 └─ DbCapabilityContext
                      └─ Arc<dyn DbCapabilityContextApi>
                           └─ ServiceDbCapabilityHandle
                                ├─ Option<ServiceDbCapabilityFactory>
                                │    └─ Arc<ServiceDbRuntime>
                                └─ Arc<tokio::sync::Mutex<DbRequestState>>
                                     ├─ Option<DbTransactionState>
                                     │    └─ mongodb::ClientSession
                                     ├─ Vec<DbLeaseHold>
                                     ├─ lease_lost
                                     └─ owner / request_id

DbCapabilityContext::require_store(...)
  └─ DbCapabilityStore
       └─ Arc<dyn DbCapabilityStoreApi>
            └─ ServiceDbCapabilityStore
                 └─ ServiceDbStore
                      ├─ Arc<ServiceDbRuntime>
                      └─ 同一个 Arc<Mutex<DbRequestState>>
```

精确证据：

- `DbCapabilityContext` 是 `Option<Arc<dyn DbCapabilityContextApi>>`，clone 只增加同一 handle 的
  strong count；`require_store` 转发给 inner：
  `runtime/capability-context/src/db.rs:369-419`。
- `DbCapabilitySource` 持 assembly/generation 级 factory，并按 owner/request id 创建 request
  context：`runtime/capability-context/src/db.rs:422-466`。
- `DbCapabilityStore` 是 `Arc<dyn DbCapabilityStoreApi>`：
  `runtime/capability-context/src/db.rs:891-912`。
- concrete handle 持 `ServiceDbCapabilityFactory` 和唯一 request-state `Arc<Mutex<_>>`；
  每次 `require_store` 只克隆这两个 owner：
  `runtime/service-db/src/capability.rs:57-109`。
- concrete store 再持 `Arc<ServiceDbRuntime>` 与同一 request-state：
  `runtime/service-db/src/store.rs:18-43`。
- request state 当前直接拥有 session、lease holds 与 lease-lost：
  `runtime/service-db/src/lib.rs:81-114`。

`FileCapabilityContext` 也 clone 同一个 `DbCapabilityContext`
（`runtime/host/src/capability_context/store.rs:76-108`），file create/read/delete 再通过
`DbCapabilityStoreApi` 访问 file-record DB operations
（`runtime/host/src/host/file_runtime.rs:46-55,83-121`）。因此 request DB context 不是只被
`program_db` 持有；新的 lifecycle core 必须由 context 共享，而不能放在某一个 evaluator
局部变量中。

### 2.2 全部 implementor

repository 中 `DbCapabilityStoreApi` implementor 精确只有三个：

| kind | implementor | 证据 |
| --- | --- | --- |
| production | `ServiceDbCapabilityStore` | `runtime/service-db/src/capability.rs:139` |
| test | `PreparedFakeStore` | `runtime/capability-context/src/db/prepared_runtime_tests/fake_store/prepared.rs:39` |
| test | `DefaultPreparedStore` | 同文件 `:159` |

相关 request/context factory implementor也已全部审计：

| kind | implementor |
| --- | --- |
| production factory | `ServiceDbCapabilityFactory`，`runtime/service-db/src/capability.rs:45-49` |
| production context | `ServiceDbCapabilityHandle`，同文件 `:101-109` |
| host test factory | `TestDbCapabilityFactory`，`runtime/host/src/host/router_session/tests.rs:80` |
| host test factory/context | `PinnedRouteDbFactory` / `PinnedRouteDbContext`，`runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs:230,246` |

不存在隐藏的第二个 production store implementor，也不存在可以接管 terminal future 的 provider
registry。§7冻结的seam由`DbCapabilitySource`给factory产出的context附加lifecycle，不改变
`DbCapabilityFactory`签名；这些factory fixture本身无需各自实现supervisor，但其request构造
测试必须迁到新的source bundle，证明cleanup owner没有被丢弃。

### 2.3 Transaction 各阶段的真实 owner

| 阶段 | 当前 session owner | 证据 / 后果 |
| --- | --- | --- |
| begin 尚未完成 | `begin_transaction` future 的局部 `ClientSession`（创建后） | client/session/start transaction 在 `runtime/service-db/src/store.rs:61-73` |
| begin 成功 | `DbRequestState.transaction` | 同文件 `:74-81` |
| body raw DB wait | `DbRequestState.transaction.session`，同时 request-state mutex guard 跨 await | raw read 示例 `:132-149` |
| body O5R2 prepared wait | 同一个 request state/session；owned wait 持 `ServiceDbStore` | `runtime/service-db/src/prepared_runtime/store.rs:153-218` |
| body O5R2 update/replace | 同一个 session并克隆当前 lease guards | 同文件 `:221-278` |
| commit 一开始 | `commit_transaction` 先从 state `take()`，局部 future 取得唯一 session | `runtime/service-db/src/store.rs:84-102` |
| commit lease fence / provider commit | 同一个可被 caller drop 的 future 局部 | 同文件 `:104-119` |
| abort 一开始 | `abort_transaction` 先从 state `take()`，局部 future 取得 session | 同文件 `:122-129` |

commit/abort future 一旦在 `take()` 后被 drop，session 不再可由 request state 找到；重新调用
abort/commit只会得到 missing/no-op。现有 `DbCapabilityStoreApi` 的三条 transaction method 都是
借 `&self` 的 caller-owned future，且 wrapper直接 await：
`runtime/capability-context/src/db.rs:645-648,914-923`。没有 guard、receipt 或 handoff owner。

raw DB operation 本身不需要迁到新的 transaction owner中。正确最小改法是让
`DbRequestState.transaction` 在 body 期间继续拥有 active session；raw/O5R2 wait保持借
`&mut session`跨 await。body future被取消时，该 wait先 drop并释放 state lock，然后唯一
transaction guard才选择 abort。只有 terminal selection CAS 成功后，session移入**同一个 owned
terminal future**；在 provider terminal 前始终由 waiter或结构化 supervisor之一可达。

### 2.4 Lease 各阶段的真实 owner

| 阶段 | 当前 owner | 证据 / 后果 |
| --- | --- | --- |
| claim provider wait | `claim_lease` caller future | `runtime/service-db/src/store.rs:598-633` |
| provider claim 成功、state 尚未更新 | 同一个 caller future 局部 `DbLeaseHandle` | 同文件 `:623-637` |
| claim 返回成功 | `DbRequestState.leases` 持 `DbLeaseHold`；evaluator局部 handle也持 capability hold | `:634-639` 与 `runtime/capability-context/src/db.rs:1253-1294` |
| body renew | 裸 `tokio::spawn` task 持 store + hold；`JoinHandle`只在 evaluator局部 | `runtime/eval/src/program_db.rs:351-366` |
| normal body terminal | evaluator只 `abort()` renew，不 join | 同文件 `:368-381` |
| release provider wait | `release_lease` caller future；request state仍保留 hold | `runtime/service-db/src/store.rs:656-657` |
| provider release成功、state lock/update尚未完成 | 仍是同一个 caller future；重建会重放 provider release | 同文件 `:657-660` |
| release error | request state继续保留 hold | `?` 在 retain-remove之前返回，仍是 `:656-660` |

`DbCapabilityLeaseHandle` 只是 cloneable `{ hold, value, ttl_ms }`，没有 drop terminal、
renew owner或 receipt。当前 evaluator outer future若在 body期间 drop，`JoinHandle` drop会 detach
renew task，且没有 release路径。

新的 request state应把 lease entry从裸 `Vec<DbLeaseHold>`提升为至少：

```text
DbLeaseEntry {
  hold,
  phase: Active | RenewInFlight | ReleaseInFlight | Released | ReleaseFailed,
}
```

active/renew阶段仍向 update/replace/delete提供同一 fence hold；release成功才删除；
release error记录 `ReleaseFailed` 且不得重放 provider side effect。为保持当前 fail-closed
行为，`ReleaseFailed` 的 hold继续参与 request 内后继 write fence，不能悄悄变成无 guard write。

### 2.5 一条真实 request 构造 → eval → completion 路径

本结论不是按类型名推断；HTTP runtime assembly真实路径如下：

1. assembly admission为每个 activation构造并保存在 generation context中的
   `DbCapabilitySource`：
   `runtime/host/src/loader/active_assembly_context.rs:31-32,99-152,224-225`。
2. Host在 request admission后构造 `RuntimeAssemblyExecutionContext`；它当前只保存
   `db_source`：`runtime/host/src/eval_capability_adapter/assembly_execution_context.rs:38-58,133-153`。
3. request execution内部调用 `program_execution_context` 才执行
   `db_source.context_for_request(activation_id, request_id)`，并把 clone同时交给 file context：
   同文件 `:156-173`。
4. `ProgramExecutionContext`直接拥有 `DbCapabilityContext`，其 clone与
   `OwnedProgramExecutionContext::capture`都继续 clone同一个 context：
   `runtime/eval/src/program_execution.rs:58-100,102-128,389-438`。
   `OwnedProgramExecutionContext`不是只在栈内短暂存在：stream producer把它放进独立
   `tokio::spawn`，且不保存task handle
   （`runtime/eval/src/program_stream.rs:151-153,363-365,923-947`）。因此root execution drop
   后仍可能有DB context clone存活，cleanup owner必须等待context lifecycle permit归零，不能
   按词法scope推断所有producer已经析构。
5. `program_db`从 context clone store，transaction/claim直接 await borrowed future：
   `runtime/eval/src/program_db.rs:87-133,137-177,206-294,316-400`。
6. Host在 request task中 pin整个 execution，并以 biased `tokio::select!`竞争 root cancel、
   deadline与 execution；cancel/deadline获胜后不再 poll execution：
   `runtime/host/src/host/request_entry/assembly.rs:298-340`。
7. 当前 pinned execution仍存活到 `finish_http_gateway_request(...).await`之后才随 task scope
   drop；Host先让 `RequestSupervisor`认领 success/error/cancel terminal：
   同文件 `:341-413`。
8. `RequestSupervisor::claim_completion`只从 active map删除 request；它不持 capability context、
   cleanup future或 join receipt：
   `runtime/host/src/host/request_supervisor.rs:295-320`。
9. 包围该流程的顶层request `tokio::spawn`没有保存`JoinHandle`
   （`runtime/host/src/host/request_entry/assembly.rs:298-353`）；一旦第8步移除active entry，
   `active_count`也无法再证明post-terminal cleanup仍存活。
10. request crate的 `RuntimeHttpGatewayRequestLifecycle::drop`只同步
   `request_activation.cancel/end_request`：
   `runtime/request/src/http_gateway_execution.rs:289-311`。
11. 最后一个 context/store Arc drop后，`DbRequestState`及其中未收束 session/holds普通析构；
    repository没有 `DbCapabilityContext` / `ServiceDbCapabilityHandle` Drop或可 await cleanup。

另外两个真实构造点也没有 cleanup checkpoint：

- WebSocket connect / JSON-RPC沿同一个 `RuntimeAssemblyExecutionContext`；
- Actor owner invocation在
  `runtime/host/src/eval_capability_adapter/actor_method_adapter.rs:166-223`创建 context，并在
  `runtime/host/src/host/actor_owner_execution.rs:234-310`执行后直接返回；顶层invoke同样drop
  spawn handle（`:113-159`），`ActorOwnerInvocationRegistry::finish`直接remove entry，
  `cancel_session`更会clear整个map
  （`runtime/host/src/host/actor_owner_invocations.rs:79-100`）。

因此当前 request/eval/runtime 层均没有可 await 的 DB cleanup/join点。后继必须把 request DB
context提前到 execution future之外构造，并让 outer Host request task独占 cleanup join handle；
不能只给 `ProgramExecutionContext`加字段，因为它正是会被取消/drop的 waiter owner。

## 3. 既有 structured cleanup 模式的可复用边界

| 既有模式 | 可复用 | 不能直接复用 |
| --- | --- | --- |
| request activation lifecycle | `end_request/cancel`的幂等 seal、request generation identity；`runtime/request/src/http_gateway_execution.rs:289-311` | Drop只做同步 capability revoke，不拥有 async future、不等待 ack |
| `StreamConsumerCleanup` / `SupervisedStreamConsumptionLease` | child drop只登记 cleanup obligation、outer CAS唯一 finalization；`runtime/capability-context/src/stream_cleanup.rs:113-188,191-275` | concrete stream cancel是同步、幂等 registry terminal；不能继续一个已真实Pending的 provider future |
| `OwnedProgramExecutionContext` stream producer | context clone会把同一DB lifecycle permit带进逃逸task，permit归零可作为request cleanup join条件；`runtime/eval/src/program_stream.rs:923-947` | producer spawn handle未保存，不能把root evaluator scope结束当作所有capability clone已drop |
| `OutboundRequestLease` | registry entry、terminal CAS、receipt/notify与 Drop fail-closed；`runtime/capability-context/src/outbound_response.rs:104-155,234-285` | Drop发送 best-effort cancel并删除pending，不承诺原 external side effect完成 |
| prepared provider unary | owned caller-heap-free wait、late result不写 caller heap；`runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary.rs:88-183` | `ProviderUnaryRequestOwner::drop`取消并**drop原provider future**（`:58-85`）；DB commit/release不能这样做 |
| Actor execution lease | RAII唯一 token与drop释放 scheduler；`runtime/eval/src/actor_instance.rs:183-214` | release完全同步，没有 provider ack |
| Actor `await_if_pending` | 必须原样复用“直接 first poll，只有真实Pending才suspend”；`runtime/eval/src/actor_executor/actor_concurrent_continuation.rs:275-293` | 它只有 waiter owner；outer future drop后不会继续 future |
| O5R2 prepared DB wait/finalizer | `'static + Send` owned wait与 late synchronous finalizer形状；`runtime/capability-context/src/db/prepared_runtime.rs:7-70` | 当前 Pending drop直接销毁 wait，没有 supervisor handoff |

结论：复用 `owned wait + CAS receipt + child-drop登记/outer-owner执行 + E3 direct first poll` 四个
结构，但不能复用任何“drop原future后发送cancel”实现。

## 4. Cancellation / terminal 真值表

表中：

- `可见 completion 等 ack=是`表示仍在运行的 evaluator正常/错误路径必须等 provider
  terminal ack后才返回；
- `可见 completion 等 ack=否（后台）`表示 cancel/deadline的可见 terminal按
  `doc/reference/runtime.md:185`立即确定，同一个 operation由 request supervisor继续并记录
  receipt；late结果不得进入 heap/env；
- Actor栏描述把该 wait交给 `await_if_pending`时的**第一次poll**。

无论可见 completion 是否等待，outer Host cleanup owner都不能提前销毁：它在可见 terminal
确定后继续持有并 join 已登记 cleanup。表中的“否”不是 drop cleanup，而是区分语言可见结果与
物理 owner 收束。

### 4.1 Transaction

| 阶段 | 唯一 terminal/resource owner | evaluator可见结果 | waiter drop后的动作 | 可重建 | 可见 completion 等 ack | Actor first poll |
| --- | --- | --- | --- | --- | --- | --- |
| begin尚未启动（owned wait未poll） | prepared begin waiter | cancel/drop；没有transaction | drop未poll future；不得后台启动begin | 否 | 否，无资源 | 未poll |
| begin first-Ready error | begin waiter直到receipt Ready | 原provider error | 无abort | 否 | 已ack | `Ready(Err)` |
| begin first-Ready success | result中的唯一 `DbTransactionGuard` | 继续body | 后续guard drop才选abort | 否 | 已ack | `Ready(Ok)` |
| begin首次真实Pending | 同一个 pinned begin future；driver为waiter | 正常则等待；cancel则取消terminal | CAS把**同一个future**移交request supervisor；若late成功，产出的guard立即drop并创建唯一abort；late error只ack | 否 | cancel时否（后台） | `Pending` |
| begin成功、commit尚未选择 | `DbTransactionGuard` + active session仍在request state | body结果/error/cancel | normal等待后选commit；error/cancel/drop选唯一abort | terminal只选一次 | error路径是；cancel否（后台） | body自己的wait决定 |
| body运行 | guard；active session在request state | body normal/error | outer drop先drop当前body wait/state lock，再由guard handoff abort | 否 | cancel否（后台） | 每个嵌套op自己的Ready/Pending |
| body error / illegal flow / result error | guard原子选择abort | 保留原body/flow error | 同一abort future若Pending则handoff | 否 | 是；abort error被抑制 | abort首poll真实值 |
| body cancel/drop（commit未选） | guard Drop → request supervisor中的唯一abort | 内部cancel terminal，无catch payload | supervisor创建并驱动一次abort | abort只创建一次 | 否（后台） | N/A，evaluator已结束 |
| commit尚未选择时cancel | guard | cancel | 与body cancel相同，选abort | 否 | 否（后台） | N/A |
| commit已选择、future尚未poll | selected commit waiter | 正常则首次直poll；cancel立即terminal | handoff未poll的**同一个**future，由supervisor执行首次poll；不得改选abort | 否 | cancel否（后台） | waiter存活时看到真实Ready/Pending |
| commit first-Ready success | commit operation/receipt | body value/success | 无abort | 否 | 已ack | `Ready(Ok)` |
| commit first-Ready lease-fence/commit error | **同一个 composite commit terminal**；其内部按现状执行一次fallback abort | 原commit/fence error；abort error继续被抑制 | composite已terminal，不再调第二次abort | 否 | 已ack | `Ready(Err)` |
| commit首次真实Pending后cancel | **用户决策点**；推荐同一个commit future转supervisor | cancel立即获胜；late commit/error不可见 | 推荐：继续同一commit；不切abort | 否 | 否（后台） | 首poll已是`Pending` |
| commit provider已执行、ack未返回 | 只能是同一个commit future | waiter仍在则等result；cancelled waiter无late可见结果 | 继续原future，绝不重放commit/abort | 否 | waiter活着是；cancel否（后台） | 首poll必为`Pending` |
| commit error（首次Pending后） | composite commit继续内部fallback abort到terminal | waiter活着看到原commit error | drop时整个composite移交；不能新建abort | 否 | waiter活着是；cancel否 | `Pending` |
| abort已选择、future尚未poll | selected abort waiter或supervisor job | 暂存原body/flow error；cancel仍是cancel | handoff并首次poll一次；不得回退到active或另建abort | 否 | 非cancel路径是；cancel否 | waiter存活时看到真实Ready/Pending |
| abort first-Ready | abort operation/receipt | 原body error/flow error；显式cancel仍是cancel | terminal完成 | 否 | 非cancel路径是 | `Ready(())` |
| abort Pending | 同一个abort future | 原错误暂存，不被abort结果替换 | exact future handoff | 否 | 非cancel路径是；cancel否 | `Pending` |
| abort provider error | 同一个abort receipt记录 suppressed provider error | 仍返回原body/flow error；无新的public错误 | ack后结束，不重试 | 否 | 非cancel路径仍等该ack | Ready或Pending取决于真实首poll |
| abort waiter drop | request supervisor | cancel | exact future继续；late error只进receipt | 否 | 否（后台） | N/A |

`commit_transaction`当前在 lease fence或Mongo commit error后内部 await abort并返回原error
（`runtime/service-db/src/store.rs:104-118`）。新接口必须把它保留为一个 composite terminal；
O6不得再在 commit error后额外调用第二条 public abort。

### 4.2 Lease

| 阶段 | 唯一 terminal/resource owner | evaluator可见结果 | waiter drop后的动作 | 可重建 | 可见 completion 等 ack | Actor first poll |
| --- | --- | --- | --- | --- | --- | --- |
| claim尚未启动 | prepared claim waiter | cancel/drop；无hold | drop未poll future，不release | 否 | 否 | 未poll |
| claim first-Ready `None` | claim receipt完成 | `false` | 无renew/release | 否 | 已ack | `Ready(Ok(None))` |
| claim first-Ready error | claim receipt完成 | provider error | 无release | 否 | 已ack | `Ready(Err)` |
| claim first-Ready success | 唯一 `DbLeaseGuard`；hold也已登记request state | binding/body继续 | guard drop负责release | 否 | 已ack | `Ready(Ok(Some))` |
| claim首次真实Pending | 同一个claim future | 正常等待；cancel立即terminal | handoff exact future；late `None/error`只ack；late success产生guard后立即drop并release | 否 | cancel否（后台） | `Pending` |
| claim provider成功、state登记尚未完成 | 同一个claim future | 尚不可见成功 | exact future继续完成登记；再按waiter是否存在交guard或release | 否 | cancel否（后台） | 首poll已`Pending` |
| claim provider成功、local登记失败 | 同一个claim composite仍拥有hold | 活waiter最终看到原local error | composite先release一次再ack error；cancelled waiter只留receipt | 否 | waiter活着是；cancel否 | Ready/Pending依fallback release |
| binding decode/import error | `DbLeaseGuard`，renew尚未启动 | decode/import error | guard选择唯一release | 否 | 是；cancel时后台 | release真实首poll |
| renew idle | `DbLeaseScope`注册在request supervisor；尚无renew future | body继续 | stop标记后直接进入release | 每个周期最多创建一次新renew；同一周期不重建 | terminal路径按body种类 | N/A（后台owner） |
| renew awaiting | supervisor持同一个owned renew future | body继续；若body结束先等renew settle | 不drop/rebuild；stop只禁止下一轮，当前renew完成后才release | 否 | cancel否（后台） | N/A |
| renew result `true` | scope更新为idle并安排下一周期 | body继续 | terminal已选则不再创建下一轮，转release | 新一周期是新operation，不是重试 | N/A | N/A |
| renew result `false` / error | scope原子标lease-lost并设置request cancel flag | body收到结构化cancel；最终claim返回lease-lost | 停止下一轮，join当前结果，release一次 | 否 | visible cancel不等；cleanup后台 | N/A |
| body normal | `DbLeaseScope` | release成功后`true` | stop/join renew → lease-lost check → release | 否 | 是 | release wait真实首poll |
| body error / illegal flow | scope暂存原body/flow error | 优先级保持：lease-lost > release error > body/flow error | stop/join → check lost → exact release | 否 | 是 | release wait真实首poll |
| body cancel/drop | scope Drop把stop/join/release obligation交supervisor | cancel，不可catch | 等当前renew exact future结束；禁止late renew；release一次 | 否 | 否（后台） | N/A |
| release尚未启动 | scope/guard，request state entry仍可达 | 暂无terminal结果 | 只有renew已join且terminal CAS成功者可创建release | 只创建一次 | 依body种类 | 尚未poll |
| release已选择、future尚未poll | selected release waiter或supervisor job | 正常则首次直poll；cancel立即terminal | handoff并首次poll同一个future；不得重新调用provider | 否 | cancel否（后台） | waiter存活时看到真实Ready/Pending |
| release first-Ready success | release operation完成provider side effect与本地entry删除 | body结果/`true` | terminal完成 | 否 | 已ack | `Ready(Ok)` |
| release首次真实Pending | 同一个release future | waiter正常等待；cancel立即terminal | exact future handoff | 否 | cancel否（后台） | `Pending` |
| provider release已执行、request-state尚未更新 | **同一个release future**拥有“provider done/local pending”phase | 尚未向活waiter返回 | 继续原future取得state lock并删除/terminalize entry；绝不再调用provider release | 否 | waiter活着是；cancel否 | 首poll已`Pending` |
| release provider error | receipt `ReleaseFailed`，request state保留fail-closed fence entry | lease-lost若已发生仍优先；否则release error覆盖body result/error，与现状一致 | 不重放；request teardown可ack error | 否 | 非cancel路径是 | Ready/Pending依真实首poll |
| release waiter drop | request supervisor | cancel | exact future继续；late状态只写request lifecycle，不写heap/env | 否 | 否（后台） | N/A |

当前 evaluator的可观察顺序就是先 await `lease_lost`、再 await release、再判 flow：
`runtime/eval/src/program_db.rs:368-400`。表中保留该顺序。file cascade、retention roots和
lease fence也保持 provider command内部现有顺序：
`runtime/service-db/src/prepared_runtime/update.rs:142-207`、
`replace.rs:117-183`。

## 5. `DECISION_REQUIRED` 的精确选项

### 选项 A：`ABORT_UNTIL_COMMIT_ACK`

commit返回ack前，cancel/drop都丢commit并启动abort。

后果：

- 用户期待取消后 DB rollback；
- 但首次 `Pending`无法证明 provider尚未执行commit；
- commit可能已经成功而ack尚未返回，abort可能无效；
- 同一个session已经进入opaque Mongo commit future，drop后没有安全方法取回并证明abort；
- 要诚实实现必须增加provider commit point/outcome token、可取消commit或幂等resolution协议。

因此当前仓库无法证明该选择，选择A会把状态变成 `TASK_SCOPE_EXPANDED`。

### 选项 B：`COMMIT_INTENT_WINS`（推荐）

`DbTransactionGuard::commit(self)` 的CAS是commit selection。selection之前cancel/drop选择abort；
selection之后无论 waiter是否仍在，都继续**同一个**commit composite future。

后果：

- caller仍立即观察cancel/deadline，late success/error被丢弃；
- 后续request可能看到已提交写入；
- 不伪称cancel是撤销，符合 runtime commit-point规则；
- session与future可以由 preserving-first-poll协议无损移交；
- exactly-once含义清楚：commit side effect最多启动一次，fallback abort只在同一composite的
  error路径启动一次。

需要用户明确确认，因为这会把 O6 的 cancel真值从“总是abort”改为“commit selection前abort，
selection后commit”。

## 6. Preserving-first-poll 可行协议

### 6.1 冻结协议

采用 **request-owned terminal supervisor + waiter-to-supervisor same-future handoff**：

1. `DbCapabilitySource::request_capability`创建 request-local registry，并返回
   `DbRequestCapability { context, cleanup_owner }`。它把 cloneable lifecycle handle附加到
   factory返回的context；factory本身仍只负责provider context。context/store clone只clone
   handle；该handle同时是registry的counted producer permit，故
   `OwnedProgramExecutionContext`等逃逸clone仍被计数。唯一且不可clone的cleanup owner留在
   outer Host request task。
2. `DbCapabilityContext::require_store`把同一lifecycle handle附加到store wrapper；public
   `prepare_*` wrapper再把它传给provider implementor，concrete factory/context不需要复制第二套
   registry。
3. 每个 begin/claim/commit/abort/release provider operation先构造成
   `Pin<Box<dyn Future<Output = DbCapabilityResult<T>> + Send + 'static>>`，不借
   heap/env/evaluator/program context。
4. prepared operation同时携带one-shot **abandon continuation**。它只在waiter已经消失时消费
   provider outcome：begin late-success接唯一abort，claim late-success接唯一release，terminal
   operation只settle receipt并丢弃late value/error。continuation本身也是heap/env-free。
5. `DbOperationWaiter<T>::poll`第一次由 evaluator/Actor线程直接调用内部 provider future的
   `poll(cx)`；不经过channel、spawn或worker调度。
6. 首poll `Ready`时，同一poll中settle receipt并返回；E3看到真实 `Ready`，不切segment。
7. 首poll `Pending`时，waiter记录 `started=true`，但仍独占同一个 pinned box；E3看到
   真实 `Pending`并按既有合同suspend。
8. waiter继续存在时仍由waiter poll。waiter Drop使用同步CAS
   `WAITER -> SUPERVISOR`，从 `Option<Pin<Box<_>>>` take该box和对应continuation，并通过request
   registry的non-blocking channel移交；underlying future没有move、drop或rebuild。unpolled
   begin/claim按policy直接记 `CancelledBeforeStart`，不移交；已选择的commit/abort/release即使
   尚未poll也必须移交。每个waiter/guard持producer permit，因此root seal后才drop的逃逸task仍能
   完成handoff。
9. supervisor把typed future包进一个type-erased completion job：
   `async move { let outcome = same_future.await; abandon(outcome).await }`。wrapper只是新driver，
   不重建provider operation。
10. begin/claim job在seal后才late-success时，abort/release由该job的abandon continuation**原地
    串接并await**；不额外排队或依赖另一个producer permit。一个registered job只有mandatory
    continuation也terminal后才算完成，因此不存在“root seal与late guard Drop竞态”。
11. guard在body drop且尚无terminal时通过CAS选 `Abort` / `Release`，直接把唯一、尚未poll的
    terminal composite登记给supervisor。root evaluator的Drop发生在execution lexical scope
    结束时；逃逸producer则可在root seal后Drop，但其permit仍保持channel开放。evaluator
    lifecycle state的Drop必须显式先
    `take()`/drop active body wait、再drop transaction/lease guard，不能依赖struct字段析构
    顺序，否则body wait仍可能持request-state mutex。此路径没有Actor waiter，因此不要求Actor
    首poll。
12. request teardown先结束拥有pinned root execution storage的lexical scope；再调用
    `cleanup_owner.seal()`，它只drop owner admission sentinel，不假定所有context clone已drop。
    receiver在所有issued producer permit释放后才close；`join`同时等待channel close、queue排空及
    active job（含mandatory continuation）完成。outer Host request task在可见response/cancel
    terminal确定后执行该join/ack。
13. receipt只记录provider/request-state terminal；它没有heap/env指针。late terminal不能重新
    materialize evaluator value、rollback heap或修改binding。

### 6.2 Rust ownership 可行性

最小内部形状：

```rust
type DbOwnedFuture<T> =
    Pin<Box<dyn Future<Output = DbCapabilityResult<T>> + Send + 'static>>;

type DbAbandon<T> =
    Box<
        dyn FnOnce(DbCapabilityResult<T>) -> ErasedDbCleanup
            + Send
            + 'static
    >;

enum DbOperationDropPolicy {
    CancelIfUnpolled,
    MustComplete,
}

struct DbOperationWaiter<T> {
    future: Option<DbOwnedFuture<T>>,
    abandon: Option<DbAbandon<T>>,
    receipt: DbTerminalReceipt,
    registry: DbTerminalRegistry,
    drop_policy: DbOperationDropPolicy,
    started: bool,
}

type ErasedDbCleanup =
    Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
```

- `Pin<Box<F>>`本身可以在owner之间移动；被pin的是box内的future，underlying `F`地址不变。
- `Future::poll`直接委托内部future，故首poll不引入调度。
- `Drop`可以 `Option::take()`；它不需要也不得await。
- registry worker只接收 `ErasedDbCleanup`，不要求把不同 `T`放进同一个enum；`DbAbandon<T>`
  在type erase之前消费typed outcome。
- receipt/phase用 atomic CAS + `Notify`；waiter与worker不允许同时poll同一future。
- lifecycle handle clone就是counted sender/producer permit；receiver只有在owner sentinel与所有
  context/store/waiter/guard clone都drop后才close。
- cleanup owner持owner admission sentinel与worker `JoinHandle`，worker独占receiver；seal
  drop sentinel后join handle。active job内部串接的abort/release属于同一个join计数。
  worker/receiver不得反向持producer permit，否则channel永不close。这不是裸 `tokio::spawn`
  后丢handle。

### 6.3 Request teardown与永不返回

权威 runtime语义已冻结：deadline/cancel的**可见**结果立即确定，外层不等待后台cleanup
（`doc/reference/runtime.md:185`）。因此：

- normal/body-error路径继续等待commit/abort/release ack，保持当前顺序；
- root/lane cancel、deadline、disconnect与future drop共享同一 handoff；可见terminal不等ack；
- Host在可见terminal发送/认领后仍持 request cleanup join；`RequestSupervisor`或
  `ActorOwnerInvocationRegistry`把entry从`Executing`改为`Cleaning`而不是remove，receipt完成
  且所有DB context producer permit归零后才remove并销毁request DB state。这样现有顶层spawn
  即使不保存handle、stream task仍短暂持context，也仍被registry计数；
- fake never-ready operation必须让receipt保持 `Pending`且start计数为1，不能伪造ack或重试；
- repository目前没有实现 `doc/reference/runtime.md:193`提到的generic cleanup grace/platform
  limit。本DAG不得私自新增duration；永久never-ready只能保持为supervisor中可计数的pending
  cleanup，直到provider完成或runtime进程终止。

若后继验收要求“物理cleanup必须在固定时限内结束”，必须停下并交给既有 timeout/host-operation
policy owner；不得在DB节点发明一个毫秒值。这个限制不改变request可见terminal，也不是本节点的
第二个产品语义选择。

### 6.4 明确排除

| 方向 | 排除原因 |
| --- | --- |
| 从一开始spawn provider operation | worker调度会把同步Ready伪装成channel Pending |
| waiter drop后重新调用trait method | provider side effect可能已启动；commit已移走session；release outcome未知 |
| blocking Drop | provider可Pending，阻塞runtime线程 |
| 裸 `tokio::spawn` cleanup | 无request注册、join、receipt或teardown |
| atomic bool但drop原future | 只能防double-start，不能让第一次operation terminal |
| clone/recreate future | Rust future不是可复制operation；违反exactly-once |
| 依赖Mongo session Drop或lease TTL | 无provider ack；fake capability无法证明；TTL只覆盖进程失败恢复 |
| 给DB cleanup私加timeout | 违反任务与runtime timeout owner边界 |
| late completion回调 evaluator | evaluator heap/env可能已结束，违反owner边界 |

## 7. 后继可直接实现的 API 草图

名称可按crate风格微调，但ownership与方法消费关系不得改变。

### 7.1 Capability-context

```rust
#[must_use = "request capability cleanup owner must be retained"]
pub struct DbRequestCapability {
    context: DbCapabilityContext,
    cleanup_owner: DbRequestCleanupOwner,
}

impl DbRequestCapability {
    pub fn into_parts(
        self,
    ) -> (DbCapabilityContext, DbRequestCleanupOwner);
}

impl DbCapabilitySource {
    pub fn request_capability(
        &self,
        owner: impl Into<String>,
        request_id: impl Into<String>,
    ) -> DbRequestCapability;
}

#[derive(Clone)]
pub struct DbRequestLifecycleHandle {
    registry: DbTerminalRegistry, // clone = counted producer permit
}

#[must_use = "request DB cleanup must be sealed and joined"]
pub struct DbRequestCleanupOwner {
    // !Clone: owner admission sentinel + structured worker JoinHandle
}

impl DbRequestCleanupOwner {
    // Drops only the owner admission sentinel. Issued producer permits remain valid.
    pub fn seal(self) -> DbSealedRequestCleanup;
}

#[must_use = "sealed request DB cleanup must be joined"]
pub struct DbSealedRequestCleanup { /* !Clone */ }

impl DbSealedRequestCleanup {
    // Waits for producer permits == 0, queue empty, and active jobs terminal.
    pub async fn join(self) -> DbCleanupReport;
}

pub struct PreparedDbLifecycleOperation<T> {
    wait: DbOwnedFuture<T>,
    lifecycle: DbRequestLifecycleHandle,
    receipt: DbTerminalReceipt,
    drop_policy: DbOperationDropPolicy,
    abandon: DbAbandon<T>,
}

impl<T: Send + 'static> PreparedDbLifecycleOperation<T> {
    pub fn into_waiter(self) -> DbOperationWaiter<T>;
}

pub struct DbTerminalReceipt { /* phase + Notify + outcome class */ }
pub enum DbTerminalKind { Begin, Commit, Abort, Claim, Renew, Release }
pub enum DbTerminalOutcome { Succeeded, ProviderError, CancelledBeforeStart }
```

`DbCapabilitySource`内部先创建 `(DbRequestLifecycleHandle, DbRequestCleanupOwner)`，再调用现有
`DbCapabilityFactory::context_for_request`，把handle附到返回的`DbCapabilityContext`。
`DbCapabilityContext::require_store`继续调用provider context，但把同一handle附到
`DbCapabilityStore` wrapper。因此factory trait不变，且concrete service-db类型不会泄漏到Host。
旧的只返回context、会静默丢cleanup owner的source构造路径在L4删除。

`PreparedDbLifecycleOperation` / waiter均不实现 `Clone`；`into_waiter(self)` one-shot。
provider future与output均为 `Send + 'static`。Drop合同：

- unpolled begin/claim wait：不启动；
- actual-Pending wait：handoff exact future；
- selected terminal wait：无论是否poll都必须由waiter或supervisor之一驱动；
- handoff后的begin/claim late-success：在同一registered job内运行abandon continuation，分别
  abort/release，不能在seal后重新注册；
- completed receipt幂等，第二个terminal selection稳定失败。

Transaction API：

```rust
pub trait DbCapabilityStoreApi: Send + Sync {
    fn prepare_begin_transaction(
        &self,
        lifecycle: DbRequestLifecycleHandle,
    ) -> DbCapabilityResult<PreparedDbLifecycleOperation<DbTransactionGuard>>;
    // ordinary/raw/prepared-runtime APIs unchanged
    fn prepare_claim_lease(
        &self,
        lifecycle: DbRequestLifecycleHandle,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> DbCapabilityResult<
        PreparedDbLifecycleOperation<Option<DbLeaseGuard>>
    >;
}

impl DbCapabilityStore {
    // Public wrappers obtain lifecycle from self; evaluator never supplies it.
    pub fn prepare_begin_transaction(
        &self,
    ) -> DbCapabilityResult<PreparedDbLifecycleOperation<DbTransactionGuard>>;

    pub fn prepare_claim_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> DbCapabilityResult<
        PreparedDbLifecycleOperation<Option<DbLeaseGuard>>
    >;
}

pub struct DbTransactionGuard { /* !Clone, terminal CAS, provider handle */ }

impl DbTransactionGuard {
    pub fn prepare_commit(
        self,
    ) -> DbCapabilityResult<PreparedDbLifecycleOperation<()>>;
    pub fn prepare_abort(
        self,
    ) -> DbCapabilityResult<PreparedDbLifecycleOperation<DbAbortAck>>;
}
```

按推荐决策，`prepare_commit(self)`消费guard并CAS选择commit；其provider future内部包含既有
lease-fence → commit → error时fallback-abort顺序。`DbAbortAck`不改变public error surface；
只让receipt/fake观察provider error是否被抑制。

Lease API：

```rust
pub struct DbLeaseGuard {
    value: DbDocument,
    ttl_ms: u64,
    // !Clone; hold/provider handle private
}

impl DbLeaseGuard {
    pub fn value(&self) -> &DbDocument;
    pub fn ttl_ms(&self) -> u64;
    pub fn start_renewal(
        self,
        request_cancelled: Arc<AtomicBool>,
    ) -> DbCapabilityResult<DbLeaseScope>;
}

pub struct DbLeaseScope { /* !Clone; renew owner + terminal CAS */ }

impl DbLeaseScope {
    pub fn prepare_finish(
        self,
    ) -> DbCapabilityResult<PreparedDbLeaseFinish>;
}

pub enum DbLeaseFinishAck {
    Released,
    LeaseLost,
}

pub type PreparedDbLeaseFinish =
    PreparedDbLifecycleOperation<DbLeaseFinishAck>;
```

`PreparedDbLeaseFinish`内部固定执行“stop future renews → join current exact renew → snapshot
lease-lost → poll exactly one release → settle receipt”。若lease已lost，它仍等待release，但
`DbLeaseFinishAck::LeaseLost`覆盖release error；否则release error作为outer
`DbCapabilityResult::Err`，从而保持§4.2顺序。renew interval保持当前
`(ttl_ms / 3).max(1)`与第一次interval tick行为；不新增timeout。`DbLeaseScope::drop`把同一个
stop/join/release composite交supervisor。

旧 borrowed transaction/lease capability methods：

```text
begin_transaction
commit_transaction
abort_transaction
claim_lease
renew_lease
release_lease
lease_lost
```

在O6R迁完后全部删除。没有非evaluator production caller使用这些terminal方法；file runtime只用
file-record methods。因此最终不保留compatibility层。

### 7.2 Concrete service-db

`DbRequestState`保持request owner，但transaction/lease变成显式phase：

```rust
enum DbTransactionState {
    Active { session: ClientSession },
    Terminal {
        kind: CommitOrAbort,
        receipt: DbTerminalReceipt,
        // session is reachable through the exact terminal future
    },
    Finished,
}

struct DbLeaseEntry {
    hold: DbLeaseHold,
    phase: DbLeasePhase,
}
```

具体合同：

- begin成功前session在exact begin future；成功后只进入 `Active`一次；
- raw与O5R2 wait只在 `Active`借 `&mut session`，现有file cascade/retention-root顺序不变；
- terminal CAS后session移入exact commit/abort future，state保存kind/receipt，禁止第二terminal；
- commit error的fallback abort留在同一composite future；
- claim provider成功后先登记 `DbLeaseEntry::Active`再向waiter交guard；provider已经给出hold但
  local登记失败时，同一claim composite先release一次再返回原error，不能让error outcome丢hold；
- renewal与release都持owned concrete hold，不再借caller handle；
- release provider成功后，同一个future再把entry标 `Released`/删除；
- release error标 `ReleaseFailed`并保留fail-closed fence，不重试provider；
- lease-lost仍在renew false/error与write fence failure时先写request state；
- terminal receipt完成前session/hold/runtime owner均可达。

### 7.3 Request owner

`RuntimeAssemblyExecutionContext`和`ActorMethodEvalExecution`必须在execution future之外通过
`DbCapabilitySource::request_capability`创建一次request bundle，并拆出：

```text
DbCapabilityContext     -> 传入 ProgramExecutionContext / FileCapabilityContext
DbRequestCleanupOwner   -> 只留在 outer Host request task
```

outer task不得为了方便在cleanup owner旁再保留一个base context clone；context必须move进root
execution。由`OwnedProgramExecutionContext`逃逸的clone则自然持counted permit，直到producer
task析构。

teardown顺序：

```text
let result = {
  create + pin execution
  -> select semantic result/cancel/deadline
}                            // 结束拥有pin storage的scope，真实drop execution
  -> let sealed = cleanup.seal()
                               // drop root sentinel；issued permits仍被计数
  -> tracker.claim_visible_terminal_and_mark_cleaning()
  -> settle/send visible request terminal
  -> sealed.join().await      // structured ack；不改变已选visible terminal
  -> tracker.finish_cleanup() // generation-matched remove
```

不能只对`tokio::pin!`产生的`Pin<&mut _>`调用`drop`后假设underlying future已析构；必须结束
拥有hidden pinned storage的lexical scope，或改成可显式drop的owning `Box::pin`。

HTTP、WebSocket connect与WebSocket JSON-RPC使用`RequestSupervisor`的
`Executing -> Cleaning -> removed`两阶段entry；Actor owner invoke对
`ActorOwnerInvocationRegistry`做同样phase split。`cancel`只作用于`Executing`；session
disconnect会取消executing entry，但不能clear/drop正在Cleaning的entry或cleanup owner。
`active_count`/health在cleanup ack前仍计数。四个入口都必须接入；只修HTTP会让同一个capability
在其它真实入口继续泄漏。

### 7.4 Fake store观测合同

fake state至少记录每个operation id/kind的：

```text
prepared
first_poll
poll_count
provider_side_effect_started
terminal
future_drop
waiter_drop
handoff
abandon_followup_start
abandon_followup_terminal
receipt_ack
producer_permits_active
```

gate模式至少有 `FirstReady(Ok/Err)`、`PendingThen(Ok/Err)`、`NeverReady`。断言：

- first Ready时handoff/worker poll均为0；
- first Pending后waiter drop只handoff一次，provider start为1；
- commit/abort、renew/release的terminal CAS只有一个winner；
- late terminal不调用任何heap/env finalizer；
- provider release成功/local-state pending时provider start仍为1；
- begin/claim在seal后late-success时不重新注册，abandon follow-up各只启动/完成一次；
- claim provider成功但local登记失败时，原error只在唯一release terminal后ack；
- never-ready receipt保持pending且没有restart；
- root seal后若escaped context permit仍存活，join保持pending；最后permit drop后才允许registry
  close；
- request visible terminal可以先于cleanup ack，但normal/error非cancel路径必须等ack。

## 8. 影响面与非目标结论

### 8.1 必须迁移的 production call site

- `runtime/capability-context/src/db.rs`及新 lifecycle child module/public re-export；
- `runtime/service-db/src/{capability,lib,store}.rs`及新 terminal lifecycle child；
- `runtime/service-db/src/prepared_runtime/store.rs`，只适配transaction phase访问；
- `runtime/eval/src/program_db.rs`及 transaction/lease/DB wait child modules；
- `runtime/host/src/eval_capability_adapter/{assembly_execution_context,actor_method_adapter,assembly_request_adapter}.rs`；
- `runtime/host/src/host/request_entry/{assembly,websocket_jsonrpc}.rs`；
- `runtime/host/src/host/{actor_owner_execution,actor_owner_invocations,request_supervisor}.rs`。

若cleanup bundle能完全留在Host adapter/outer task，`runtime/request` trait无需变化；若实际Rust
lifetime迫使handle穿过request execution handles，才允许窄改：

- `runtime/request/src/{http_gateway_execution,websocket_connect_execution,websocket_jsonrpc_execution}.rs`。

停止条件是不得把concrete service-db类型泄漏进request crate；穿过边界的只能是
capability-context cleanup owner/receipt。

### 8.2 必须迁移的 tests/fixtures

- capability fake的两个 `DbCapabilityStoreApi` implementor与
  `raw_read_api.rs` transaction/lease macro；
- `runtime/service-db/src/tests.rs`与 `tests/prepared_runtime/**`；
- `runtime/driver/eval/eval_context/tests.rs`、
  `runtime/driver/eval/tests/program_execution.rs` 中的
  `ServiceDbCapabilityHandle::with_state` fixture；
- `runtime/host/src/loader/assembly_admission/tests/execution/runtime.rs`；
- `runtime/host/src/host/file_runtime/tests.rs`；
- host router-session中的两个factory/context fixture及其source request构造断言
  （factory trait本身保持不变）；
- `runtime/host/src/host/router_session/tests/websocket_generation_lifecycle.rs`中的直接
  `DbCapabilitySource` request构造；
- O6R新增的真实 `program_db` + Actor frame store fixture。

真实Mongo roundtrip继续ignored；所有后继GREEN必须 `CARGO_NET_OFFLINE=true`，不得使用network。

### 8.3 已检查且不扩大语义

- **raw DB in transaction**：仍由 request state拥有session并跨await借用；不需要给每条raw op新
  transaction token。
- **O5R2 prepared runtime wait**：继续持owned store并访问同一session；不回退旧heap-borrowing
  路径。
- **file cascade / retention roots**：仍在同一 provider transaction及现有顺序内；terminal owner
  只改变session可达性。
- **lease-lost / Mongo error顺序**：保持 §4.2 与当前 evaluator顺序；不新增错误类型。
- **cancel/timeout/disconnect/future drop**：都先使waiter drop，再走同一个handoff/seal；不建四套
  cleanup。
- **escaped owned context / stream producer**：不改stream scheduler；DB lifecycle handle本身作为
  counted permit，cleanup join等待其归零，因此root future drop不再被误当成所有context已释放。
- **nested/illegal transaction flow**：仍由现有 nested检查与 evaluator flow error产生abort；
  heap checkpoint仍只rollback新增allocation，不声称撤销既有heap mutation。
- **Actor**：只消费 E3 `await_if_pending`；不改Actor lease、field codec、scheduler或E3。
- **语言/wire/TTL**：均不改；lease进程失败仍可由TTL恢复，但TTL不是request内exact terminal的
  替代品。

## 9. 条件实现 DAG

只有用户先选择 §5 的 commit取消语义，DAG才进入 implementation。下列节点中只有 L2 与 L3
可以并行；重复文件owner的节点严格串行，不能在不同worktree并发修改。

### D0 — Commit cancellation decision

- **直接前置**：本 result。
- **production/test写集**：无。
- **输出**：确认 `COMMIT_INTENT_WINS`（推荐）或 `ABORT_UNTIL_COMMIT_ACK`。
- **停止条件**：若选后者，不执行L1，另发provider outcome/resolution架构节点并标
  `TASK_SCOPE_EXPANDED`。

### L1 — Capability preserving-first-poll seam

- **直接前置**：D0、`a5920a32`、`98add404`。
- **独占写集**：
  `runtime/capability-context/src/db.rs`、
  `runtime/capability-context/src/db/terminal_lifecycle.rs`、
  `runtime/capability-context/src/db/terminal_lifecycle_tests.rs`及child fixtures、
  `runtime/capability-context/src/db/prepared_runtime_tests/fake_store/prepared.rs`、
  `runtime/capability-context/src/lib.rs`。
- **test-first RED**：先写 direct Ready、Pending handoff、never-ready、CAS double-terminal、
  guard-drop fallback、seal后begin/claim late-success continuation、escaped producer permit
  延迟channel close测试；缺少types/methods应编译失败。新trait method在L1到L4之间可有明确
  标注的temporary unavailable default，让L2与L3独立编译；三个implementor迁完后L4删除default
  与旧borrowed methods。
- **focused GREEN**：

  ```bash
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo test -p skiff-runtime-capability-context \
      db_terminal_lifecycle -- --nocapture
  ```

  result必须记录实际非零数，至少覆盖12个合同测试；0个匹配不算GREEN。
- **验收**：Ready worker poll=0；Pending exact handoff；unpolled begin/claim不启动；selected
  unpolled terminal仍启动一次；terminal CAS一个winner；seal后late-success在同一job串接cleanup；
  escaped context最后一个permit drop前join不完成；never-ready无假ack；
  future drop/start/poll计数精确。
- **停止条件**：需要unsafe、blocking Drop、future重建、caller heap/env borrow或裸spawn。
- **旧接口**：本节点新增owned seam；旧borrowed methods尚未被新consumer使用，不在此并行删除。

### L2 — Concrete service-db transaction/lease owner

- **直接前置**：L1。
- **独占写集**：
  `runtime/service-db/src/capability.rs`、
  `runtime/service-db/src/lib.rs`、
  `runtime/service-db/src/store.rs`、
  `runtime/service-db/src/prepared_runtime/store.rs`、
  新 `runtime/service-db/src/terminal_lifecycle.rs`、
  `runtime/service-db/src/tests.rs`与窄 `tests/terminal_lifecycle/**`。
- **test-first RED**：hermetic provider driver先证明session/hold在terminal Pending可达、
  commit/abort CAS、claim provider-success/local-registration-error fallback release、
  release provider-done/local-pending、renew awaiting→release顺序。
- **focused GREEN**：

  ```bash
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo test -p skiff-runtime-service-db \
      service_db_terminal_lifecycle -- --nocapture
  ```

  实际必须非零，至少12项；另跑 `prepared_runtime` 现有11项。
- **验收**：begin/claim/commit/abort/renew/release Ready/Pending/drop；provider start exactly once；
  release local state最终收束；O5R2 session/cascade/roots/lease-lost顺序无回归。
- **停止条件**：需要真实Mongo、无法用hermetic driver证明concrete state、或必须改变DB/error/TTL。

### L3 — Request cleanup owner与真实teardown checkpoint

- **直接前置**：L1；可与L2并行。
- **独占写集**：
  Host eval adapters、HTTP/WS/Actor request entry、`request_supervisor.rs`、
  `actor_owner_invocations.rs`及相应Host tests；
  只有lifetime确实要求时才加入§8.1列出的三个 `runtime/request` 文件。
- **test-first RED**：fake never-ready DB terminal在root cancel/deadline/route disconnect后：
  lexical pin scope先结束并真实drop execution，waiter handoff一次，visible terminal先确定，
  registry entry进入`Cleaning`且`active_count`仍为1，outer task仍持join；另用
  `OwnedProgramExecutionContext`/stream-style escaped context证明root drop后permit仍阻止cleanup
  remove；
  gate放行后receipt ack且registry清零。
- **focused GREEN**：

  ```bash
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo test -p skiff-runtime-host db_request_cleanup -- --nocapture
  ```

  若改request crate，再跑同名 `-p skiff-runtime-request` selector；每个声称的selector实际非零。
- **验收**：HTTP、WS connect、WS JSON-RPC、Actor owner四入口；normal/error/cancel/deadline/drop；
  visible terminal不被cleanup覆盖；cancel/session disconnect不删除Cleaning entry；cleanup ack后
  且producer permit归零才generation-matched remove；无未注册的detached cleanup JoinHandle。
- **停止条件**：只能把cleanup owner放回会被drop的execution future；或需要新cleanup timeout。

### O6R — Evaluator DB actual-Pending state machines重发

- **直接前置**：L2、L3及D0确认后的O6 task修订。
- **独占写集**：沿原O6 evaluator写集，只改
  `runtime/eval/src/program_db.rs`、`program_db/**`、`db_eval.rs`、`db_eval/**`及其tests/result。
- **只消费接口**：
  `prepare_begin_transaction -> DbTransactionGuard`、
  `guard.prepare_commit/abort`、
  `prepare_claim_lease -> DbLeaseGuard`、
  `start_renewal/prepare_finish`、
  `PreparedDbLifecycleOperation::into_waiter`、
  既有O5R2六个prepared runtime operation，
  以及E3 `await_if_pending`。
- **test-first RED / GREEN**：沿原O6矩阵，并新增 decision行、never-ready waiter drop与
  provider-done/local-pending release。命令：

  ```bash
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo test -p skiff-runtime-eval program_db -- --nocapture
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo test -p skiff-runtime-eval db_actor -- --nocapture
  ```

  两个result中声称的selector都必须实际非零。
- **验收**：所有外部wait first Ready不切segment，first Pending只切一次；late terminal不写heap/env；
  transaction两种source共用guard；state Drop先销毁active wait再触发guard；lease renew
  stop/join/release exactly once。
- **停止条件**：需要改E1/E3、复制scheduler、借heap/env跨wait或重新选择本result已冻结的owner。
- **O6R重发时机**：L2和L3各自GREEN并集成后；不能仅L1 seam完成就重发。

### L4 — 删除旧 borrowed terminal API

- **直接前置**：O6R。
- **独占写集**：重新独占（不与其它节点并发）
  `runtime/capability-context/src/db.rs`及fake macros、
  `runtime/service-db/src/capability.rs/store.rs`与相关tests。
- **test-first RED**：先加source-boundary反向检查或编译fixture，禁止旧七条method被声明/调用，
  禁止Host request继续调用会丢cleanup owner的
  `DbCapabilitySource::context_for_request`，并禁止L1 temporary terminal defaults残留。
- **focused GREEN**：

  ```bash
  rg -n 'begin_transaction|commit_transaction|abort_transaction|claim_lease|renew_lease|release_lease|lease_lost' \
    runtime/capability-context/src/db.rs runtime/eval/src/program_db.rs
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<worktree>/build/cargo-target \
    cargo check -p skiff-runtime-capability-context \
      -p skiff-runtime-service-db -p skiff-runtime-eval --locked
  ```

  `rg`只允许新prepared/guard命名与文档注释，不允许旧borrowed trait/wrapper/call。
- **验收**：三个store implementor全部迁完；无compat/default fallback；file-record非terminal API仍工作。
- **停止条件**：发现新的非evaluator production terminal caller；此时只允许同提交迁移，不保留旧层。
- **旧接口删除owner**：本节点。

### R — Combined acceptance

- **直接前置**：L4、O6R。
- **production/test写集**：无；只新增combined result。
- **唯一完整gate owner**：

  ```bash
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target \
    cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target \
    cargo test -p skiff-runtime-service-db --locked --no-fail-fast
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target \
    cargo test -p skiff-runtime-eval --locked --no-fail-fast
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target \
    cargo test -p skiff-runtime-host db_request_cleanup -- --nocapture
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target \
    cargo check -p skiff-runtime-capability-context \
      -p skiff-runtime-service-db -p skiff-runtime-eval \
      -p skiff-runtime-request -p skiff-runtime-host --locked
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<integration>/build/cargo-target cargo fmt --check
  git diff --check
  ```

- **验收**：每个focused selector记录实际非零数；service-db真实Mongo test继续ignored；Ready/Pending/
  drop/never-ready矩阵、escaped context permit、request visible-terminal-before-cleanup、
  最终receipt清零全部通过。
- **停止条件**：任何test访问network/Mongo、零test、旧API反向搜索命中、worker/receipt未清零。
- **combined acceptance owner**：本节点；此前各节点不得重复跑完整跨crate gate。

## 10. 最终判定

owner与实现方式已经收敛，且不需要新语言、DB、Actor或wire设计；但 commit selection 后的取消会
改变持久化可观察结果，现有权威文本互相不足以替用户选择。因此本节点不能标
`READY_FOR_IMPLEMENTATION_DAG`。

下一步唯一需要用户回答：

```text
确认 COMMIT_INTENT_WINS：
commit guard一旦被消费并选择commit，即使同一future首次真实Pending后caller取消，
仍由request supervisor继续该commit；caller只丢弃late result。
```

确认后，从L1开始；若拒绝并选择“ack前都abort”，停止当前DAG并先设计provider outcome/resolution
能力。
