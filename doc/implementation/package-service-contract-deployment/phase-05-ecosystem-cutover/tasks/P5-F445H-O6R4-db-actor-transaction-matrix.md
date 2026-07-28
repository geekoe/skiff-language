# P5-F445H-O6R4 DB/Actor transaction matrix

状态：Ready。共享fixture已经冻结。本节点只补两种transaction source的phase矩阵，不修改fixture或
production。

## 直接父节点

- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

production prerequisite为integration commit `ceb73fbc`。父节点沿
`P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md` §5 F2冻结了完整矩阵。

## 唯一写集

- `runtime/eval/src/program_db/tests/transaction.rs`
- `P5-F445H-O6R4-db-actor-transaction-matrix-result.md`

不得修改共享fixture、`program_db.rs`、ordinary/lease child、production、Cargo或lockfile。

## 必须覆盖

使用共享两种linked transaction builder、phase script/gate/metrics和真实Actor frame，至少形成7个
非零`db_actor_transaction_*`测试：

1. legacy与显式source的Ready success trace都严格为begin/body-create/commit，无abort；
2. 两种source分别把begin、body DB、commit和abort设为唯一Pending phase，证明每个phase只构造一次、
   Actor只切一次；
3. begin Ready-error与pending-then-error都不abort；
4. body error与显式非法flow只abort一次并保留原错误；
5. commit error只commit一次、随后abort一次，返回commit error；
6. commit真实Pending后drop：commit constructed/drop各一次，terminal/abort/finalize为零；
7. body真实Pending后drop：commit/abort均为零。

参数化source/phase可以在单个test function内循环，但selector实际函数不少于7；每个case必须独立断言
ordered event trace、metrics、checkpoint rollback和竞争Actor segment。`abort_transaction`没有错误
返回，不测试不存在的abort error。异常drop不要求abort acknowledgement。

必须穿过legacy/explicit真实evaluator入口，不能只调用`TransactionLifecycle`。当前checkpoint缺失
selector就是RED；不得改production制造失败。测试若发现实现错误，保留最小失败证据并返回FAIL。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r4-transaction/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数，少于7或零测试不算完成。不运行完整eval/stage gate、stable/live/network/Mongo。

冻结fixture若缺少唯一机械helper，五分钟内返回`TASK_SCOPE_EXPANDED`和精确缺口；不得修改fixture、
放宽矩阵或复制fake。不得派子Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r4-transaction
branch   codex/p5-f445h-o6r4-transaction
```

先提交tests，再单独提交result；worktree clean，不得merge/rebase/push。

