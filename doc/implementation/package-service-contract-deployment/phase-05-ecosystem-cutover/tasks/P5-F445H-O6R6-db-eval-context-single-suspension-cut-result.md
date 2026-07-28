# P5-F445H-O6R6 DB EvalContext single-suspension cut result

状态：`PASS / SINGLE_SUSPENSION_OWNER / 2_GREEN_REGRESSIONS`。

production 与 GREEN 回归提交为 `cdc31e54`。移植后的 transaction RED 与 lease RED 分别在
`920a3614`、`ae97b3bb`；本节点只修改任务允许的两个测试文件、`eval_context.rs` 和本 result，
没有修改 program_db production、共享 fixture、Actor E3、其它 call site、Cargo 或 lockfile。

## 1. RED 复现

production 修改前分别运行两个精确 selector，均发现 1 个测试并按预期失败：

- `db_actor_transaction_explicit_body_actual_pending_releases_actor_segment`：
  `0 passed; 1 failed`，返回
  `InvalidArtifact("Actor continuation attempted to suspend without an execution token")`；
  phase 为 `[Begin, BodyCreate, Abort]`，actual-Pending body future 在 terminal 前被丢弃。
- `db_actor_lease_claim_pending_uses_one_actor_segment`：
  `0 passed; 1 failed`，返回相同的 execution-token 错误。

两条 RED 都从真实 evaluator expression 路径首 poll 到 actual-Pending，不是编译失败、零测试或外部
环境失败。

## 2. 单一 production 修正

`runtime/eval/src/eval_context.rs` 中以下五个 `LinkedExprIr` arm 不再在 DB evaluator 外层调用
`suspend_actor_segment` / `resume_actor_segment`：

- `DbOperation`
- `DbQuery`
- `DbTransaction`
- `DbLeaseClaim`
- `DbLeaseRead`

每个 arm 现在直接等待原有 `eval_program_db_*` 入口并原样映射结果。production diff 精确删除五对、
共 10 行 outer suspend/resume；没有复制 first-poll，没有直接调用 frame suspend/resume，也没有增加
特殊状态。文件中的 helper 与 emit、service、callback、Actor、native 等其它 owner 保持原样。

lease RED 在 production blocker 消失后到达 terminal，但原测试尾部误用了只适用于 first-Ready 的
`assert_completed_once`。测试文件内将其收敛为共享 fixture 已冻结的 actual-Pending 完成指标：

```text
constructed=1, polls=3, pending_returns=2, ready_returns=1,
dropped_before_terminal=0, dropped_after_terminal=1
```

没有修改 fixture 或扩展 lease/transaction 矩阵。

## 3. GREEN 与合同验证

在代码提交 `cdc31e54` 上执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_transaction_explicit_body_actual_pending_releases_actor_segment -- --nocapture
```

结果：`1 passed; 0 failed`。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_lease_claim_pending_uses_one_actor_segment -- --nocapture
```

结果：`1 passed; 0 failed`。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture
```

结果：`12 passed; 0 failed`，满足 ordinary 非零合同。

以下静态命令也通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo fmt --check
git diff --check
```

`cargo check` 仅报告仓库既有 warning。

## 4. 反向搜索与边界

反向检查五个 DB arm 的完整区间没有 `suspend_actor_segment` 或 `resume_actor_segment`。全文件搜索仍能
找到 helper 定义以及其它未迁移 owner 的 9 对调用，证明本节点没有触碰其它 call site。相对任务
checkpoint `60520c5f`，代码提交只改变：

```text
runtime/eval/src/eval_context.rs
runtime/eval/src/program_db/tests/lease.rs
runtime/eval/src/program_db/tests/transaction.rs
```

没有运行完整 eval/stage gate，没有启动或连接 stable/live/network/Mongo，也没有 merge、rebase 或
push。本节点只解除 O6R4/O6R5 继续实现矩阵的 production blocker，不代表两个矩阵或 J1 已经完成。
