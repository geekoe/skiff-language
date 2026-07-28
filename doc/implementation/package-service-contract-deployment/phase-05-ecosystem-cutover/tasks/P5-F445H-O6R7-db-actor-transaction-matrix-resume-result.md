# P5-F445H-O6R7 DB/Actor transaction matrix resume result

状态：`TASK_SCOPE_EXPANDED / FROZEN_FIXTURE_BLOCKER / MATRIX_NOT_COMPLETED`。

production prerequisite `2d5df5ae` 存在，O6R6 的最小 actual-Pending 回归继续为 GREEN。
tests commit 为 `aede909b`。本节点只修改
`runtime/eval/src/program_db/tests/transaction.rs` 与本 result；没有修改冻结 fixture、production、
Cargo、manifest、lockfile或其它任务文件。

## 1. 停止原因

叶子任务要求覆盖“显式非法 flow”，并禁止 transaction child 复制 linked program 或修改冻结
fixture。当前唯一 linked executable 不能构造该 case：

- `body.blocks` 只有 `entry` 与 `empty`；
- 两个 block 的 `statements` 引用列表都为空；
- executable 的全局 `body.statements` 也为空。

`DbTransactionIr.body` 只能选择已有 block。两个空 block 都从真实
`exec_program_block` 返回 `Flow::Continue`，因此不能进入
`eval_program_explicit_db_transaction_with_context` 的 `Flow::Return`、
`Flow::Parked | Flow::ContinueConsumer` 或 `Flow::Break | Flow::LoopContinue` 非法-flow 分支。

完成该 case 至少需要以下任一越界动作：

1. 修改 `runtime/eval/src/program_db/tests/fixture/program.rs`，加入一个 statement-backed block 与
   对应机械 builder；或
2. 在 `transaction.rs` 复制/改写 `LinkedExecutable`、`LinkedFileUnit` 或 linked program。

第一项超出唯一写集且违反 fixture frozen，第二项被任务明确禁止。把“缺失 result expression”或
“不存在的 block label”改称非法 flow 会降低矩阵：它们分别进入 result decode error 或普通 body
error，不覆盖要求的非法 `Flow` 分支。

因此按“fixture 缺少完成矩阵所需纯机械 helper 时停止”的条件返回
`TASK_SCOPE_EXPANDED`，没有继续扩写其它矩阵，也没有触碰 production。

## 2. 最小失败证据

保留测试：

```text
db_actor_transaction_fixture_exposes_explicit_illegal_flow_case
```

它只读取共享 fixture 的真实 linked executable，不创建 fake、Actor frame、transaction lifecycle
或替代 linked program。focused selector 实际发现 2 个 `db_actor_transaction_*` 测试：

```text
running 2 tests
test ...db_actor_transaction_fixture_exposes_explicit_illegal_flow_case ... FAILED
test ...db_actor_transaction_explicit_body_actual_pending_releases_actor_segment ... ok

the frozen transaction fixture exposes no statement-backed block for the required
explicit illegal-flow case; blocks=[("entry", []), ("empty", [])]; statements=[]

test result: FAILED. 1 passed; 1 failed; 0 ignored; 306 filtered out
```

实际函数数为 2，低于完成态至少 7 个的要求；这是
`MATRIX_NOT_COMPLETED` 的显式证据，不得接收为 combined transaction PASS。现有 O6R6 最小
GREEN 未被删除或弱化。

## 3. 验证

以下命令在 tests commit 内容上执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::transaction:: -- --nocapture
```

结果：预期 RED，2 tests，1 passed / 1 failed；失败是冻结 fixture capability 缺口，不是编译、
零测试、production actual-Pending 回归或外部服务失败。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
```

结果：PASS；仅有仓库既有 warning。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r7-transaction/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果：PASS。没有运行完整 eval/stage gate、stable、live、network 或 MongoDB。

## 4. 自验收矩阵

| 任务条款 | 代码/反向证据 | 验证 | 结果 |
| --- | --- | --- | --- |
| 只写 `transaction.rs` 与 result | `git diff-tree` 只包含两份允许文件 | worktree 最终 clean | PASS |
| 保留 O6R6 最小 GREEN | 原测试未改；focused selector 中实际通过 | focused selector | PASS |
| 显式非法 flow | fixture 只有两个空 block且无 statements；最小探针精确失败 | focused selector | BLOCKED |
| 至少 7 个非零 transaction tests | 实际仅 2 个 | `rg '^\\s*(async )?fn db_actor_transaction_'` | FAIL |
| Ready/Pending/error/drop 完整矩阵 | 因 frozen fixture blocker 按停止条件未继续 | 未伪造缩减矩阵 | NOT RUN |
| ordered trace、phase metrics、checkpoint、竞争 segment | 仅保留既有 body actual-Pending 最小证据；完整逐案矩阵未形成 | 不冒充 acceptance | NOT RUN |
| 禁止复制 fake/linked program/Actor frame/lifecycle | 新探针只读 `fixture.linked.executable()` | 反向审阅 tests diff | PASS |
| 不改 fixture/production/Cargo/lockfile | tests commit 仅改 `transaction.rs` | `git diff-tree --name-only aede909b^ aede909b` | PASS |
| 聚焦静态验证 | check/fmt/diff-check 均通过 | 上述命令 | PASS |
| 不运行越界环境验证 | 未启动 stable/live/network/MongoDB | 命令记录 | PASS |

## 5. 解除阻塞条件

由 fixture owner 单独增加一个纯机械的显式非法-flow block/builder，并重新冻结 fixture；随后应从新的
integration checkpoint 重建本节点。fixture、transaction production、Actor E3 或相关依赖发生
变化都会使本结果证据失效。
