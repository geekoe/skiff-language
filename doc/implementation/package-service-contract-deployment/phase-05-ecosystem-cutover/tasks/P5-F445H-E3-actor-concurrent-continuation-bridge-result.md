# P5-F445H-E3 Actor concurrent continuation bridge result

状态：`IMPLEMENTATION_COMPLETE / EVAL_GREEN / CORRECTION_GREEN`。

E3 已完成 Actor concurrent continuation bridge。outer 当前同步 segment 只提交一次；每个 lane
拥有独立 lease slot，并继续通过真实 `ActorInstanceStore` scheduler 串行化同步 segment。只有
future 首次真实返回 `Pending` 才释放 segment，因此多个外部异步操作可以同时 pending，ready
future 仍留在当前 segment 内。独立验收修正后，child continuation 也可以按同一合同嵌套 bridge；
每一层 gate 独立跟踪其直接 child，最外层 lane 在嵌套期间保持 open。

本节点没有修改 `actor_instance.rs`、`eval_context.rs` 或 Actor ABI，也没有引入
`maySuspend` 运行时分支、兼容路径或 fallback。

## 1. 输入与提交

| 项 | commit |
| --- | --- |
| production prerequisite | `648627fe` |
| task document | `141653da` |
| implementation | `bc4f3719` |
| independent acceptance correction | `d80271e7` |

implementation 写集精确为：

- `runtime/eval/src/actor_executor.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation/bridge.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests.rs`

既有 `ActorExecutionFrame` 从 1200 行以上的 root 移入 405 行 child module；291 行的 bridge
module 独立负责 lane ownership、active-child gate 与 RAII cleanup。root 只保留 module 声明、
E4 后继所需的 crate-private re-export 和 test module 声明。

## 2. Test-first 证据

先新增真实 store fixture 测试
`actor_concurrent_continuation_parent_suspends_once_and_children_are_independent`，再运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target \
  cargo test -p skiff-runtime-eval \
  actor_concurrent_continuation_parent_suspends_once_and_children_are_independent \
  -- --nocapture
```

结果为预期 RED，exit `101`：

- `ActorExecutionFrame::begin_concurrent` 不存在；
- `ActorExecutionFrame::has_execution_lease` 不存在。

该 RED 来自缺失的 concurrent continuation 合同，不来自旧 exhaustive match、mock scheduler
或写集外组件。随后才加入 production bridge，并扩展完整行为矩阵。

独立验收指出嵌套 bridge 被错误拒绝，以及已安装 lease 的 frame 调用 `resume` 会在同一 scheduler
上自等待。修正仍先加入两个 focused 测试：

```text
CARGO_TARGET_DIR=/tmp/skiff-p5-f445h-e3-actor-continuation-target \
  cargo test -p skiff-runtime-eval \
  actor_concurrent_continuation_nested_bridge_composes_both_gates_and_commits \
  -- --nocapture

CARGO_TARGET_DIR=/tmp/skiff-p5-f445h-e3-actor-continuation-target \
  cargo test -p skiff-runtime-eval \
  actor_concurrent_continuation_resume_with_installed_lease_fails_on_first_poll \
  -- --nocapture
