# P5-F445H-O6R1 DB/Actor fixture owner preflight result

状态：`EXECUTABLE / TEST_ONLY_WRITE_SET_FROZEN`。

production prerequisite `6ef1bf9f` 已存在。本次只读审计没有发现需要 production API、
capability-context、service-db、E3 scheduler 或 driver test-support 扩张的缺口。推荐按一个共享
fixture checkpoint 加 ordinary / transaction / lease 三个不重叠 child test 节点执行。

本节点没有修改 production 或 tests，没有运行测试、network、stable、live 或 MongoDB。

## 1. 推荐落点与精确写集

唯一推荐落点是 `skiff-runtime-eval` 的 `program_db` crate-local unit tests，不是
`runtime/driver/eval/tests/program_execution.rs`。

共享 checkpoint 写集：

- `runtime/eval/src/program_db.rs`：只增加 `#[cfg(test)] mod tests;`，不得改非 test 代码。
- `runtime/eval/src/program_db/tests/mod.rs`：一次性声明 `fixture`、`ordinary`、`transaction`、
  `lease` 四个 child module。
- `runtime/eval/src/program_db/tests/fixture.rs`：唯一共享 fake store、gate/probe、Actor frame、
  context、linked IR/executable builder owner。
- `runtime/eval/src/program_db/tests/{ordinary,transaction,lease}.rs`：checkpoint 只建立空 module
  文件，使 module graph 可编译；后续分别由三个 child owner 独占。

checkpoint 后的并行写集：

| 节点 | 唯一写入 |
| --- | --- |
| ordinary | `runtime/eval/src/program_db/tests/ordinary.rs` |
| transaction | `runtime/eval/src/program_db/tests/transaction.rs` |
| lease | `runtime/eval/src/program_db/tests/lease.rs` |

不得写 `actor_executor.rs`、`actor_executor/**`、ordinary `test_runtime.rs`、
capability-context prepared fake、driver eval tests、Cargo、manifest 或 lockfile。最终组合验收不写
代码。

`program_db.rs` 的唯一改动受 `cfg(test)` 保护；因此这是 test-only 写集，不改变 library artifact、
production API 或 feature=`test-support` surface。

## 2. 现有 fixture 的复用边界

### Actor fixture

现有 `actor_executor.rs::tests::{Fixture, fixture, context, execution_frame}` 是 inline private test
module。`program_db::tests` 是 sibling，不能引用这些 private items。把它们改成共享 API 会扩大到
E3 owner，且没有必要。

可以复用现有 production/test-only 可见 seam：

- `ActorInstanceStore`、activation/fence/handle 和 acquire/commit execution API；
- crate-private `ActorExecutionFrame::new`、`finish`；
- crate-private `ProgramExecutionContext::with_actor_execution_frame`；
- actor executor fixture 已验证过的最小 declaration、activation、field plan 构造模式。

不依赖 `ActorExecutionFrame::has_execution_lease`：该方法是
`actor_concurrent_continuation` 的 `#[cfg(test)] pub(super)`，只对
`crate::actor_executor` 可见，对 sibling `program_db` 不可见。DB fixture 用真实 store 的竞争
acquire 证明 segment 状态：

1. Ready 后竞争 acquire 的 first poll 仍为 `Pending`；
2. DB operation first `Pending` 后，gate 未释放时竞争 acquire 能完成；
3. 竞争 segment commit、DB gate 放行、evaluator 恢复后，第二个竞争 acquire 的 first poll 又为
   `Pending`；
4. `frame.finish(heap)` 后该 acquire 才完成。

配合 operation probe 只有一个 future、一个 pending transition，这形成
`held -> released -> held` 的单次切段证据，不复制 scheduler。

### ordinary test runtime

可以直接复用 `crate::actor_executor_test_runtime`。`lib.rs` 已在 `cfg(test)` 下把
`assembly_execution/ordinary/test_runtime.rs` 映射成 crate-visible module，其中
`runtime_factory`、execution/config/file/stream/websocket/effects/actor/outbound context 都是
`pub(crate)`。

其默认 context 使用 `DbCapabilityContext::unavailable()`，所以共享 fixture 只重建
`ProgramExecutionInput` 并注入 fake DB context，其余 capability 全部调用该 test runtime。不得修改
test runtime。

`OwnedProgramExecutionContext::capture` 不保留 `actor_execution_frame`，不能用它把 DB evaluator
spawn 成 `'static` task。测试应在当前 task 内 pin 真实 evaluator future、手工 first-poll、观察
gate 后 drop 或继续 await；这足以覆盖 pending/drop，不是 production seam 缺失。

