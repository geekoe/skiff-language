# P5-F445H-O6R8 DB/Actor lease matrix resume result

状态：`TASK_NOT_EXECUTABLE / FROZEN_FIXTURE_MISSING_REQUIRED_VARIANT / MATRIX_NOT_IMPLEMENTED`。

production prerequisite 为 integration commit `2d5df5ae`；本节点的最小 tests 证据提交为
`cdce8a3c`。O6R6 保留的
`db_actor_lease_claim_pending_uses_one_actor_segment` 继续为 GREEN，但冻结 fixture 无法构造任务要求的
正向 binding case。依任务停止条件，本节点发现该缺口后停止，没有修改 fixture、production、Cargo、
manifest 或 lockfile，也没有复制 linked program 或改用 lease evaluator 直接入口制造降级矩阵。

## 1. 硬停止原因

`runtime/eval/src/program_db/tests/fixture/program.rs` 中唯一真实 linked claim expression 由
`lease_claim()` 构造，并固定为：

```text
binding_slot = None
body = "empty"
```

`LinkedDbActorFixture::new()` 没有 claim/body/binding variant 参数，`linked_file()` 又已经把该固定 claim
复制进 executable 的 `LinkedExprIr::DbLeaseClaim`。因此 `lease.rs` 无法在真实
`eval_program_expr_ref` 入口下形成“claim success 后才导入 binding，并证明 binding 可见”的正向 case。

可选绕路都被任务合同明确禁止：

- 修改 `fixture/program.rs` 增加机械 variant builder 超出唯一写集；
- 在 `lease.rs` 重建 `LinkedFileUnit` / `EvalRuntimeProgram` 会复制冻结 linked program；
- 直接调用 `eval_program_db_lease_claim` 会绕过任务要求的真实 expression evaluator 入口；
- 只验证 `binding_slot = None` 或删减 binding 断言会降低必须覆盖矩阵。

这已经满足“fixture 缺少完成矩阵所需的纯机械 helper”停止条件，不需要继续触发 renew、lost、release、
read 或 drop case 才能判定任务不可执行。

## 2. 最小失败证据

tests commit 增加
`db_actor_lease_fixture_exposes_required_binding_variant`。该 probe 从冻结 executable 中找到 evaluator
实际消费的 `LinkedExprIr::DbLeaseClaim`，并要求矩阵必需的非空 `binding_slot`；它没有构造第二份 fake
或 linked program。

执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
```

实际发现 2 个 `db_actor_lease_*` 测试：

```text
db_actor_lease_fixture_exposes_required_binding_variant ... FAILED
db_actor_lease_claim_pending_uses_one_actor_segment ... ok

TASK_NOT_EXECUTABLE: frozen expression fixture has no lease binding variant
test result: FAILED. 1 passed; 1 failed; 0 ignored
```

这是非零 selector 中的精确 fixture-contract 失败，同时证明 O6R6 最小 GREEN 未回退。实际函数数为
2，低于完成要求的至少 8 个，因此不声明矩阵完成。

## 3. 自验收矩阵

| 任务条款 | 证据 | 结果 |
| --- | --- | --- |
| production 基线为 `2d5df5ae` | 本分支从包含该 integration commit 的 `edcaddc0` 开始 | PASS |
| 首次修改与停止时限 | 先修改 `lease.rs`；确认冻结 expression 无 binding variant 后立即停止扩展 | PASS |
| 保留 O6R6 最小 GREEN | 同一 selector 中原测试 `ok` | PASS |
| 真实 expression 入口下的 binding 正向证据 | 固定 `LinkedExprIr::DbLeaseClaim.binding_slot == None`，无 variant builder | BLOCKED |
| 至少 8 个非零 lease tests | 实际 2 个 | FAIL |
| claim/read/renew/lost/release 完整矩阵 | 按硬停止条件未继续实现或降低矩阵 | NOT RUN |
| 唯一写集 | 仅 `lease.rs` 与本 result | PASS |
| 禁止复制/绕路 | 未复制 fake、linked program、Actor frame 或 lifecycle；未直调 lease evaluator | PASS |
| 禁止 production/shared fixture 改动 | 无相关 diff | PASS |
| 不运行 stable/live/network/Mongo/完整 gate | 均未运行 | PASS |

## 4. 静态验证

以下命令通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo fmt --check
git diff --check
```

`cargo check` 只有仓库既有 warning。没有运行完整 eval/stage gate，没有连接 stable、live、network 或
MongoDB，也没有 merge、rebase 或 push。

## 5. 解除条件

fixture owner 需要在其独立写集中为同一个 linked executable 增加纯机械 lease case builder，至少能
选择 claim `binding_slot` 与 body flow，同时保持 `DbActorFixture`、真实 Actor frame 和真实 expression
入口不变。该 prerequisite 集成后，本 lease owner 才能重新开始至少 8 个测试的完整行为矩阵；fixture、
lease production、Actor E3 或相关依赖变化都会使本结果证据失效。
