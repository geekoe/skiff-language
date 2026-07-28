# P5-F445H-O6R7 DB/Actor transaction matrix resume

状态：Ready。O6R6 已消除真实 transaction body actual-Pending 路径的双重挂起，
并保留了一条通过的最小回归。本节点从该检查点继续完成 transaction 行为矩阵；不再修改 production
或共享 fixture。

## 直接父节点

- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`
- `P5-F445H-O6R4-db-actor-transaction-matrix-result.md`
- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

父节点继续沿 `P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md`、O6R 与 D1 引用链追溯到
唯一权威设计。production prerequisite 为 integration commit `2d5df5ae`；其中
`cdc31e54` 是单一挂起 owner 修复，现有
`db_actor_transaction_explicit_body_actual_pending_releases_actor_segment` 已为 GREEN。

## DAG 位置与 owner

本节点是 O6 combined acceptance 的 transaction 测试 owner。完成后解除 transaction 侧的
combined probe；lease 矩阵由 O6R8 独立拥有。当前候选仍是实现检查点，不是稳定验收候选。

父节点已确认：

- production 真实入口是 legacy `db.transaction(body-expression)` 与显式 `DbTransactionIr`
  evaluator，不得只调用 `TransactionLifecycle`；
- `program_db::wait::await_operation` 是 DB actual-Pending 的唯一 Actor segment 挂起 owner；
- transaction begin/body/commit/abort 使用冻结 fixture 的 FIFO script、gate、metrics、
  ordered trace、真实 Actor frame 与 checkpoint；
- O6R4 的失败由 O6R6 修复，现有最小 GREEN 必须保留并纳入完整矩阵；
- 上游双重挂起曾遮挡 body Pending 后的 commit/abort/error/drop 行为，本节点必须逐项形成可观察证据。

## 唯一写集

- `runtime/eval/src/program_db/tests/transaction.rs`
- `P5-F445H-O6R7-db-actor-transaction-matrix-resume-result.md`

不得修改：

- `runtime/eval/src/program_db/tests/fixture.rs` 或 `fixture/**`；
- `ordinary.rs`、`lease.rs`、`program_db.rs`、`eval_context.rs` 或任何 production；
- Actor E3、capability-context、service-db、driver tests；
- Cargo、manifest、lockfile、生成物或其它任务文档。

不得复制 fake、linked program、Actor frame 或 transaction lifecycle 来绕过冻结 fixture。

## 必须覆盖

最终 selector 中至少有 7 个非零 `db_actor_transaction_*` Rust 测试函数，并保留现有最小
GREEN。参数化 source/phase 可以在一个函数内循环，但每个 case 必须打印 source/phase 并独立断言
ordered event trace、phase metrics、checkpoint rollback 与竞争 Actor segment。

1. legacy 与显式 source 的 Ready success trace 都严格为
   `begin -> body-create -> commit`，abort 为零；
2. 两种 source 分别把 begin、body DB、commit、abort 设为唯一 actual-Pending phase，逐案证明
   operation 只构造一次、Pending 不重启、Actor segment 只切一次；
3. begin Ready-error 与 pending-then-error 都不调用 abort；
4. body error 与显式非法 flow 都只调用一次 abort，并保持原错误；
5. commit error只构造/等待一次 commit，随后一次 abort，返回 commit error；
6. commit actual-Pending 后 drop：commit constructed/drop 各一次，
   ready terminal、abort、finalize 均为零；
7. body actual-Pending 后 drop：body future 在 terminal 前 drop，commit/abort 均为零。

`abort_transaction` 没有错误返回，不测试不存在的 abort error。异常 outer drop 不要求 abort
acknowledgement，不允许 detached cleanup。两种 source 的正常和错误路径必须可对比，不能只覆盖显式
source。

每个被选择 phase 都断言：

- `constructed == 1`；
- first-Ready 没有 `pending_returns`；
- actual-Pending 在 gate 放行前至少一次 Pending，放行后同一 future terminal，不重建；
- drop case 为 `dropped_before_terminal == 1` 且 `ready_returns == 0`；
- 禁止 phase 的 constructed/poll/terminal 均为零。

## 完成与停止条件

必须通过真实 evaluator 入口。若现有 fixture 缺少完成矩阵所需的纯机械 helper，或任一 case 暴露
production 行为错误，五分钟内停止，保留最小失败证据并返回
`TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`；不得修改 fixture、production、降低矩阵或复制 fake。

从启动到第一次修改 `transaction.rs` 不超过五分钟；此前不跑测试、不重做设计。不得派子 Agent。

风险：高（transaction phase 生命周期与 Actor actual-Pending）。本节点只产生开发自验收证据；
独立 combined acceptance 由后续唯一 owner 完成。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试函数数；少于 7 或零测试不算完成。不运行完整 eval/stage gate、stable、live、network 或
MongoDB。不得 merge、rebase 或 push。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r7-transaction
branch   codex/p5-f445h-o6r7-transaction
```

先提交 tests，再单独提交 result；返回两个 commit、变更摘要、未决问题和自验收矩阵。worktree 必须
clean。证据仅对以 `2d5df5ae` production 状态为基础的本分支有效；fixture、transaction production、
Actor E3 或相关依赖变化会使证据失效。
