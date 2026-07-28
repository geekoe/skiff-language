# P5-F445H-O6R4 DB/Actor transaction matrix result

状态：`FAIL / PRODUCTION_BLOCKER / MATRIX_NOT_COMPLETED`。

production prerequisite `ceb73fbc` 已存在。tests commit 为 `94359d15`。本节点严格只修改
`runtime/eval/src/program_db/tests/transaction.rs` 与本 result；没有修改冻结 fixture、production、
Cargo 或 lockfile。

## 1. 最小失败证据

在冻结 fixture 上保留了一个真实 evaluator 回归测试：

```text
db_actor_transaction_explicit_body_actual_pending_releases_actor_segment
```

该测试使用共享 explicit transaction builder、真实 `ActorExecutionFrame`、真实
`Interpreter::eval_program_explicit_db_transaction` 入口和 scripted body `create`。begin 为
first-Ready，body create 为唯一 actual-Pending phase。按合同，evaluator 首次 poll 应返回
`Pending`，释放 Actor segment，并保留同一个 body future。

实际 selector：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
```

实际发现 1 个测试并失败：

```text
running 1 test
an actual-Pending body DB operation must suspend the transaction evaluator;
got Err(InvalidArtifact("Actor continuation attempted to suspend without an execution token"));
phases=[Begin, BodyCreate, Abort];
body=OperationMetrics {
  constructed: 1, polls: 1, pending_returns: 1, ready_returns: 0,
  dropped_before_terminal: 1, dropped_after_terminal: 0
};
abort=OperationMetrics {
  constructed: 1, polls: 1, pending_returns: 0, ready_returns: 1,
  dropped_before_terminal: 0, dropped_after_terminal: 1
}
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 293 filtered out
```

这不是零测试、fixture 缺 phase、外部服务或编译失败。

## 2. production blocker

失败路径是：

1. explicit transaction evaluator 在 result expression 中进入真实 linked DB operation；
2. `EvalContext` 的 `LinkedExprIr::DbOperation` arm 先调用 `suspend_actor_segment`；
3. body create 首次返回真实 `Pending` 后，`wait::await_operation` 再通过同一个 Actor frame 调用
   `await_if_pending`；
4. 第二次 `suspend` 找不到 execution token，返回
   `InvalidArtifact("Actor continuation attempted to suspend without an execution token")`；
5. transaction 把该内部 Actor 错误当作 body error，选择一次 abort，并丢弃尚未 terminal 的原 body
   future。

因此 frozen F2 的“body actual-Pending 释放一次、保持 Pending、不得 abort”在当前 production 上不可
实现。修复需要修改 `eval_context.rs`、`program_db` wait/entry 接线或 Actor continuation production
语义，均超出本节点唯一写集。

## 3. 合同收束

按任务“测试若发现实现错误，保留最小失败证据并返回 FAIL”的要求，本节点在首个确定 production
blocker 后停止，没有：

- 扩写其余 legacy/explicit success、error、commit/abort/drop 矩阵；
- 修改或绕开共享 fixture；
- 直接调用 `TransactionLifecycle` 伪造通过；
- 修改 production 修复错误；
- 运行完整 gate、stable/live/network/Mongo；
- merge、rebase 或 push。

因此当前实际 `db_actor_transaction_*` 数为 1，低于完成态要求的至少 7；这是
`MATRIX_NOT_COMPLETED` 的明确证据，不得将本节点接收为 PASS。

## 4. 其它验证

以下静态命令在 tests commit 上通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo fmt --check

git diff --check
```

`cargo check` 只有仓库既有 warning。聚焦 selector 按预期保持 RED，不能列为通过验证。
