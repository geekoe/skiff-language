# P5-F445H-O6 Evaluator DB actual-Pending state machines result

状态：`TASK_SCOPE_EXPANDED`。

本节点没有修改 production 或 tests。普通 DB operation、`DbQuery` 和 lease read 都可以在 O6
写集内接入 O5R2 prepared operation 与 E3 `await_if_pending`；但是 transaction 与 lease claim
缺少能够在 evaluator future 被 drop 后继续受控收束的 terminal owner。当前 capability/store
接口只暴露由调用方 poll 的 async commit/abort/release，且 concrete service-db 会在 terminal
provider wait 完成前移走 request-state ownership。O6 无法在不修改 capability-context /
service-db 或引入任务明确禁止的 detached cleanup task、阻塞 `Drop`、重试副作用的前提下证明
exactly-once abort/release。

## 1. 输入与停止状态

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md` |
| 直接父节点 | `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md` |
| 直接父节点 | `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md` |
| production prerequisite | `69ba325a` |
| task document / worktree base | `fdb2e6d9` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o6-eval-db` |
| branch | `codex/p5-f445h-o6-eval-db` |
| production / retained test 修改 | 无 |
| stable / live / network / real MongoDB | 未运行 |

停止原因精确命中任务的两个明示条件：

- transaction future drop/cancel 无法保证 abort exactly once；
- claim future drop/cancel 无法保证 renew terminal 与 lease release exactly once。

这不是 DB、transaction、lease、timeout、错误优先级或 Actor 语义的新选择，而是 terminal
resource ownership seam 缺失。

## 2. Transaction 阻塞

### 2.1 Capability 只有可被调用方丢弃的 async terminal

`runtime/capability-context/src/db.rs:645-648` 的 trait 形状是：

```rust
fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()>;
fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()>;
fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
```

`DbCapabilityStore` 在 `db.rs:914-923` 只直接 await 这些 future。接口没有：

- begin 后返回的 transaction ownership token；
- cancellation-safe one-shot commit/abort operation；
- evaluator drop 时可同步交接的 abort guard；
- request-scoped、可 join 的 cleanup supervisor；
- terminal completion acknowledgement。

O6 可以 clone `DbCapabilityStore` 并用 `async move` 构造 caller-heap-free wait，所以普通
actual-Pending 接线不是问题；问题是 evaluator future drop 会直接 drop 正在 poll 的 terminal
future，而 capability contract没有另一个 owner接管同一个 operation。

### 2.2 Concrete store 在 provider wait 前移走 transaction

`runtime/service-db/src/store.rs:84-119` 的 `commit_transaction` 在 terminal provider wait 前执行：

```rust
let (mut transaction, leases) = {
    let mut state = self.request_state.lock().await;
    (
        state.transaction.take().ok_or_else(missing_transaction_error)?,
        state.leases.clone(),
    )
};
```

随后才 await lease fence与 Mongo commit。若 future在其中一次真实 `Pending` 后被 drop，
transaction session 已不在 `request_state`；新的 `abort_transaction()` 无法取得它。让原 commit
future在后台继续会允许 evaluator drop 后仍 commit，违反本任务“begin成功后的 future drop必须
abort”；drop commit后再调 abort则没有 session可收束。

`runtime/service-db/src/store.rs:122-129` 的 abort也先
`state.transaction.take()`，再 await provider abort。若 abort future在 provider wait期间被
drop，重调 abort只会看到 `None`，既不能证明第一次 provider abort完成，也不能把同一个 future
继续交给现有 owner。依赖 Mongo session的隐式析构也不能满足 capability fake store可观察的
exactly-once abort合同。

因此 evaluator内增加多个 phase/bool/atomic 只能防止再次调用，不能保证已经启动的异步 terminal
完成；这不是 O6 局部状态枚举可以修复的问题。

## 3. Lease claim 阻塞

### 3.1 Lease handle不是 terminal owner

`runtime/capability-context/src/db.rs:858-876` 只提供 claim、renew、release和lease-lost async
方法。`DbCapabilityLeaseHandle` 在 `db.rs:1280-1294` 只是可 clone 的
`{ hold, value, ttl_ms }`，没有 `Drop` terminal、renew cancellation、release acknowledgement
或 cleanup owner。

concrete `ServiceDbStore::release_lease`
（`runtime/service-db/src/store.rs:656-660`）先 await provider release，成功后才从
`request_state.leases`删除 hold：

```rust
self.runtime.release_lease(hold).await?;
let mut state = self.request_state.lock().await;
state.leases.retain(|candidate| candidate != hold);
```

release future在 provider wait后或 state update前被 drop时，调用方无法判断 provider side effect
是否已发生。重建 release会重放 side effect；不重建则不能证明 request state与provider resource
已经收束。

### 3.2 当前 renew task证明缺少 drop owner

