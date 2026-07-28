# P5-F445H-J1 prepared operation combined review result

状态：`PASS / E4R_EXECUTABLE`。

冻结代码候选
`4a6c70b9d6acd852956d6ad4e742a66053e1776a` 已闭合 O1–O6 operation owner
prerequisite。现存 evaluator pre-suspend 可以只在 E4R call-site 层改接
`prepare -> Ready / heap-free wait -> E3 await_if_pending -> finalize`；本次独立审查没有发现
必须回改 native、outbound、Actor invocation、provider、callback、service-db 或 eval DB owner
的 blocker。

J1 只证明 operation owner prerequisite 闭合，不关闭 F445H 或 Phase 05。E4R 仍须删除静态
pre-suspend、完成 timeout/concurrent/catch/checkpoint/stream 接线并运行自己的完整 gate；I6
仍未开始。

## 1. Verdict

- Verdict：`PASS / E4R_EXECUTABLE`。
- Blocking issues：无。
- Non-blocking follow-up：
  - `runtime/eval/src/eval_context.rs` 当前为 2159 行，九处旧 pre-suspend 和多种 call target
    仍集中在同一文件。E4R 应把共同的 actual-Pending 组合收进窄 helper，避免为每个 branch
    再复制一套 poll/suspend/finalize 样板；这是维护性建议，不是 owner prerequisite blocker。
  - 联合编译仍报告既有 compiler-source unused import、linker dead-code、
    `service_error_channel.rs` unreachable-pattern 和 ordinary test unused-import warning。
    本候选 focused owner 路径没有新增 warning，warning cleanup 不阻塞 E4R。

## 2. 候选与写集

