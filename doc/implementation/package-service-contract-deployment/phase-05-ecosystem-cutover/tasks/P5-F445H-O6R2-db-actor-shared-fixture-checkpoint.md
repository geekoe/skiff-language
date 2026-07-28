# P5-F445H-O6R2 DB/Actor shared fixture checkpoint

状态：Ready。O6R1已经证明缺失矩阵可完全留在`skiff-runtime-eval` test-only写集，并冻结
“共享fixture checkpoint → ordinary / transaction / lease”扇出。本节点只落共享fixture和一个真实
smoke test，不实现三个child矩阵。

## 直接父节点

- `P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md`

production prerequisite为integration commit `4fbe49e5`。

父result §1—§4、§5 F0与§7完整定义owner、可见性、fake形状和停止条件。本任务文件只补执行边界；
不得改写其测试语义或从顶层设计自行扩张。

## 目标与唯一写集

只允许：

- `runtime/eval/src/program_db.rs`：只增加`#[cfg(test)] mod tests;`
- `runtime/eval/src/program_db/tests/mod.rs`
- `runtime/eval/src/program_db/tests/fixture.rs`
- `runtime/eval/src/program_db/tests/fixture/**`：仅当单文件职责或长度需要拆分
- `runtime/eval/src/program_db/tests/ordinary.rs`
- `runtime/eval/src/program_db/tests/transaction.rs`
- `runtime/eval/src/program_db/tests/lease.rs`
- 本result

三个child文件在本节点只建立可编译空module；不得提前写F1/F2/F3测试。不得修改任何non-test
production、Actor E3、ordinary test runtime、capability-context、service-db、driver tests、Cargo、
manifest或lockfile。

共享fixture必须一次性冻结：

1. `FakeDbContext`：返回同一个`DbCapabilityStore`；
2. `DbPhase`、FIFO `Script<T>`、`OperationMetrics`、有序event trace与gate/probe；
3. 单个`FakeDbState`，由后续child配置并读取，不允许child各造fake；
4. fake store真实实现raw create、prepared create、begin/commit/abort、
   claim/renew/lease-lost/release/read；其余required trait method以method名fail-fast；
5. prepared create返回真实`PreparedDbValueRuntimeOperation`与one-shot
   `DbRuntimeFinalizer`，旧heap-borrowingruntime method必须fail-fast；
6. test id lease hold handle；
7. 唯一linked program/file/address/IR builder，包括raw/prepared create、query、两种transaction、
   claim/read和最小Actor declaration；
8. 使用`crate::actor_executor_test_runtime`构造其余capability，并注入fake DB context；
9. 使用真实`ActorInstanceStore`、`ActorExecutionFrame::new`和
   `ProgramExecutionContext::with_actor_execution_frame`；
10. 提供后续child可复用的current-task first-poll/gate/drop与竞争Actor acquire helper，不复制E3
    scheduler。

若`fixture.rs`同时承担scripted future、store trait、IR builder和Actor fixture而明显过长，应按上述
职责拆进`tests/fixture/**`；root只re-export给三个child使用。所有共享API为test-only
`pub(super)`或更窄，不增加feature=`test-support`表面。

## Smoke test

先写真实RED，再完成fixture。Selector中恰好一个测试：

```text
program_db::tests::fixture::db_actor_fixture_checkpoint
```

该测试必须同时经过真实evaluator/Actor frame并证明：

1. raw builder命中fake raw `create`且只构造一次；
2. prepared builder命中`prepare_create_runtime`、wait/finalizer各一次，旧`create_runtime`为零；
3. Actor frame持有segment时竞争acquire first poll为`Pending`，`finish`后才能完成；
4. linked address/file/executable可由真实DB evaluator解析；
5. 所有已触发phase满足constructed/poll/ready/drop计数不变量。

不得把smoke降级为直接调用`wait::await_operation`、fake store method或Actor helper。

验证owner：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    program_db::tests::fixture::db_actor_fixture_checkpoint -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r2-fixture/build/cargo-target \
  cargo fmt --check
git diff --check
```

必须记录RED、最终`1/1`非零测试和静态检查。不得连接MongoDB、运行stable/live/network或完整阶段
gate。

## 停止条件

若需要修改Actor可见性、`OwnedProgramExecutionContext`、capability-context fake导出、driver DB
factory、任何non-test evaluator代码或Cargo依赖，立即返回`TASK_SCOPE_EXPANDED`。若五分钟内无法形成
第一处实际测试修改，返回`TASK_NOT_EXECUTABLE`。

风险：中（共享test checkpoint）。完成后只是fixture接口检查点；F1/F2/F3才拥有行为矩阵，后续不得
再修改本fixture，除非其直接机械缺陷阻止child编译。

## Worktree与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r2-fixture
branch   codex/p5-f445h-o6r2-fixture
```

先提交fixture/smoke implementation，再单独提交
`P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`。Worktree clean；不得merge、rebase或push。

这是新的、一次性开发Agent会话。当前共享文件高度耦合，不派子Agent。完成后不得自行继续F1/F2/F3。