`runtime/eval/src/program_db.rs:351-381` 当前用裸 `tokio::spawn`启动 renew loop，正常路径只调用
`renew_task.abort()`且不 join。outer claim future若在 body期间被 drop，`JoinHandle`随 future
一起 drop并只会 detach，renew loop继续持有 store与hold；没有路径停止 renew或 release lease。

可以在 O6 中为 renew loop增加 cancellation carrier并在正常路径 stop/join，但 outer future的
`Drop` 不能 await join或release。让 renew task在 channel close后自行 release会变成没有
request-scoped join owner、没有 terminal deadline/ack的 detached cleanup task；这正是任务禁止
的“无界 detached task”，也无法证明 late renew与release terminal的顺序。

## 4. 为什么禁止的局部规避不能成立

| 方向 | 失败原因 |
| --- | --- |
| `Drop` 中 `tokio::spawn(abort/release)` | cleanup没有结构化 owner或join点，provider future可永久Pending；命中任务明示禁令 |
| `Drop` 中阻塞等待 | async store/provider可能Pending，会阻塞runtime线程且命中禁令 |
| drop后重建 commit/abort/release future | terminal side effect可能已经启动；commit已移走session，release结果未知；违反同一future与exactly-once |
| 让后台task从一开始执行所有terminal | task调度会把provider first-poll Ready伪装成response-channel Pending，破坏E3 Ready/Pending segment合同 |
| drop后继续原commit future | commit可能成功；任务要求begin成功后的 evaluator drop走abort |
| 只用atomic phase阻止第二次调用 | 能避免double call，不能让已drop的异步provider terminal完成 |
| 依赖Mongo session / TTL自然清理 | capability abstraction与fake store无此保证，且不是exactly-once abort/release |
| 给cleanup私自加timeout | 本任务禁止设计新的timeout/cancellation协议，仍不能决定未知provider terminal是否已生效 |

这些方向也无法同时满足“begin/commit/abort或claim/release各自经过 actual-Pending”与
“first Ready不切segment”。O6不能复制future executor或Actor scheduler来制造新的交接机制。

## 5. 推荐的最小前置修正

在重发 O6 前增加 capability-context / service-db lifecycle ownership checkpoint。该节点只补
terminal ownership，不改变语言或DB可观察语义：

1. capability-context提供显式、one-shot、owned transaction lifecycle owner；begin成功后，
   commit/abort中任一 terminal即使等待方被drop也由结构化request cleanup owner继续驱动，且能
   join/ack；
2. service-db在commit/abort真正terminal前保持session可达，定义并测试 terminal future
   cancellation safety，不能在可丢弃future中先 `take()` 后失去abort能力；
3. lease claim返回或关联显式lease lifecycle owner，统一拥有renew stop/join与exactly-once
   release；release pending/drop时保留同一个 operation与可观测completion；
4. cleanup owner必须有请求生命周期内的注册与收束点，不能由 evaluator `Drop` 裸 spawn；
5. capability fake store与concrete service-db都测试 begin/commit/abort及claim/renew/release在
   first-Ready、Pending、error、cancel和waiter-drop下的 start/poll/terminal/drop精确计数；
6. 保持现有 provider error、lease-lost优先级、checkpoint truncate和Actor E3 seam不变。

完成该 checkpoint 后，O6 可以只消费其 lifecycle owner，并继续在当前写集中实现：

- 唯一 heap-free DB wait runner；
- raw与O5R2 recoverable ordinary operation；
- 不释放的纯 `DbQuery`；
- 两种source形状共用的transaction evaluator phase owner；
- claim body/binding与renew orchestration；
- lease read；
- 真实Actor Ready/Pending矩阵。

## 6. Partial implementation 判定

普通 operation、`DbQuery`、lease read以及 transaction/claim的非drop happy path可以局部实现，
但保留这些改动会让任务同时存在“已切 actual-Pending”与“资源drop仍不安全”的半状态，且无法通过
合同要求的完整 test matrix。按照停止规则，本节点没有保留这类 partial production或tests，也
没有新增第二套状态机。

## 7. 验证

任务在 production/test修改前由静态owner审计命中强制停止条件，因此没有可运行的
RED→GREEN候选，也没有运行合同中的 Cargo suite。审计确认：

- O5R2六个 prepared runtime operation仍提供 caller-heap-free one-shot wait/finalizer；
- E3 `ActorExecutionFrame::await_if_pending`无需修改；
- `ProgramExecutionContext` / `ExecutionScope`没有DB terminal cleanup注册或join接口；
- service-db / capability-context没有 transaction或lease lifecycle `Drop` owner；
- O6允许写集之外至少需要 capability-context与service-db production改动。

最终只对本 result运行：

```text
git diff --check
```

未运行 stable、live、network、真实 MongoDB、merge、rebase或push。