### capability-context prepared fake

`runtime/capability-context/src/db/prepared_runtime_tests/fake_store/**` 只在
capability-context 自身 unit-test crate 内编译，items 还是 `pub(super)`；eval 依赖该 library 时
无法导入。该 fake 也没有 Actor/transaction/lease phase scripting。

不得为复用它而导出 test-support API或反转 capability-context -> eval 依赖。eval fixture 只复用
公开的 `DbCapabilityStore::new`、prepared operation/finalizer 类型和相同的 one-shot probe 模式。

### driver eval fixture

`runtime/driver/eval/tests/program_execution.rs` 的 linked IR helpers 和
`ProgramTestInvocation` 都是该 integration module private。其 DB 注入点被固定为 concrete
`Option<ServiceDbCapabilityFactory>`，而且外部 crate 无法构造 crate-private
`ActorExecutionFrame` 或调用 `with_actor_execution_frame`。因此不能承载本矩阵。

只把其中 `program_with_executables`、DB operation/query JSON builder 的形状作为构造依据；不修改
其 8777 行 owner，也不修改 `runtime/driver/eval/tests/mod.rs`。

## 3. Fake DB 的最小稳定形状

### Context

`FakeDbContext` 只实现 `DbCapabilityContextApi::require_store`：记录一次 target/reason 并返回同一个
`DbCapabilityStore` clone。该 trait 没有其它 method。

### Store state 与 probe

`fixture.rs` 冻结以下概念接口，child tests 只能配置和读取，不能各造 fake：

```text
DbPhase =
  RawCreate | PreparedCreateWait | PreparedCreateFinalize |
  Begin | BodyCreate | Commit | Abort |
  Claim | Renew | LeaseLost | Release | Read

Script<T> = first-Ready 或 gate 后 Ready，terminal 为 Ok(T) / Err(DbCapabilityError)

OperationMetrics =
  constructed + polls + pending_returns + ready_returns +
  dropped_before_terminal + dropped_after_terminal

FakeDbState =
  每 phase 的 FIFO script + 有序 DbEvent trace + phase metrics +
  prepared finalize count/drop + renew task first-poll/drop signal
```

每个 store method 调用时记录 `constructed/start`，返回拥有 `Arc<FakeDbState>`、owned output 和可选
oneshot gate 的 `Send + 'static` future。`poll`、terminal 与 Drop 都由同一个 future instance
计数。测试完成标准统一要求：

- 每个被选 phase `constructed == 1`；
- first-Ready 为 `pending_returns == 0`；
- actual-Pending 在 gate 放行前至少返回一次 `Pending`，但不构造第二个 future；
- success/error terminal 均 `ready_returns == 1`；
- pending drop 为 `dropped_before_terminal == 1`、`ready_returns == 0`；
- late sender不能物化结果、启动 terminal 或重建 operation；
- prepared finalizer只在 wait成功并恢复 Actor segment后调用一次。

### 真实实现的方法

为本验收真实 scripted 实现：

- transaction：`begin_transaction`、`commit_transaction`、`abort_transaction`；
- ordinary/body：raw `create`；
- prepared ordinary：`prepare_create_runtime`，返回真实
  `PreparedDbValueRuntimeOperation` 与 one-shot `DbRuntimeFinalizer`；
- lease：`claim_lease`、`renew_lease`、`lease_lost`、`release_lease`、`read_lease`。

`DbCapabilityLeaseHoldHandle` 只需一个 test id handle，实现 `as_any` 和同 id equality。

### fail-fast stub

trait 中所有其它无 default method只做带 method name 的 fail-fast stub，包括：

- raw find-many/find-one、insert-many、update/upsert/replace/delete/count/exists；
-旧的 `*_runtime` heap-borrowing methods；
- file record insert/find/delete。

未覆盖的五个 `prepare_*_runtime` 保留 trait 的 fail-closed default，并且 event trace 不得出现它们。
这样 prepared create若错误落到旧 runtime或其它 prepare入口会立即失败，不会生成一个假成功路径。

## 4. 最小 linked IR / executable owner

`fixture.rs` 是唯一 linked fixture owner。它构造一个 `EvalRuntimeProgram`、一个 service
`LinkedFileUnit` 和固定 `ExecutableAddr`，不经过 compiler，也不依赖 driver crate private helper。
稳定 builder 至少提供：

- raw `create` `DbOperationIr`：输入和 result 使用普通 wire/JSON type，必须命中
  `DbCapabilityStoreApi::create`；
- prepared `create` `DbOperationIr`：带一个最小 `Thread` DB object record/type-plan 与
  heap-attached value，checkpoint smoke 必须观察
  `prepare_create_runtime == 1`、旧 `create_runtime == 0`；
