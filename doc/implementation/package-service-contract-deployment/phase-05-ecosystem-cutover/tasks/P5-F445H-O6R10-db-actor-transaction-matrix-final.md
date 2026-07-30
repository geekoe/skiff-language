# P5-F445H-O6R10 DB/Actor transaction matrix final

状态：Ready。O6R9 已重新冻结 transaction 所需的 statement-backed body-create 与非法-flow
block；O6R7 的 fixture blocker 已解除。本节点只完成 transaction 行为矩阵。

## 直接父节点

- `P5-F445H-O6R9-db-actor-fixture-case-closure-result.md`
- `P5-F445H-O6R7-db-actor-transaction-matrix-resume-result.md`
- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`

引用链继续沿 O6R1、O6R 与 D1 追溯到唯一权威设计。production prerequisite 为 `2d5df5ae`，
重新冻结的 test fixture checkpoint 为 `637567f3`。当前已有两个 GREEN：

- `db_actor_transaction_fixture_exposes_explicit_illegal_flow_case`
- `db_actor_transaction_explicit_body_actual_pending_releases_actor_segment`

不得删除、忽略或弱化它们。

## DAG、入口与 owner

本节点是 O6 combined acceptance 的 transaction child owner。完成后解除 transaction 侧
combined probe；lease 由 O6R11 独立拥有。当前仍是实现检查点。

必须复用唯一 fixture：

- legacy source 使用 `linked.legacy_transaction` 和 production legacy transaction evaluator；
- explicit source 使用 `linked.explicit_transaction` 和 production explicit transaction evaluator；
- result/body DB case 使用已存在的 raw create expression与
  `BODY_CREATE_BLOCK_LABEL`；
- 显式非法 flow 使用 `ILLEGAL_FLOW_BLOCK_LABEL`；
- phase script/gate/metrics、ordered trace、heap checkpoint 与真实 Actor frame 均来自 frozen
  fixture。

可以 clone frozen `CallIr` / `DbTransactionIr` 后只选择已有 expression 或 block label；不得复制
program、executable、fake 或 lifecycle。不得只调用 `TransactionLifecycle`。

## 唯一写集

- `runtime/eval/src/program_db/tests/transaction.rs`
- `P5-F445H-O6R10-db-actor-transaction-matrix-final-result.md`

不得修改 fixture、ordinary/lease child、production、Actor E3、capability-context、service-db、
driver tests、Cargo、manifest 或 lockfile。

## 必须覆盖

最终 selector 至少包含 7 个非零 `db_actor_transaction_*` Rust 测试函数；每个参数化 case 必须打印
source/phase 并独立断言 ordered trace、metrics、heap checkpoint/rollback 与竞争 Actor segment。

1. legacy 与 explicit Ready success 都严格为
   `Begin -> BodyCreate -> Commit`，Abort 为零；
2. 两种 source 分别覆盖 Begin、BodyCreate、Commit、Abort 为唯一 actual-Pending phase，证明
   operation 只构造一次、Pending 不重启、Actor segment 只切一次；
3. Begin Ready-error 与 pending-then-error 均不调用 Abort；
4. body error 与 explicit illegal flow 均只调用一次 Abort，并保持原错误；
5. Commit error只调用一次 Commit，随后一次 Abort，返回 Commit error；
6. Commit actual-Pending 后 drop：Commit constructed/drop 各一次，Ready terminal、Abort、
   finalize 为零；
7. BodyCreate actual-Pending 后 drop：body future terminal 前 drop，Commit/Abort 均为零。

`abort_transaction` 没有错误返回，不测试不存在的 abort error。异常 outer drop 不要求 abort
acknowledgement，也不允许 detached cleanup。两种 source 的正常与错误 trace 必须可对比；不能只覆盖
explicit。

每个选中 phase 断言：

- `constructed == 1`；
- first-Ready 没有 Pending；
- actual-Pending 在 gate 放行前至少一次 Pending，放行后同一 future terminal；
- drop case 为 `dropped_before_terminal == 1`、`ready_returns == 0`；
- 禁止 phase 的 constructed/poll/terminal 均为零。

## 停止条件与验证

如果重新冻结的 fixture 仍不足，或任一 case 暴露 production 缺陷，五分钟内停止并保留最小失败证据，
返回 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`；不得改 fixture/production、降低矩阵或复制
fake。启动后五分钟内首次修改 `transaction.rs`；此前不跑测试或重做设计。不得派子 Agent。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试函数数；少于 7 或零测试不算完成。不运行完整 eval/stage gate、stable、live、network 或
MongoDB。

风险：高。开发自验收不代替后续独立 combined acceptance。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r10-transaction
branch   codex/p5-f445h-o6r10-transaction
```

先提交 tests，再单独提交 result；返回两个 commit、自验收矩阵与未决问题。worktree clean；不得
merge/rebase/push。fixture、transaction production、Actor E3 或依赖变化会使证据失效。