```

两次均为预期 RED，exit `101`：

- 嵌套测试收到 `Actor child continuation cannot create a nested outer bridge`；
- installed-lease 测试第一次直接 poll 得到 `Pending`，触发
  `resume with an installed lease must fail before scheduler acquisition`。

第二个测试不使用 timeout；它只 poll 一次，直接证明失败必须发生在 scheduler acquisition 之前。
随后修正 production，并提交 `d80271e7`。

## 3. Frame、scheduler 与 actual-Pending

parent 与所有 child frame 共享一个不可变 continuation metadata `Arc`，其中包含 store、handle、
instance identity fence 和 linked field plans。每个 child 则新建自己的
`Mutex<Option<ActorInstanceExecutionLease>>`，不再共享 parent 的 execution slot。

`begin_concurrent` 的顺序固定为：

1. 安装 active-child gate；
2. 用 parent 当前 lease 和 outer heap clone 提交当前同步 segment 一次；
3. 为每个 lane 构造初始 suspended child frame；
4. 按 lane index 单次 claim lane handle。

child 在进入同步 Actor 代码前调用真实 store `acquire_execution`。store 原有 scheduler guard
继续保证同一 incarnation 任一时刻最多一个同步 segment。resume acquisition 使用
cancellation-safe permit；等待 acquire 的 future 被取消或报错时，lane 不会永久停在 acquiring
状态，也不会安装 lease。

`resume` 在 outer gate、budget 与 store acquire 之前先检查当前 lease slot。slot 已安装时稳定返回
`InvalidArtifact("Actor continuation attempted to resume while an execution token is already installed")`，
因此不会对自己持有的 scheduler guard 发起 acquire。acquire 后仍保留同一错误的竞态复查。

既有 `await_if_pending` poll-once 语义原样保留在 frame core：

- 第一次 poll 为 `Ready`：不 commit、不释放、不 reacquire；
- 第一次 poll 为 `Pending`：提交该 child segment，等待外部 future；
- future ready 后：先检查 execution budget/cancel，再 acquire scheduler、检查 instance fence，
  最后通过 field codec 导入最新 committed fields；
- 新 production 和测试中没有 `maySuspend`、`native_call_suspends` 或 preemptive suspend 分支。

## 4. Completion、drop 与 outer gate

`ActorConcurrentContinuationLane` 是 fail-safe ownership handle：

- `complete(heap)` 提交最后一个同步 segment并关闭 child；
- `abandon()` 或 handle drop 只 take/drop 当前 lease，回滚未提交 segment并释放 scheduler guard；
- suspended child 的 drop 不产生第二次 commit；
- bridge drop 会幂等 abandon 所有尚未完成或尚未 claim 的 child；
- child acquiring、holding、suspended、finished 四态都只使 remaining-child 计数归零一次。

outer 只能通过 gate 为零后的 `resume_parent` 重新进入 scheduler。仍有 child open 时立即返回稳定
`InvalidArtifact`，不会在 acquire await 中暗中与 child 共用 parent lease。全部 child 完成或放弃
后，outer 从 store 读取最新 committed fields；传入的 outer heap 不被替换，因此 continuation-local
heap handle 继续有效。

嵌套 bridge 复用相同状态机而不共享 gate：开始嵌套时，outer child 提交并释放它在直接 parent
gate 上的 active segment，但该 lane 的 remaining-child 计数保持 open；嵌套 lanes 只登记到新
gate。嵌套 parent 恢复时重新取得 outer child 的 segment，使直接 parent gate 从 0 个 held
segment 回到 1 个；outer child 最终完成后，最外层 gate 才归零。真实 store 测试逐步验证字段
`2 -> 3 -> 4 -> 5` 的提交可见性、两层 gate 的 `0/1` held-segment 计数、最终 outer restore，
并通过后续 linked Actor 调用证明没有 scheduler guard 泄漏。

replacement、stale epoch、cancel 和 deadline budget error 都沿既有错误类型返回。失败路径保持
child frame 无 lease，且 RAII cleanup 不泄漏 scheduler guard。

## 5. 自验收矩阵

| 任务合同 | production 证据 | 真实测试证据 |
| --- | --- | --- |
| parent 一次 suspend；child slot 独立 | `begin_concurrent`、shared metadata、per-frame lease mutex | `parent_suspends_once_and_children_are_independent` |
| 同步 segment 串行、外部 future 重叠 | store `acquire_execution` + child `suspend` | `serializes_segments_but_overlaps_pending_futures` 使用真实 scheduler、oneshot 与 poll tracker，观测同时 2 个 pending future |
| ready 不释放 | 复用 poll-once `await_if_pending` | `ready_future_keeps_the_current_segment` 证明第二 lane 仍被 scheduler 阻塞 |
| resume 前 budget/cancel/fence | resume budget tick、cancel token、identity fence | `cancel_and_budget_fail_without_reinstalling_leases`；`rejects_replacement_and_stale_epoch_without_a_lease` |
| child commit 后最新字段可见 | `complete` -> `commit_execution`；resume codec import | `commits_children_in_order_and_preserves_outer_heap` |
| error/cancel/drop cleanup | lane/bridge `Drop`、幂等 child `finish` | `error_and_drop_release_without_double_commit` |
| outer fail closed，结束后恢复 | remaining-child gate + `resume_parent` | error/drop test 和 winner test 都先/后验证 gate |
| winner 取消全部 child | `abandon` 与 bridge fail-safe cleanup | `winner_abandons_all_children_before_outer_resume` |
| outer local heap 保留 | resume 只 codec-import Actor fields | `commits_children_in_order_and_preserves_outer_heap` |
| child 可嵌套 bridge | 每层 frame 自有 `outer_gate`；child state 只更新直接 parent gate | `nested_bridge_composes_both_gates_and_commits` 验证两层 gate、字段提交、outer restore 与无 lease 泄漏 |
| installed lease fail closed | `resume` 在 gate/budget/store acquire 前检查 lease slot | `resume_with_installed_lease_fails_on_first_poll` 不使用 timeout，首 poll 即获稳定 `InvalidArtifact` |
| 单 continuation 不回归 | frame core 行为未改成 preemptive path | 既有 buffered、pending、stale、cancel tests 全部包含在 21/21 filter 中 |

## 6. 验证

首次实现的 Cargo 命令使用 worktree 内独立 target；独立验收修正使用另一个独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target
/tmp/skiff-p5-f445h-e3-actor-continuation-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval actor_concurrent_continuation -- --nocapture` | PASS：实际执行 10/10 unit tests；其它 test binary 为 0 个匹配测试，不计作证据 |
| `cargo test -p skiff-runtime-eval actor_executor::tests -- --nocapture` | PASS：实际执行 21/21 unit tests，其中 10 个 bridge tests、11 个既有 executor tests |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 linker dead-code、compiler-source unused import、ordinary test unused import 和
`service_error_channel.rs` unreachable-pattern warnings；本节点没有新增 warning 或既有失败。

反向搜索：

```text
rg 'may_suspend|maySuspend|native_call_suspends|suspend_actor_segment' \
  runtime/eval/src/actor_executor/actor_concurrent_continuation.rs \
  runtime/eval/src/actor_executor/actor_concurrent_continuation/bridge.rs \
  runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests.rs
```

结果为空。`git diff 141653da..d80271e7 -- runtime/eval/src/actor_instance.rs
runtime/eval/src/eval_context.rs` 同样为空，确认没有越过禁止写集。

## 7. E4 后继接口

E4 可直接消费以下 crate-private 流程，不需要复制 store acquire、identity fence、field codec 或
active-child gate：

1. `ActorExecutionFrame::begin_concurrent(&outer_heap, lane_count)`；
2. `bridge.lane(index)`；
3. `lane.resume(&mut lane_heap, &execution).await` 后把 `lane.frame().clone()` 安装进 lane context；
4. 正常路径 `lane.complete(lane_heap)`，error/cancel/winner 路径 `lane.abandon()` 或 drop；
5. 所有 lane 终结后 `bridge.resume_parent(&mut outer_heap, &execution).await`。

E4 仍独占 evaluator call site 与 preemptive suspend 迁移；E3 没有接 IR、lane scheduler、stream、
service/DB/native/interface await path，也没有运行 stable/live/network 验证。