-纯 `DbQueryIr`；
- legacy `db.transaction(body-expression)` executable；
-显式 `DbTransactionIr`，body/result 可选择 success、DB error 与非法 flow；
- `DbLeaseClaimIr`，body 可选择 success、DB pending/error 与非法 flow；
- `DbLeaseReadIr`；
-最小 Actor declaration/fence/field plan，用来构造真实 store lease 和
  `ActorExecutionFrame`。

IR 统一用 typed struct 或 `serde_json::from_value` 在 fixture 内构造。ordinary/transaction/lease
child 不得自行复制 `LinkedFileUnit`、`RuntimeActivation`、address、Actor fence 或 DB target。

checkpoint 的一个 smoke test必须同时证明：

1. raw builder命中 raw `create`；
2. prepared builder命中 `prepare_create_runtime` 而不是 legacy runtime；
3. Actor frame可持有 segment、finish后竞争 acquire可继续；
4. linked address/file/executable可以被真实 evaluator解析。

若 checkpoint 不能满足这四项，停止后续 child；不能把 fake 降级为直接测试
`wait::await_operation`。

## 5. DAG、RED、selector 与完成标准

“RED”在当前 production checkpoint上的含义是 acceptance selector/真实入口证据缺失；每个新增测试
还必须能杀死对应的 restart、pre-suspend、错误 terminal 或 detached-renew mutant。只证明 helper
自身计数的测试不算 RED。

```text
6ef1bf9f
  -> F0 shared fixture checkpoint
       -> F1 ordinary/query tests ----+
       -> F2 transaction tests -------+-> F4 combined read-only acceptance
       -> F3 lease tests -------------+
```

### F0 shared fixture checkpoint

- 写集：共享 checkpoint 五个文件和三个空 child 文件，如第 1 节。
- 真 RED：当前没有可解析的 DB+Actor linked fixture，也没有任何测试证明 evaluator选择
  `prepare_create_runtime`。
- selector：
  `cargo test -p skiff-runtime-eval program_db::tests::fixture::db_actor_fixture_checkpoint -- --nocapture`
- 非零要求：恰好 1 个 smoke test。
- 完成：第 4 节四项全部通过；fixture API冻结，F1/F2/F3不得再改 `fixture.rs`。

### F1 ordinary/query

唯一文件内冻结至少 9 个 `db_actor_ordinary_*` test：

1. query Ready不触碰 store，竞争 acquire仍被当前 segment阻塞；
2. raw create first-Ready start一次且不切 segment；
3. raw create actual-Pending释放一次、恢复后才 decode/materialize；
4. raw first-Ready error与pending-then-error均不重建；
5. raw pending drop销毁同一 future且无 terminal/result；
6. prepared create first-Ready wait/finalizer各一次且不切 segment；
7. prepared actual-Pending期间无 finalize，恢复后结果才进入 caller heap；
8. prepared wait Ready-error、pending-then-error和finalizer error均不重放；
9. prepared pending drop不 finalize、不重建。

其中 pending success用真实竞争 Actor segment证明 `held -> released -> held`；同时比较 finalize前后
heap checkpoint/stats/result visibility。

- 真 RED：删除 `await_if_pending`、把 Ready预先 suspend、改回 heap-borrowing runtime、重复构造
  future、在wait前finalize或error后retry，至少一项必失败。
- selector：
  `cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture`
- 非零要求：至少 9 个 tests。
- 完成：所有 phase metrics、Actor竞争 segment、heap/result assertion同时满足。

### F2 transaction

唯一文件内冻结至少 7 个 `db_actor_transaction_*` test：

1. legacy与显式 source的Ready success trace都严格为 begin/body-create/commit，无 abort；
2. 两个 source分别把 begin/body/commit/abort 设为唯一 Pending phase，逐案证明只切一次且 phase
   不重启；
3. begin Ready-error与pending-then-error均不调用 abort；
4. body error与显式非法 flow只调用一次 abort并保持原错误；
5. commit error只调用一次 commit、随后一次 abort，返回 commit error；
6. commit actual-Pending后drop：commit constructed/drop各一次，terminal/abort/finalize为零；
7. body actual-Pending后drop：commit/abort均为零。

参数化 source/phase可以放在一个 Rust test function 内，但每个 case必须打印 source/phase并独立断言
event trace与metrics。

- 真 RED：任一 source绕开 `TransactionLifecycle`、begin失败abort、commit retry、commit失败不abort、
  drop改选terminal或 phase Pending pre-suspend，至少一项必失败。
