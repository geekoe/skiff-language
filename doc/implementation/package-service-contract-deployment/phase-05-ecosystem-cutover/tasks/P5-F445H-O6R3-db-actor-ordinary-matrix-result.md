# P5-F445H-O6R3 DB/Actor ordinary matrix result

状态：`PASS / 12_TESTS / ORDINARY_ONLY`。

直接父节点为
`P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`；production prerequisite 为
`ceb73fbc`，tests implementation commit 为 `c47c3ea4`。本节点只修改
`runtime/eval/src/program_db/tests/ordinary.rs` 与本 result，没有修改冻结 fixture、production、
transaction/lease child、Cargo 或 lockfile。

## 1. 实现矩阵

本节点新增 12 个非零 `db_actor_ordinary_*` selector，全部通过真实
`Interpreter::eval_program_db_operation` 或纯 query 的
`Interpreter::eval_program_db_query_value` 进入 evaluator；没有直接调用
`wait::await_operation`、fake store method 或第二套 fake。

| 合同 | selector 与证据 |
| --- | --- |
| query Ready 不触碰 store、segment 仍持有 | `db_actor_ordinary_query_ready_keeps_segment_and_does_not_touch_store`；结果为 caller-heap value，`context_require_calls == 0`、phase trace 为空，竞争 acquire 为 `Pending`。 |
| raw first-Ready 只启动一次且不切 segment | `db_actor_ordinary_raw_create_ready_once_keeps_segment`；`RawCreate` 完成一次、DB context require 一次、竞争 acquire 为 `Pending`。 |
| raw Pending 释放、恢复后 materialize | `db_actor_ordinary_raw_create_pending_releases_and_reacquires_segment`；同一竞争 acquire 依次证明 `Pending -> Ready`，恢复后新竞争 acquire 再次为 `Pending`；suspended snapshot heap 小于返回后的 caller heap。 |
| raw Ready-error / pending-error 不重建 | `db_actor_ordinary_raw_create_ready_error_is_not_rebuilt` 与 `db_actor_ordinary_raw_create_pending_error_is_not_rebuilt`；两者 `constructed == 1`，无重放。 |
| raw Pending drop | `db_actor_ordinary_raw_create_pending_drop_drops_only_same_future`；gate 未释放，唯一 future `DropBeforeTerminal == 1`、`Ready == 0`。 |
| prepared first-Ready wait/finalizer 各一次、不切 segment | `db_actor_ordinary_prepared_create_ready_wait_and_finalizer_once`；wait/finalizer 各完成一次、legacy runtime call 为零、竞争 acquire 为 `Pending`。 |
| prepared Pending 恢复后才 finalize/materialize | `db_actor_ordinary_prepared_create_pending_finalizes_only_after_resume`；Pending 时 finalizer metrics 全零；同一竞争 acquire 证明 segment 释放，恢复后 finalizer 才运行且 caller heap 才增长，最后 segment 再次持有。 |
| prepared 三种 error 不重放 | `db_actor_ordinary_prepared_wait_ready_error_is_not_replayed`、`db_actor_ordinary_prepared_wait_pending_error_is_not_replayed`、`db_actor_ordinary_prepared_finalizer_error_is_not_replayed`；wait/finalizer各自至多 constructed 一次。 |
| prepared Pending drop | `db_actor_ordinary_prepared_pending_drop_does_not_finalize_or_rebuild`；wait future 只 drop 同一实例，finalizer metrics 全零，gate 未释放。 |

Pending drop case 在外层 evaluator future 被取消后保留 Actor continuation 的 suspended 终态，并由
fixture teardown 回收；测试不伪造 terminal/result，也不把已取消 continuation 误报为 method
completion。

## 2. 精确 operation 计数

冻结 fixture 的真实计数如下：

- first-Ready success/error：
  `constructed=1, polls=1, pending=0, ready=1, drop-before=0, drop-after=1`；
- Pending success/error：
  `constructed=1, polls=3, pending=2, ready=1, drop-before=0, drop-after=1`；
- Pending drop：
  `constructed=1, polls=2, pending=2, ready=0, drop-before=1, drop-after=0`；
- prepared finalizer 被调用时：
  `constructed=1, polls=1, pending=0, ready=1, drop-before=0, drop-after=1`；
- prepared wait error 或 drop 时，finalizer所有计数均为零。

Pending 路径多出的 poll 是 Actor continuation 的 first-poll/suspend state machine 对同一个
scripted future 的推进；所有 case 的 `constructed == 1` 证明 operation 没有重建。

## 3. 聚焦测试

执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture
```

结果：

```text
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 293 filtered out
```

selector 实际测试函数为 12，满足不少于 9 且非零的合同。后续两个 integration test binary 在同一
Cargo 命令中各运行零个匹配测试，不计入本 selector 的 12 个 unit tests。

## 4. 静态检查与反向审计

以下命令均通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo fmt --check

git diff --check
```

`cargo check` 仅报告仓库既有 warning。反向审计确认：

- `ordinary.rs` 中恰好 12 个 `async fn db_actor_ordinary_*`；
- tests commit 只修改 `runtime/eval/src/program_db/tests/ordinary.rs`；
- 没有 `await_operation`、Mongo、network、stable/live URL 或 service-db 引用；
- 没有修改 fixture、production、transaction、lease、Cargo、manifest 或 lockfile；
- 未运行完整 eval/stage gate，未连接 stable/live/network/Mongo。

冻结 fixture 足以完成矩阵，未发现 fixture API 缺口，也未暴露需要越界修复的 production failure。
