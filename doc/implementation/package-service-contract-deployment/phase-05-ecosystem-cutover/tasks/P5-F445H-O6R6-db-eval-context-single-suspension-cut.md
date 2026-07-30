# P5-F445H-O6R6 DB EvalContext single-suspension cut

状态：Ready。O6R4与O6R5分别通过真实transaction body和lease claim expression证明：DB operation
owner已经使用E3 actual-Pending，但`EvalContext`五个DB expression arm仍保留旧的无条件
pre-suspend，造成同一Actor segment被释放两次。本节点只删除这一个共享旧owner。

## 直接父节点

- `P5-F445H-O6R4-db-actor-transaction-matrix-result.md`
- `P5-F445H-O6R5-db-actor-lease-matrix-result.md`
- `P5-F445H-O6R3-db-actor-ordinary-matrix-result.md`

production prerequisite为integration commit `449f8c66`。

真实RED commits：

- transaction：`94359d15`
- lease：`f32337a5`

任务worktree由主Agent把这两个单文件RED移植到当前checkpoint。父result已经冻结精确因果；不得重新
设计Actor或DB生命周期。

## Production修正

唯一production owner是`runtime/eval/src/eval_context.rs`的以下五个
`LinkedExprIr` arm：

- `DbOperation`
- `DbQuery`
- `DbTransaction`
- `DbLeaseClaim`
- `DbLeaseRead`

删除每个arm外层的：

```text
suspend_actor_segment
  -> await DB evaluator
  -> resume_actor_segment
```

改为直接等待对应`eval_program_db_*`入口并原样映射结果。理由与完成语义：

- 普通DB、transaction各phase、claim/read/lost/release的真实external wait已经统一由
  `program_db::wait::await_operation` first-poll；只有真实Pending才通过E3释放/恢复；
- `DbQuery`本身没有DB I/O，不能预先释放；其中嵌套expression由各自owner处理；
- first-Ready DB结果或错误不得切Actor segment；
- actual-Pending错误必须先由E3恢复segment，再向外层transaction/claim传播；
- transaction body不能再把“双重suspend”的内部错误误判为业务错误并选择abort；
- 不修改`suspend_actor_segment`/`resume_actor_segment` helper本身，也不触碰其它尚待E4R迁移的
  service/native/callback/emit/Actor/stream call site。

不得复制first-poll、直接调用frame `suspend/resume`或给DB operation增加特殊bool。

## RED→GREEN与验证

允许测试写集仅为移植来的：

- `runtime/eval/src/program_db/tests/transaction.rs`
- `runtime/eval/src/program_db/tests/lease.rs`

本节点只保持两条最小回归，不提前完成O6R4/O6R5矩阵。两条都必须从真实expression入口首poll得到
`Pending`，竞争Actor acquire可完成，且trace中没有因双重suspend产生的abort/release terminal。

验证：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_transaction_explicit_body_actual_pending_releases_actor_segment -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_lease_claim_pending_uses_one_actor_segment -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录`1/1`、`1/1`与ordinary非零测试数。反向搜索证明五个DB arm不再调用
`suspend_actor_segment`/`resume_actor_segment`，其它arm保持原样。不运行完整eval/stage gate、
stable/live/network/Mongo。

## 写集与停止条件

只允许：

- `runtime/eval/src/eval_context.rs`
- 上述两个最小RED test文件
- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`

不得修改program_db production、wait/transaction/lease owner、fixture、Actor E3、其它EvalContext
arm、Cargo或lockfile。

若修复需要改变E3、DB runner或其它call-site owner，返回`TASK_SCOPE_EXPANDED`。若两条RED在移植后的
当前base不能精确复现，返回`TASK_NOT_EXECUTABLE`，不得修改测试绕过。

风险：高（Actor suspension ownership共享检查点）。完成后只解除O6R4/O6R5继续测试，不代表矩阵或
J1已经PASS。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r6-db-callsite
branch   codex/p5-f445h-o6r6-db-callsite
```

先提交production+两条GREEN回归，再单独提交result。Worktree clean；不得merge/rebase/push。当前修复
单一明确，不派子Agent。