- selector：
  `cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture`
- 非零要求：至少 7 个 tests。
- 完成：两种 source全覆盖；正常/error/drop trace与 checkpoint rollback assertion一致。

### F3 lease

唯一文件内冻结至少 8 个 `db_actor_lease_*` test：

1. claim None的Ready与Pending都不导入binding、不renew、不release；
2. claim success的Ready与Pending只在成功后导入binding，claim只启动一次；
3. normal success、body error、显式非法 flow都按
   stop/join renew -> lease_lost -> release 收尾；
4. lease_lost Ready/Pending与release error保持一次调用和既有可见优先级；
5. body DB Pending期间drop会使真实 renew future被 abort/drop，之后poll计数不再增长，release允许为
   零；
6. release actual-Pending期间drop不重建release、无 late terminal；
7. lease read覆盖Ready、Pending、None、store error与小 heap budget触发的decode error；
8. lease read actual-Pending drop不重建、不 materialize。

renew fake在 first poll发送信号后保持 Pending；normal stop carrier与 outer Drop都必须使其 Drop
metric增加，前者还必须让 claim future正常 join完成。decode error使用返回 object/array加受限
`RequestHeapLimits`，不伪造 store error代替 decode。

- 真 RED：claim前导入binding、renew detach、正常路径不join、lost/release/read绕开 actual-Pending、
  drop重建release/read或 pending后pre-suspend，至少一项必失败。
- selector：
  `cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture`
- 非零要求：至少 8 个 tests。
- 完成：claim/read/lost/release/renew每个实际方法都有非零 constructed/poll证据；所有禁止 phase为零。

### F4 combined acceptance

只读 gate owner在三个 child commit集成后执行并记录实际测试数：

```bash
cargo test -p skiff-runtime-eval program_db::tests:: -- --nocapture
cargo test -p skiff-runtime-eval db_actor_ -- --nocapture
```

第一个 selector至少 25 个 tests；第二个必须包含全部新增 `db_actor_*` tests且非零。随后按实现任务已有
owner决定是否运行完整 `skiff-runtime-eval` gate；F1/F2/F3不各自重复昂贵完整 gate。

## 6. 对 O6R“全矩阵”的有界化

以下条款不能继续用“全矩阵”表述：

- ordinary不是所有 raw DB method乘Ready/Pending/error/drop；代表 raw `create`覆盖 owned input、
  provider wait与decode，prepared `create`覆盖O5R2 wait/finalizer。六个 prepared method的
  provider-specific mapping仍由 capability-context/service-db既有 owner负责。
- transaction不是两个 source乘所有 body flow乘四个 phase乘Ready/Pending/error/drop的笛卡尔积；
  两 source都跑 success、phase-Pending、begin/body/commit error和drop，非法 flow只需显式 source。
- `abort_transaction`没有错误返回值，不能要求“abort error”；只验Ready/Pending、一次选择与drop。
- lease renew由周期 task驱动，不能要求精确 tick 数；只冻结first-poll、normal stop/join、
  outer-drop abort和drop后poll不增长。
- “Pending只切一次”不增加 E3计数 API；用真实竞争 acquire和单一 scripted future证明
  `held -> released -> held`。
- Actor identity/replacement fail-closed属于现有 actor_executor tests，不在 DB child复制；F4只确认
  未改 E3文件，阶段 owner可引用其既有 selector证据。
- transaction共用 lifecycle core不能只靠黑盒证明；测试要求两 source trace等价，接收时再反向搜索
  两入口仍构造同一个 `TransactionLifecycle`。
-异常 outer drop不要求 transaction abort acknowledgement或 lease release exactly-once；分别接受
  service-db fallback与TTL fallback，精确按D1/O6R合同断言零 detached cleanup。

## 7. 范围判定

判定：不返回 `TASK_SCOPE_EXPANDED`，也不返回 `TASK_NOT_EXECUTABLE`。

唯一必须碰到的 production-path 文件是 `program_db.rs` 中受 `cfg(test)` 保护的 module声明；其余均是
新 test files。现有 `DbCapabilityContext::new`、`DbCapabilityStore::new`、prepared
operation/finalizer、Actor store/frame/context crate-private API和 ordinary test runtime已经组成
完整 seam。没有缺少的 production/test-support API。

后续如果实现者发现必须修改 `actor_executor` 可见性、把 Actor frame加入
`OwnedProgramExecutionContext`、导出 capability-context fake、修改 driver invocation DB factory，
或修改任何 non-test evaluator code，应立即返回 `TASK_SCOPE_EXPANDED`；这些都不是本审计冻结方案的
前置条件。
