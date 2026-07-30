# P5-F445H-O6R2 DB/Actor shared fixture checkpoint result

状态：`PASS / FIXTURE_API_FROZEN / F1_F2_F3_NOT_IMPLEMENTED`。

production prerequisite 为 `4fbe49e5`；implementation commit 为 `a58e2ed5`。本节点只修改
`skiff-runtime-eval` crate-local test module graph、共享 fixture 与唯一 smoke，没有修改
non-test evaluator、Actor E3、capability-context、service-db、driver tests、Cargo、manifest 或
lockfile。

## 1. 实现结果

- `program_db.rs` 只增加 `#[cfg(test)] mod tests;`。
- `program_db/tests/mod.rs` 声明 `fixture`、`ordinary`、`transaction`、`lease`；三个 child 文件只保留
  空 module 占位，没有提前实现 F1/F2/F3。
- fixture 按职责拆成：
  - `state.rs`：`DbPhase`、FIFO `Script<T>`、one-shot gate、probe、ordered event trace 与
    `OperationMetrics`；
  - `store.rs`：唯一 `FakeDbContext` / `FakeDbStore`、真实 scripted DB methods、prepared
    operation/finalizer 与 test lease hold；
  - `program.rs`：唯一 linked program/file/address/executable，以及 raw/prepared create、
    query、legacy/explicit transaction、claim/read 和最小 Actor declaration；
  - `actor.rs`：真实 `ActorInstanceStore` / `ActorExecutionFrame`、fake DB context 注入、
    current-task first-poll 与竞争 acquire helper。
- required DB trait methods中，矩阵所需 raw create、prepared create、begin/commit/abort、
  claim/renew/lease-lost/release/read 均走同一 `FakeDbState`；其它 required methods按 method name
  fail-fast。未覆盖的五个 prepared method继续使用 trait fail-closed default。
- prepared create 返回真实 `PreparedDbValueRuntimeOperation` 与 one-shot
  `DbRuntimeFinalizer`；旧 heap-borrowing runtime methods fail-fast，并由独立计数证明 smoke 中为零。

共享 API 可见性限制在 `crate::program_db::tests`，没有增加 `feature = "test-support"` surface。

## 2. 唯一 smoke 证据

selector：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    program_db::tests::fixture::db_actor_fixture_checkpoint -- --nocapture
```

最终结果：

```text
running 1 test
test program_db::tests::fixture::db_actor_fixture_checkpoint ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 292 filtered out
```

该测试通过真实 `Interpreter::eval_program_db_operation` 与注入
`ActorExecutionFrame` 的 `ProgramExecutionContext` 证明：

1. raw linked builder命中 fake `create`且 `RawCreate` constructed/poll/ready/drop各一次；
2. recoverable `Thread` linked builder命中 `prepare_create_runtime`，wait/finalizer各一次，
   `create_runtime == 0`；
3. frame持有 segment 时竞争 acquire 两次 first-poll 均为 `Pending`，`frame.finish(heap)` 后同一
   acquire future完成；
4. 固定 `ExecutableAddr`、`LinkedFileUnit` 与 executable 能被真实 DB evaluator解析；
5. 三个已触发 phase 的 metrics 都精确为
   `constructed=1, polls=1, pending=0, ready=1, drop-before=0, drop-after=1`，ordered phase trace为
   `RawCreate -> PreparedCreateWait -> PreparedCreateFinalize`。

smoke 没有直接调用 `wait::await_operation`、fake store method 或 Actor helper来替代 evaluator。

## 3. 真实 RED

在 implementation 前先加入指定 selector 下唯一测试，测试仍以
`panic!("RED: DB/Actor shared fixture is not implemented")` 表示缺失 acceptance fixture。相同命令
真实返回：

```text
running 1 test
test program_db::tests::fixture::db_actor_fixture_checkpoint ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 292 filtered out
```

失败位置为 `runtime/eval/src/program_db/tests/fixture.rs`，原因精确为共享 DB/Actor fixture 尚未实现；
不是编译失败、零测试或外部服务失败。

## 4. 静态检查

以下命令均在 implementation commit 内容上通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo fmt --check

git diff --check
```

`cargo check` 只有仓库既有 warning，没有本 fixture 新增 warning。反向检查确认：

- 新目录中恰好一个 `#[tokio::test]` / `db_actor_*` test；
- `program_db.rs` diff仅为 `#[cfg(test)] mod tests;`；
- 没有 MongoDB、network、stable/live、service-db 或 test-support 引用；
- 没有 Cargo/manifest/lockfile变更。

本节点没有运行完整阶段 gate，也没有连接 MongoDB、network、stable 或 live target。

## 5. 后续边界

fixture API至此冻结。后续 F1、F2、F3 分别只写
`ordinary.rs`、`transaction.rs`、`lease.rs`，复用本节点的唯一 state/store/linked/Actor fixture；
本结果不构成任何 child 行为矩阵的提前实现。
