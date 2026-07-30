# P5-F445H-O6R13 DB/Actor combined acceptance result

状态：`PASS / O6_DB_ACTOR_ACCEPTANCE_CLOSED / E4R_J1_STILL_OPEN`。

本次独立只读验收锚定冻结代码候选
`ce01def6895cc54955cf8d9bb7c4ccf222c4b8fc`。命令实际执行时的验收合同 HEAD 为
`b1ec88f36f4ce143fcbb01dd21886c453a35c332`；`ce01def6..b1ec88f3` 只新增
`P5-F445H-O6R13-db-actor-combined-acceptance.md`，production、tests、Cargo 与 lockfile 均无差异。

结论只关闭 O6 DB/Actor combined acceptance，不关闭整个 F445H 或 Phase 05；后续 E4R/J1 仍未完成。

## 1. Verdict

- Verdict：`PASS`。
- Blocking issues：无。
- Non-blocking follow-up：`runtime/eval/src/program_db/tests/lease.rs` 为 1002 行，虽然仍是单一 lease
  矩阵 owner，并且已复用共享 fixture、metrics 与 gate helper，但 claim/read case 仍重复较多
  evaluator wiring。后续可在不复制 lifecycle/fake 的前提下提取 test-only invocation harness 或按行为组
  拆子模块。`transaction.rs` 为 874 行，但已有统一 source driver、phase trace helper 和行为 case
  driver，当前没有第二套机制；仅文件较长不构成 blocker。

## 2. Production 独立审计

### Actual-Pending 与单一 suspension owner

- `runtime/eval/src/program_db/wait.rs:9-23`：`await_operation` 在 Actor frame 存在时只调用 E3
  `frame.await_if_pending(...)`；非 Actor 分支只执行 `operation.await`。
- `runtime/eval/src/actor_executor/actor_concurrent_continuation.rs:275-293`：
  `await_if_pending` 先 poll 一次；first-Ready 直接返回，只有观察到 `Pending` 才
  `suspend`，原 future terminal 后再 `resume`，没有重建 operation。
- `runtime/eval/src/eval_context.rs:698-778`：`DbOperation`、`DbQuery`、`DbTransaction`、
  `DbLeaseClaim`、`DbLeaseRead` 五个 arm 均无外层 `suspend_actor_segment` /
  `resume_actor_segment`。同文件其它 owner 反向搜索仍有 9 个 suspend call 与 9 个 resume call，
  没有被顺手删除。

### Ordinary raw / prepared / query

- `runtime/eval/src/program_db.rs:461-779`：raw 分支各自把 store clone 与 owned input 移入一个
  `async move`，只把该 future 交给一次 `await_operation`。
- 五个 prepared 分支分别只调用一次 `prepare_*_runtime`，消费同一个
  `operation.into_wait()`，并且均在 `await_operation` 返回后才调用
  `finalizer.finalize(heap)`；caller heap 不被 pending wait 捕获。
- `runtime/eval/src/db_eval.rs:412-432` 的 query value 只构造 query/projection wire value并解码到
  heap，不 require store、不发 I/O。ordinary query 测试同时断言 DB context require 为零、phase
  trace 为空。
- 共享 scripted metrics 对 raw/prepared Ready、Pending、error 与 outer-drop 均验证
  `constructed == 1`；prepared Pending 时 finalizer 尚未构造，恢复后才进入 caller heap。

### Transaction lifecycle

- `runtime/eval/src/program_db.rs:159-207` 与 `:227-301`：legacy 与 explicit evaluator 都只通过
  `TransactionLifecycle::begin` 进入同一个 owner。
- `runtime/eval/src/program_db/transaction.rs:27-94`：
  - begin terminal 成功后才构造 owner，因此 begin error 不会选择 abort；
  - body error和非法 flow 只消费一次 `abort(self, ...)`；
  - commit 先标记 `CommitSelected`，只 poll 一次 commit；commit error 再选择一次 abort并返回原
    commit error；
  - `abort_selected` 只构造一次 abort future；
  - 没有 `Drop` implementation、spawn 或 detached terminal cleanup。
- 反向搜索确认 `begin_transaction`、`commit_transaction`、`abort_transaction` 在该 evaluator
  production 范围只出现在 `transaction.rs`；两个 evaluator 之外没有旁路 lifecycle。

### Lease lifecycle

- `runtime/eval/src/program_db.rs:333-456`：
  - claim `None` 在 binding、renew、Lost、Release 前直接返回；
  - binding 只在 claim success handle 到达后导入；
  - body normal/error/illegal-flow 都先消费同一个 `renew_owner.stop_and_join()`，再按
    `LeaseLost -> Release` 顺序执行；
  - Lost 优先于 Release error，Release error优先于原 body flow/error，符合当前 evaluator
    终态。
