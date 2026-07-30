# P5-F445H-O6R Evaluator DB internal-stop state machines result

状态：`IMPLEMENTED_PENDING_ACCEPTANCE_FIXTURE`。

implementation commit：`89406b14`。

本节点在 evaluator DB 写集内完成 production 接线，但没有完成任务要求的真实
`eval_program_db_*` + `ActorExecutionFrame` + fake store 全矩阵 fixture。因此实现可供 J1
继续审查，当前证据不能把本节点宣称为完整 GREEN。

## 实现摘要

- `program_db/wait.rs` 增加唯一 caller-heap-free DB wait runner。future 必须为
  `Send + 'static`；Actor context 只调用既有 `ActorExecutionFrame::await_if_pending`，非 Actor
  context等待同一个 future。
- raw ordinary DB operation在 wait 前拥有 store、type、selector/query/order/projection、
  document/change等输入，wait 后才 decode 到 caller heap。
- recoverable find-many、两种 find-one、create、update-one、replace-one全部改用 O5R2
  `prepare_*_runtime` one-shot wait/finalizer；删除 evaluator 对六条借 heap runtime async 入口的
  调用。
- legacy `db.transaction(...)` 与显式 `DbTransactionIr` 共用
  `TransactionLifecycle`。显式 phase固定 begin/body/commit-selected/abort-selected/complete；
  begin失败不 abort，body/flow error abort，commit error abort且不重试 commit，正常 commit不
  abort。Drop不 spawn、不阻塞、不另选 terminal。
- lease claim/read/lease-lost/release均改为 caller-heap-free actual-Pending wait。claim成功后才
  import binding并启动 renew owner；正常路径 stop/join renew后再读 lease-lost并等待 release。
- `LeaseRenewOwner::Drop`同步调用 `JoinHandle::abort()`；正常 terminal通过 stop carrier join同一
  task，outer drop不会 detach renew。异常 drop不启动 release cleanup，保留 TTL fallback。
- `DbQuery`保持直接调用 `DbIrEvaluator::eval_query_value`，没有包进 external wait。

没有修改 capability-context、service-db、Actor E3、request owner、compiler、artifact、router、
manifest或 lockfile；没有增加 compatibility、timeout、cleanup supervisor、detached
abort/release或新的 DB 语义。

## 验证

全部命令只使用 worktree build target；未连接 MongoDB，未运行 stable/live，未访问网络。

| 命令 | 结果 | 实际测试数 |
| --- | --- | ---: |
| `cargo test -p skiff-runtime-eval program_db -- --nocapture` | PASS | 3 |
| `cargo test -p skiff-runtime-eval db_actor -- --nocapture` | PASS | 1 |
| `cargo test -p skiff-runtime-eval --locked --no-fail-fast` | PASS | 292 lib + 10 integration + 1 doc |
| `cargo check -p skiff-runtime-eval -p skiff-runtime-service-db -p skiff-runtime-capability-context --locked` | PASS | 不适用 |
| `cargo fmt --check` | PASS | 不适用 |
| `git diff --check` | PASS | 不适用 |

输出只有既有 workspace warning：linker dead code、eval unreachable baseline及 test-only unused
import；无新增编译错误或测试失败。

## 自验收矩阵

| 任务条款 | 代码证据 | 反向搜索 / 测试证据 | 判定 |
| --- | --- | --- | --- |
| 唯一 actual-Pending runner | `program_db/wait.rs::await_operation` | runner唯一调用 `await_if_pending`；future有 `Send + 'static`约束 | PASS |
| raw ordinary operation只启动一次、恢复后 decode | `execute_db_command`的所有 raw arm均构造 owned `async move`交给 runner | direct store await只存在 runner closure内部；完整 eval suite PASS | PASS |
| 六条 recoverable prepared入口 | find-many、find-one-key/query、create、update、replace arm | evaluator中六条借 heap `*_runtime(...).await`反向搜索为零 | PASS |
| finalizer在 resume 后执行 | prepared wait先经 runner，随后 `finalizer.finalize(heap)` | `program_db` selector PASS | PASS |
| `DbQuery`不包装 wait | `eval_program_db_query_value`保持直接转发 | 该函数 diff无 external wait | PASS |
| 两种 transaction source共用 lifecycle | 两个入口都消费 `TransactionLifecycle` | begin/commit/abort只有 lifecycle owner调用；无 Drop cleanup | PASS（缺专项fixture） |
| transaction显式 terminal phase | `TransactionPhase` | 无松散 terminal bool、无 commit/abort restart | PASS（缺专项fixture） |
| lease renew不 detach | `LeaseRenewOwner`及其 `Drop` | drop-abort、normal stop/join测试 PASS | PASS |
| lease claim/read/release actual-Pending | claim/read/lost/release owned closure均经 runner | direct evaluator await为零；完整 eval suite PASS | PASS（缺 store/Actor矩阵） |
| drop不 detached cleanup | transaction Drop为空；lease Drop只 abort renew | 无 spawn abort/release、无阻塞 Drop | PASS |
| 不复制 E3 / 不 pre-suspend | runner只调用既有 frame API | 写集中无 scheduler acquire/release、`suspend`、`resume`、`yield_now`、unsafe heap alias或 heap mutex | PASS |
| `program_db` / `db_actor`真实入口矩阵 | 只新增 renew owner生命周期测试 | 两个 selector非零且通过，但未穿过真实 DB evaluator/store Actor fixture | **MISSING** |

## 未决问题

唯一未决项是任务合同要求的专项 acceptance fixture：尚未新增覆盖 Ready/Pending/error/drop 的 raw、
recoverable、transaction、claim/read/release矩阵，也未以 fake store穿过真实
`eval_program_db_*`入口和真实 `ActorExecutionFrame`计数。现有完整 eval suite证明没有回归，
但不能替代该矩阵。J1在接收 implementation commit前应补齐或要求补发该 fixture；在此之前本结果
不声称解除 J1。

worktree没有 merge、rebase或 push。