| 项 | 值 |
| --- | --- |
| frozen code candidate | `4a6c70b9d6acd852956d6ad4e742a66053e1776a` |
| review contract HEAD | `1f156d140581f122994f1aca4cf6f882eb49bda9` |
| branch | `codex/p5-f445h-j1` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-j1` |

`4a6c70b9..1f156d14` 只新增
`P5-F445H-J1-prepared-operation-combined-review.md`；production、tests、Cargo 和 lockfile
均无差异。O6 验收代码候选 `ce01def6..4a6c70b9` 也只新增 O6R13 task/result 两份文档，
因此 O6R13 的代码事实与本候选相同。

各 operation owner 的 task-branch implementation 经过 integration cherry-pick 后 patch-id
保持一致：

| task implementation | integration commit | patch-id |
| --- | --- | --- |
| `70598c80` | `f0886061` | match |
| `956c2963` | `030a3571` | match |
| `010a6bcd` | `b1f781ab` | match |
| `acce0964` | `821f7b15` | match |
| `91b35b05` | `5cac388b` | match |
| `dfe24f12` | `968927cf` | match |

本 review 没有修改 production、tests、fixture、既有 task/result、Cargo、manifest 或 lockfile，
没有 merge、rebase、push，也没有启动 stable、live、MongoDB 或 network。

## 3. Operation owner 独立审查

### 3.1 Native

- `runtime/native/src/dispatch/prepared.rs::PreparedNativeCall` 精确区分
  `Ready(RuntimeValue)` 与 `ExternalWait(PreparedExternalNativeOperation)`；prepare 没有
  `Pending` 预判。
- `PreparedExternalNativeOperation::into_parts` 交出
  `NativeExternalWait` 与 `NativeExternalFinalize`。wait outcome 是 opaque owned value；
  `NativeExternalFinalize::finalize(self, outcome, &mut RequestHeap)` 才重新取得 caller heap，
  并以 heap checkpoint 回滚失败的本次 materialization。
- `runtime/native/src/dispatch/adapter.rs::NativeDispatch::prepare_resolved_native_call` 与
  `runtime/native/src/dispatch/core.rs::prepare_resolved_native_call` 是唯一 prepared route
  入口；旧 async dispatch 只调用 prepare、await 同一 wait、finalize。
- `runtime/native/src/dispatch/time.rs::TimeNativeDispatch::prepare` 对 sleep zero 仍返回
  `ExternalWait`。`sleep_for_millis` 在真实首次 poll 才决定 zero 直接 Ready；没有按 binding
  静态判断。
- `runtime/native/src/dispatch/websocket.rs::WebsocketNativeDispatch::prepare` 的四个 send
  同步调用 capability 后返回 `Ready`；`requestJsonToConnection` 才返回 owned external wait。
- `runtime/native/src/dispatch/file.rs::FileNativeDispatch::prepare` 在 wait 前完成
  string/bytes/options/file/stream plan 解码。`create_file_from_stream` 的
  `StreamConsumerCleanup` 与 end marker 由同一个 wait 拥有；自然 End disarm，error/drop
  只清理该 owner。
- `runtime/native/src/dispatch/http.rs::HttpNativeDispatch::prepare` 的 request/stream/SSE 和
  response emit 是 external wait；request/header/response helper 与 stream event constructor
  是 `Ready`。HTTP stream internal handle 只在 paired finalizer 中物化。
- `runtime/native/src/dispatch/actor.rs::ActorNativeDispatch::prepare` 在 prepare 中固化 actor
  key、activation fence 与 bootstrap；get/replace/find/remove wait 只持 owned request/context，
  ActorRef/bool 在 finalize 后返回。
- 对 `runtime/native/src/dispatch/**` 和 `runtime/native/src/capability.rs` 反向搜索
  `may_suspend|maySuspend|native_call_suspends|suspend_actor_segment|resume_actor_segment|yield_now`
  为零。native owner 不读取 callable `may_suspend`，也没有 pre-suspend 或 future restart。

结论：external wait 不借 caller `RequestHeap`、`Env` 或 `EvalContext`；simple future 的
compiler state、`StreamConsumerCleanup`、WebSocket request owner 和 Actor capability request
均随同一个 wait drop，不存在 drop 后重建或重放副作用的路径。

### 3.2 Outbound service、remote interface 与 Actor invocation

- `runtime/eval/src/service_dispatch.rs::prepare_outbound_service` 与
  `prepare_outbound_service_operation` 最终都进入
  `prepare_outbound_service_request`。payload encode 与唯一一次 `start_request` 在 prepare
  完成。
- `runtime/eval/src/service_dispatch/prepared_operation.rs::PreparedOutboundUnaryOperation`
  只保存 owned `OutboundServiceContext`、`OutboundServiceDispatch`、
  `OutboundRequestLease` 和 `OutboundResponseReceiver`；
  `into_wait(self) -> Future + Send + 'static` 不借 heap/env。
- `OutboundServiceUnaryCompletion::finalize(self, heap, env)` 才 decode、coerce 并检查 caller
  stream sink cancellation，失败回滚本次 heap allocation。
- `serverStream` 在 `outbound_service_stream_value` 同步建立 source-backed stream，
  lease/receiver 转交 `OutboundServiceStreamSource` 后返回
  `PreparedOutboundServiceCall::Ready`；等待只发生在 consumer `next()`。
- `OutboundRequestLease::{complete,cancel,drop}` 共享 atomic terminal，response error、drop 和
  late response 只能结算一个 owner。
- `runtime/eval/src/actor_dispatch.rs::prepare_actor_method` 在 caller heap 中完成 receiver、
  method/arity/plan 校验和 argument encode，再构造 owned `ActorInvocationRequest`。
- `runtime/eval/src/actor_dispatch/prepared_operation.rs::PreparedActorMethodInvocation`
  保存 `OwnedActorCapabilityContext`、request、return plan、method 与 timeout；
  `into_wait(self) -> Future + Send + 'static` 只启动一次 `invoke_actor`。
- `ActorMethodInvocationCompletion::finalize(self, heap)` 才处理 returned payload
  JSON/boundary import/coercion以及 cancel/timeout/Actor/transport error；returned payload
  失败回滚本次 heap materialization。

结论：legacy dependency 与 remote interface 共用同一 outbound owner；unary 与 Actor wait
均 heap/env-free。lease/invocation drop 与 late sender 的 focused matrix 证明副作用不重放、
terminal 不重复结算。

### 3.3 Activation-relative provider

- `runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary.rs::prepare_provider_unary`
  在 caller segment 内完成 contract/arity/effect/plan、provider target/context、独立 provider
  heap 与 caller-to-provider 参数 materialization。
- `PreparedProviderUnary` 的字段全部 owned：
  `Interpreter`、`OwnedProgramExecutionContext`、provider heap、detached invocation `Env`、
  addresses/type args/arguments、owned execution 与 `ProviderUnaryRequestOwner`。detached env
  只复制 stream capabilities/type substitutions，不复制 caller slots/self。
- `PreparedProviderUnary::wait(self) -> Future + Send + 'static` 只持 provider state；
  provider executable 在同一个 future 首次 poll 启动一次。`ProviderUnaryRequestOwner::drop`
  取消未完成请求，terminal 后 disarm，不接受 late completion。
- `CompletedProviderUnary::finalize(self, caller_heap)` 才导入 normal result或导出 provider
  failure；already-fixed failure不重复提交诊断。provider result materialization 的失败原子性
  由 canonical boundary plan 与 focused test覆盖。
- `OwnedProgramExecutionContext::capture` 不包含 `actor_execution_frame`；
  prepared test也直接断言 provider context无 caller Actor frame。
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs::start_provider_stream` 保持独立：
  setup同步建立 stream value并 spawn producer；producer、stream lifetime和 consumer各自拥有
  cleanup，没有进入 unary prepared protocol。

结论：unary Ready/Pending 使用同一个 owned wait，由 E3 真实首 poll决定；serverStream setup
不释放 Actor segment。

### 3.4 Callback

- `runtime/native/src/callback_adapter.rs::InProcessCallbackAdapter::try_lock_owner_heap_owned`
  clone `Arc<tokio::sync::Mutex<RequestHeap>>` 并返回
  `OwnedMutexGuard<RequestHeap>`；重入时立即 `OwnerStateUnavailable`，guard drop后可重新取得。
- `runtime/eval/src/assembly_execution/callback_native/prepared.rs::prepare_interface_call`
  完成 generation/owner/capability/contract/slot/method/arity 校验、owner activation切换、
  owned context capture、guard取得及 caller-to-owner 参数 materialization。
- `prepare_owner_arguments` 在本次 materialization 前 checkpoint；任一参数失败只回滚本次
  新 allocation并保留既有 owner state。
- `PreparedCallbackInvocation` 只保存 owned owner guard/context/call env/receiver/args/contract
  facts；不保存 caller heap/env/EvalContext。owner call env不复制 caller slots/self。
- `PreparedCallbackInvocation::wait` 在调用前再次 fail closed 检查 owner context无 Actor frame，
  并只调用一次 recursive owner evaluator。Pending future drop随同一个 guard释放 owner heap，
  没有 restart路径。
- `CompletedCallbackInvocation::finalize(self, caller_heap)` 在 guard仍存活时只导入一次 normal
  result；method error/cancel直接释放 guard且不伪造 rollback，保持已经发生的 owner mutation。

结论：callback wait不捕获 caller Actor frame，owner heap authority与 completed outcome均为
single-owner、one-shot。

### 3.5 Service DB 与 eval DB

- `runtime/capability-context/src/db/prepared_runtime.rs::DbPreparedRuntimeWait<T>` 是
  `Future + Send + 'static`；`PreparedDbRuntimeOperation::into_wait(self)` 消费唯一 wait，
  `DbRuntimeFinalizer::finalize(self, heap)` 才接收 caller heap并在失败时 rollback。
- `runtime/service-db/src/prepared_runtime/store.rs` 的六个 concrete入口：
  `prepare_find_one_by_key_runtime_operation`、
  `prepare_find_one_by_query_runtime_operation`、
  `prepare_find_many_page_runtime_operation`、
  `prepare_create_runtime_operation`、
  `prepare_update_one_runtime_operation`、
  `prepare_replace_one_runtime_operation`
  均先完成 owned command，再把 `ServiceDbStore`、runtime owner与 command移入
  `PreparedDbRuntimeOperation`。
- `PreparedFindOne`、`PreparedFindMany`、`PreparedCreate`、`PreparedUpdate`、
  `PreparedReplace` 只保存 String、Mongo plan/BSON document、recoverable context/roots、
  lease/cascade facts；不保存 caller heap、heap handle、evaluator或输入 `RuntimeValue`。
- create finalizer从 prepare时固化的 owned document重建结果，不保存原输入 handle。
  find-many zero limit仍先校验 query/order，再形成不启动 provider的空 plan。
- wait开始时取得同一个 request state；active transaction继续使用 owner session，update/replace
  继续使用 lease guards。unpolled/Pending drop只丢弃同一个 command future，不重建 Mongo
  command；implicit transaction/lease cleanup仍由既有 owner处理。
- `runtime/eval/src/program_db/wait.rs::await_operation` 是 raw/prepared、transaction和lease共用
  的唯一 E3 adapter：Actor frame存在时只调用
  `ActorExecutionFrame::await_if_pending`，否则 await同一个 future。
- `runtime/eval/src/program_db.rs::execute_db_command` 的 raw分支把 store和owned input移入
  `async move`；prepared分支只调用一次 concrete prepare、消费同一个 `into_wait()`，
  并在 `await_operation` 返回后才调用 finalizer/decode。
- `transaction.rs::TransactionLifecycle::{begin,commit,abort,abort_selected}` 通过相同
  `await_operation` 执行每个 external phase；commit error只选择一次 abort。该 owner没有
  `Drop` async cleanup、spawn或detached terminal。
- `lease.rs::LeaseRenewOwner` 只拥有一个 renew task。normal terminal
  `stop_and_join(self)`；outer drop只 abort同一个 task。claim/lost/release/read各自把一个
  owned future交给 `await_operation`，没有 detached cleanup或重建。
- `runtime/eval/src/db_eval.rs::DbIrEvaluator::eval_query_value` 只求值 query/options并
  `runtime_from_wire` 到 caller heap，不 require DB store、不发 I/O，因此 `DbQuery` 没有
  external wait，也不需要 Actor cut。

结论：service-db六条 prepared wait均不保存 caller heap/handle；eval raw/prepared、
transaction和lease使用同一 actual-Pending adapter。O6 已完成的 DB call-site 不需要 E4R
重新包 pre-suspend，只需保持现状。

## 4. Evaluator call-site → prepared owner 映射

当前 `eval_context.rs` 反向搜索得到九个 pre-suspend pair：

- stream emit三处；
- remote interface；
- callback capability；
- activation-relative service；
- Actor dispatch；
- legacy service dependency；
- native（由 `native_call_suspends` 条件控制）。

DB五个 expression arm已没有 pre-suspend。映射如下：

| current call site | prepare / synchronous step | Ready 或 heap/env-free wait → E3 | resume 后 finalize / 例外 |
| --- | --- | --- | --- |
| `eval_context.rs:398-455` 三条 `Emit` | runtime projection、跨 heap clone或 typed wire encode先完成 | `StreamSink::{send_internal_with_cancellation,send_with_cancellation}` future不借 caller heap；E4R交给 `frame.await_if_pending` | 只映射 send error；删除外层 pre-suspend |
| `eval_context.rs:1201-1240` remote interface | `prepare_outbound_service_operation` | `Ready` 或 `PreparedOutboundUnaryOperation::into_wait` → E3 | `OutboundServiceUnaryCompletion::finalize(heap, env)` |
| `eval_context.rs:1242-1257` callback | `prepare_interface_call` | `PreparedCallbackInvocation::wait(interpreter)` → E3 | `CompletedCallbackInvocation::finalize(heap)` |
| `eval_context.rs:1386-1418` activation-relative unary | resolve target后 `prepare_provider_unary` | `PreparedProviderUnary::wait` → E3 | `CompletedProviderUnary::finalize(heap)`；serverStream继续同步 `start_provider_stream`，不切 segment |
| `eval_context.rs:1427-1431` Actor method | `prepare_actor_method` | `PreparedActorMethodInvocation::into_wait` → E3 | `ActorMethodInvocationCompletion::finalize(heap)` |
| `eval_context.rs:1439-1460` legacy dependency | `prepare_outbound_service` | `Ready` 或同一 outbound unary wait → E3 | 同一 outbound finalizer；serverStream `Ready` |
| `eval_context.rs:1462-1506` ordinary native | `NativeDispatch::prepare_resolved_native_call` | `PreparedNativeCall::Ready` 原 segment；`ExternalWait::into_parts().0` → E3 | paired `NativeExternalFinalize::finalize`，再执行 native return carrier coercion |
| `eval_context.rs:1730-1834` `createFromStream` producer special path | producer setup和O1 native prepare都可在 consumer构造前完成 | 同一个 native external wait在 `exec_prepared_native_stream_producer_arg` consumer内交给 E3 | paired native finalizer；producer consumption/cleanup owner保持原样 |
| `program_db.rs:464-779` raw/prepared DB operation | raw command已owned；recoverable path调用六个 concrete prepare API | `wait::await_operation` → E3 | raw decode或 `DbRuntimeFinalizer::finalize(heap)` |
| `program_db/transaction.rs` transaction | `TransactionLifecycle::begin`创建唯一 lifecycle owner | begin/commit/abort各自owned future →同一 `await_operation`/E3 | phase transition即对应 finalize；body递归 operation各自 actual-Pending |
| `program_db.rs:325-457` lease claim/read | key encode；claim成功后建立唯一 `LeaseRenewOwner` | claim、stop/join、lost、release、read各自owned wait → E3 | binding/value只在wait后导入；normal先join renew再lost/release |
| `eval_context.rs:713-734` `DbQuery` | `DbIrEvaluator::eval_query_value` | 无 external wait | 同步例外：不释放 |
| `program_stream.rs:63-90` Actor stream consumer `next` | clone runtime/value/signals/token | `next_with_cancellation` →既有 `frame.await_if_pending` | resume后才 materialize item/error；无需owner修改 |

`program_invocation.rs` 的 response-stream consumer与
`program_stream.rs::drain_stream_producer_output` 使用独立 invocation/cleanup heap，不是 caller
Actor continuation call site，不需要套 E3 frame。

同步例外已经闭合：

- 四个 WebSocket send是 `PreparedNativeCall::Ready`，不释放；
- outbound与activation-relative `serverStream` 创建不释放，consumer `next`按真实 Pending；
- `DbQuery` 不释放；
- sleep zero虽为 `ExternalWait`，真实首次 poll Ready时不释放；
- 其它 prepared `Ready` 均留在当前 segment。

`runtime/eval/src/eval_context.rs::native_call_suspends`、一个静态调用点和九组 pre-suspend/
resume仍存在，精确符合 E4R 的预期 RED。owner production中没有任何代码要求保留这些
pre-suspend。

## 5. Selector listing 与 focused gate

所有 Cargo命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target
```

先分别以相同 selector执行 `-- --list`，再执行任务合同的 `-- --nocapture`。实际计数：

| selector | listing 主 binary | execution |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-native dispatch` | 37 | `37 passed; 0 failed; 76 filtered` |
| `cargo test -p skiff-runtime-eval service_dispatch` | 12 | `12 passed; 0 failed; 316 filtered` |
| `cargo test -p skiff-runtime-eval actor_dispatch` | 6 | `6 passed; 0 failed; 322 filtered` |
| `cargo test -p skiff-runtime-eval async_stream_cancel` | 31 | `31 passed; 0 failed; 297 filtered` |
| `cargo test -p skiff-runtime-eval callback_native` | 11 | `11 passed; 0 failed; 317 filtered` |
| `cargo test -p skiff-runtime-native callback_adapter` | 8 | `8 passed; 0 failed; 105 filtered` |
| `cargo test -p skiff-runtime-service-db prepared_runtime` | 11 | `11 passed; 0 failed; 102 filtered` |
| `cargo test -p skiff-runtime-capability-context prepared_db` | 8 | `8 passed; 0 failed; 52 filtered` |

八个主 selector共实际执行 `124/124`。eval selector附带的
`catch_fixture_closure` 与 `representation_wrap_consumer` integration binary均为0匹配，
不计作证据。

其它合同命令：

| 命令 | 结果 |
| --- | --- |
| `cargo check -p skiff-runtime-native -p skiff-runtime-eval -p skiff-runtime-service-db -p skiff-runtime-capability-context --locked` | PASS，四个直接选择 package及依赖全部完成，exit 0 |
| `cargo fmt --check` | PASS，exit 0 |
| `git diff --check` | PASS，exit 0 |

依任务合同没有重跑 O6R13 已在同一 production候选拥有的
`program_db::tests::`、`db_actor_` 或完整 eval gate。直接父结果记录的同候选证据为：

- `program_db::tests::` listing/execution `36/36`；
- `db_actor_` listing/execution `37/37`；
- 完整 eval unit/integration/doc合计 `339/339`。

本节点没有运行 full eval/native/service-db suite、MongoDB、stable、live或network。

## 6. Residual risk 与后继边界

- J1 focused tests证明 owner wait的借用、single-start、drop/cancel和finalizer合同，但 evaluator
  尚未把九处旧 pre-suspend换成 E3。因此 owner级 Ready/Pending 证据不能外推为 E4R
  Actor commit/release/reacquire集成已通过。
- `std.file.createFromStream` 同时组合 producer task、supervised consumption、native wait与
  Actor actual-Pending，是 E4R 最复杂的 call-site。O1 owner已提供所需 heap-free wait，
  但 E4R 仍须用其 stream/checkpoint/cancel matrix证明组合后不双重 drain或cancel。
- native paired finalizer之后还有 evaluator return-plan coercion，prepared find-many finalizer
  之后还有 array container allocation。E4R checkpoint gate必须覆盖整个 call-site组合，
  不能只依赖 owner内部 checkpoint。
- provider与callback都在 owned context中递归运行 evaluator，owner context明确不带 caller
  Actor frame；它们首次真实 Pending由外层 E3观察。nested concurrent、catch与replacement
  fence仍须由 E4R 集成测试验证。
- DB O6 matrix使用真实 evaluator/Actor frame但 deterministic fake store；本任务禁止
  MongoDB/live，因此没有重新证明真实 driver/session timing。O5R2记录的真实 Mongo与旧
  namespace fixture限制没有在J1绕过或修复。
- 本 PASS 只允许启动 E4R。E4R 必须删除 `native_call_suspends` 与全部 pre-suspend pair，
  接线 timeout/concurrent/catch/checkpoint/stream，并运行自己的完整 gate。I6仍未开始。