- `runtime/eval/src/program_db/lease.rs:20-85`：
  - production 区域只有一个 `tokio::spawn`、一个 `renew_lease` 构造点；
  - tick 后只 pin 一个 renew future；内层 `biased` select 把 stop/closed watch 放在 renew
    terminal 之前，normal stop 会离开作用域并 drop 同一 pending renew future；
  - `stop_and_join` 只 send stop并 await同一个 `JoinHandle`，没有 abort；
  - outer `Drop` 的唯一动作是对仍在 owner 内的同一 task 调用唯一 production
    `task.abort()`，没有 detach 或额外 cleanup task。
- Release 与 Read 都把单一 owned future交给 `await_operation`；outer drop 只丢弃该实例。
  matrix 的 late-gate 断言确认之后不再 poll、不重建、不物化晚到结果。

### API 与 fixture 边界

- 在 `program_db`、DB capability context 与 service-db 范围反向搜索
  `CancelError`、公开 cancel surface、cleanup acknowledgement 与 exactly-once 承诺均为零；
  lease renew failure只写 evaluator 内部 execution cancel flag。
- `runtime/eval/src/program_db/tests/fixture/` 及三个 child 反向计数：
  `EvalRuntimeProgram::new == 1`、`LinkedExecutable` literal `== 1`、
  `impl DbCapabilityStoreApi == 1`。
- ordinary/transaction/lease child 中 `TransactionLifecycle == 0`、
  `LeaseRenewOwner == 0`，没有复制 production lifecycle、linked program、executable 或 fake。
- 测试函数静态计数精确为 ordinary 12、transaction 9、lease 14；连同 fixture checkpoint 为
  `36`。

## 3. Selector 前置确认

均使用任务指定的独立 target dir。

```text
cargo test -p skiff-runtime-eval program_db::tests:: -- --list
unit: 36 tests, 0 benchmarks
integration catch_fixture_closure: 0
integration representation_wrap_consumer: 0
PASS

cargo test -p skiff-runtime-eval db_actor_ -- --list
unit: 37 tests, 0 benchmarks
  = fixture/ordinary/transaction/lease 36
  + production db_actor_lease_owner_drop_aborts_renew_task 1
integration catch_fixture_closure: 0
integration representation_wrap_consumer: 0
PASS
```

selector 在执行 workload 前已确认非零，并包含全部新增命名测试。

## 4. 合同命令与实际计数

以下六条合同命令均在验收合同 HEAD `b1ec88f3`（代码候选 `ce01def6`）执行一次：

| 命令 | 实际计数 / 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval program_db::tests:: -- --nocapture` | 主 unit `36 passed; 0 failed; 292 filtered out`；两个 integration binary 各 `0` 匹配（分别 `4`、`6` filtered）；PASS。 |
| `cargo test -p skiff-runtime-eval db_actor_ -- --nocapture` | 主 unit `37 passed; 0 failed; 291 filtered out`；两个 integration binary 各 `0` 匹配（分别 `4`、`6` filtered）；PASS。 |
| `cargo test -p skiff-runtime-eval --locked --no-fail-fast` | unit `328/328`；integration `4/4` 与 `6/6`；doc-test `1/1`；合计 `339/339`，无 ignored/failed；PASS。 |
| `cargo check -p skiff-runtime-eval -p skiff-runtime-service-db -p skiff-runtime-capability-context --locked` | 三个直接选择的 package及依赖全部 check 完成，exit `0`；PASS。 |
| `cargo fmt --check` | exit `0`，无输出；PASS。 |
| `git diff --check` | exit `0`，无输出；PASS。 |

命令集实际为 `2` 条 selector/list 前置确认加 `6/6` 条合同 gate。Cargo 输出仍包含仓库其它路径的
unused/dead-code/unreachable-pattern warning，但 O6 审计路径无新增 warning，且所有 gate 均为零退出。

依合同未重复 O6R12 单个精确 selector；完整 `skiff-runtime-eval` gate 只由本节点运行一次。未运行
stage gate、stable、live、network 或 MongoDB。

## 5. Residual risk

- matrix 使用真实 evaluator、Actor frame和单一 production lifecycle，但 DB store是确定性
  scripted fake；本验收按任务禁止连接 MongoDB/live，因此没有重新证明真实 driver/session fallback
  与 Mongo transaction/lease timing。
- oneshot/watch gate 覆盖 actual-Pending、stop race和 late sender，但不是长时间 scheduler stress；
  极端调度与真实 I/O cancellation latency 仍留给后续更高层 gate。
- 两个长矩阵文件仍有维护成本，尤其 lease evaluator wiring；当前共享 fixture与唯一 owner约束可防止
  语义分叉，但后续新增 case应避免再复制 harness。
- E4R/J1 尚未完成；因此本 PASS 不能外推为整个 F445H、Phase 05 或 ecosystem cutover PASS。
