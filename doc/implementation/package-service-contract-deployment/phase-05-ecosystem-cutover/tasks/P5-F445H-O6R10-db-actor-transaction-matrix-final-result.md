# P5-F445H-O6R10 DB/Actor transaction matrix final result

状态：`PASS / TRANSACTION_MATRIX_COMPLETE / COMBINED_TRANSACTION_CHILD_UNBLOCKED`。

tests 提交为 `b52f63b6`。production prerequisite 仍为 `2d5df5ae`，重新冻结的 fixture checkpoint
仍为 `637567f3`；本节点从 `da8bf072` 启动。本结果只完成 O6 combined acceptance 的 transaction
child，lease 仍由 O6R11 独立拥有，不声明 combined acceptance 已通过。

## 1. 实现结果

`runtime/eval/src/program_db/tests/transaction.rs` 现有 9 个非零
`db_actor_transaction_*` Rust 测试函数。两个 prerequisite GREEN 原样保留并继续通过：

- `db_actor_transaction_fixture_exposes_explicit_illegal_flow_case`
- `db_actor_transaction_explicit_body_actual_pending_releases_actor_segment`

新增矩阵通过统一 test-only driver 覆盖 legacy 与 explicit 两个 production evaluator。legacy
只 clone `linked.legacy_transaction` 并选择 fixture 中既有 raw-create expression；explicit 只 clone
`linked.explicit_transaction` 并选择 `BODY_CREATE_BLOCK_LABEL` 或
`ILLEGAL_FLOW_BLOCK_LABEL`。没有复制 program、executable、fake、lifecycle 或 Actor frame，也没有
直接调用 `TransactionLifecycle`。

每个参数化 case 都打印 source/phase，并独立验证：

- construction phase 的全局顺序，以及每个 phase 的完整 event kind 顺序；
- first-Ready 或 actual-Pending/drop 对应的完整 metrics；
- 成功 body allocation 保留，错误路径 rollback，或 outer-drop 时的预期 heap 状态；
- first-Ready 全程持有 Actor segment；actual-Pending 能让竞争 Actor acquire，gate terminal 后须等待
  同一 frame 恢复，并在完成态重新持有 segment。

## 2. 行为矩阵

| 行为 | legacy | explicit | 结果 |
| --- | --- | --- | --- |
| Ready success | `Begin -> BodyCreate -> Commit` | `Begin -> BodyCreate -> Commit` | PASS；Abort 为零 |
| Begin actual-Pending | 1 case | 1 case | PASS；构造一次，同一 future terminal |
| BodyCreate actual-Pending | 1 case | 1 case | PASS；构造一次，同一 future terminal |
| Commit actual-Pending | 1 case | 1 case | PASS；构造一次，同一 future terminal |
| Abort actual-Pending | body error 后 1 case | body error 后 1 case | PASS；原 `DbDecode` message 保留 |
| Begin Ready-error | 1 case | 1 case | PASS；BodyCreate/Commit/Abort 全为零 |
| Begin pending-then-error | 1 case | 1 case | PASS；BodyCreate/Commit/Abort 全为零 |
| Body error | 1 case | 1 case | PASS；Abort 恰好一次，原 message 保留 |
| explicit illegal flow | 不适用 | `Return` block 1 case | PASS；Abort 恰好一次，非法-flow error 保留 |
| Commit error | 1 case | 1 case | PASS；Commit 一次、Abort 一次、heap rollback |
| Commit actual-Pending outer drop | 1 case | 1 case | PASS；Commit pre-terminal drop 一次，Abort/finalize 为零 |
| BodyCreate actual-Pending outer drop | 1 case | 1 case | PASS；body pre-terminal drop 一次，Commit/Abort/finalize 为零 |

actual-Pending completion case 在 gate 放行前都观测到至少一次 Pending；放行后同一个 scripted future
产生唯一 Ready terminal，`constructed == 1`。drop case 均为
`dropped_before_terminal == 1`、`ready_returns == 0`。first-Ready case 均为一次 poll、零 Pending、唯一
Ready terminal。每个 case 对未进入的 transaction phase 都断言默认零 metrics 与空 event trace。

