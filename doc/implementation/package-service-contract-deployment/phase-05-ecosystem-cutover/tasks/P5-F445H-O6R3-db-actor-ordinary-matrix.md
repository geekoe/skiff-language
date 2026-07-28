# P5-F445H-O6R3 DB/Actor ordinary matrix

状态：Ready。共享fixture已经冻结。本节点只补ordinary/query行为矩阵，不修改fixture或production。

## 直接父节点

- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

production prerequisite为integration commit `ceb73fbc`。父节点沿
`P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md` §5 F1冻结了完整矩阵；本任务把它变成
单文件执行合同。

## 唯一写集

- `runtime/eval/src/program_db/tests/ordinary.rs`
- `P5-F445H-O6R3-db-actor-ordinary-matrix-result.md`

不得修改共享fixture、`program_db.rs`、transaction/lease child、production、Cargo或lockfile。

## 必须覆盖

使用共享`DbPhase`、script/gate/metrics、linked builder和真实Actor竞争acquire，至少形成9个非零
`db_actor_ordinary_*`测试：

1. 纯query Ready不触碰store，当前segment仍持有；
2. raw create first-Ready只启动一次且不切segment；
3. raw create真实Pending释放一次，恢复后才decode/materialize；
4. raw first-Ready error与pending-then-error都不重建；
5. raw pending drop只drop同一future，无terminal/result；
6. prepared create first-Ready的wait/finalizer各一次且不切segment；
7. prepared真实Pending期间不finalize，恢复后才向caller heap物化；
8. prepared wait的Ready-error、pending-then-error和finalizer error都不重放；
9. prepared pending drop不finalize、不重建。

参数化子case可以在同一Rust test中，但selector实际测试函数不少于9。Pending success必须用竞争Actor
segment证明`held -> released -> held`；同时断言operation constructed/poll/pending/ready/drop、
finalizer计数和heap/result可见时点。不得直接测试`wait::await_operation`代替真实
`Interpreter::eval_program_db_operation`。

当前checkpoint没有该selector，缺失证据就是本验收节点的RED；不得临时修改production或写
`panic!("RED")`制造假失败。测试若直接暴露production错误，保留最小失败证据并返回FAIL，不在本节点修复。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数，少于9或零测试不算完成。不运行完整eval/stage gate、stable/live/network/Mongo。

若冻结fixture缺少完成本矩阵所需的唯一机械helper，五分钟内报告精确缺口并返回
`TASK_SCOPE_EXPANDED`；不得修改fixture、放宽断言或复制第二套fake。不得派子Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r3-ordinary
branch   codex/p5-f445h-o6r3-ordinary
```

先提交tests，再单独提交result；worktree clean，不得merge/rebase/push。