## 3. 聚焦验证

以下命令均锚定 tests commit `b52f63b6` 的代码状态，并使用任务指定的 target dir：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
```

结果：PASS。主 unit test binary 实际运行 9 tests，`9 passed; 0 failed; 307 filtered out`。随后两个
integration test binary 各有零个名称匹配项；权威 transaction selector 本身非零且超过至少 7 个函数
的门槛。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
```

结果：PASS；仅报告仓库既有 warning，没有本任务新增 warning。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r10-transaction/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果：均 PASS。

## 4. 反向检查与写集

函数计数：

```text
rg -c '^\s*(?:async\s+)?fn db_actor_transaction_' \
  runtime/eval/src/program_db/tests/transaction.rs
9
```

`transaction.rs` 中反向搜索
`EvalRuntimeProgram::new`、`executables: vec![LinkedExecutable` 与 `TransactionLifecycle` 均为零。
tests commit `b52f63b6` 的唯一文件为：

```text
runtime/eval/src/program_db/tests/transaction.rs
```

本 result 是唯一额外写入。fixture、ordinary/lease child、production、Actor E3、
capability-context、service-db、driver tests、Cargo、manifest 与 lockfile 均无修改。

## 5. 自验收矩阵

| 任务条款 | 代码 / 反向证据 | 验证 | 结果 |
| --- | --- | --- | --- |
| 保留两个既有 GREEN | 两个函数原样存在 | transaction selector 中二者均通过 | PASS |
| legacy/explicit 真实 evaluator | source driver 分别调用 production legacy/explicit evaluator | 两种 source 的全矩阵输出 | PASS |
| Ready success 严格 trace | 两种 source 均断言 `Begin -> BodyCreate -> Commit`，Abort 零 | Ready matrix | PASS |
| 四 phase actual-Pending | 两种 source × Begin/BodyCreate/Commit/Abort | 8 个参数化 case | PASS |
| operation 构造一次且 Pending 不重启 | selected phase `constructed == 1`；gate 前 Pending、后同 future Ready | metrics + event trace | PASS |
| Actor segment 只在 actual-Pending 切出 | 竞争 acquire、gate 后等待、terminal 后 frame 重新持有 | 每个 Pending case 独立断言 | PASS |
| Begin 两类 error 不 Abort | Ready-error 与 pending-then-error，各覆盖两种 source | Body/Commit/Abort 零 metrics | PASS |
| body error 与非法 flow | 两种 source body error；explicit 真实 Return flow | Abort 一次且 error 保留 | PASS |
| Commit error | 两种 source 均 Commit 一次、Abort 一次 | 结构化 `DbDecode` + rollback | PASS |
| Commit Pending drop | 两种 source 的同一 pending Commit future 被 pre-terminal drop | Ready/Abort/finalize 零 | PASS |
| Body Pending drop | 两种 source 的同一 pending body future 被 pre-terminal drop | Commit/Abort/finalize 零 | PASS |
| ordered trace / metrics / heap / 竞争 segment | 统一断言 helper，但每个参数化 case 独立执行 | selector `--nocapture` | PASS |
| 至少 7 个非零函数 | 实际 9 个 | `rg -c` + selector 9/9 | PASS |
| 不复制 fixture 或 lifecycle | 三个 owner/construction 反向搜索均为零 | tests diff 审阅 | PASS |
| 唯一写集与提交顺序 | tests commit 仅 `transaction.rs`；本文单独提交 | `git show --name-only` | PASS |
| 静态质量 | locked check、fmt check、diff check | 全部通过 | PASS |
| 环境边界 | 未运行完整 eval/stage gate、stable、live、network 或 MongoDB；未 merge/rebase/push | 命令记录 | PASS |

## 6. 未决问题

本节点无 transaction 未决问题。O6 combined acceptance 仍需等待 O6R11 lease child 与后续独立
combined probe；fixture、transaction production、Actor E3 或相关依赖变化会使本结果证据失效。
