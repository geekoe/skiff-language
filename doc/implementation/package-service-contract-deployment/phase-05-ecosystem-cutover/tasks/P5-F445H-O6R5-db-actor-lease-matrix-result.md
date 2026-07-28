# P5-F445H-O6R5 DB/Actor lease matrix result

状态：`FAIL / PRODUCTION_BLOCKED / MATRIX_NOT_IMPLEMENTED`。

本节点在冻结 shared fixture 上通过真实 linked claim expression、`eval_program_expr_ref` 与真实
`ActorExecutionFrame` 复现 production 双重 suspension。依任务合同，发现 production 失败后立即停止；
没有修改 fixture、production、Cargo 或 lockfile，也没有用直接调用
`eval_program_db_lease_claim` 绕过真实 expression 入口来制造 PASS。

## 1. 最小 RED

tests commit：`f32337a5`。

唯一保留的测试
`db_actor_lease_claim_pending_uses_one_actor_segment` 从 shared executable 中定位
`LinkedExprIr::DbLeaseClaim`，脚本化真实 claim method 为 Pending，并通过
`Interpreter::eval_program_expr_ref` 进入 production expression arm。首 poll 的期望是
`Pending`，实际立即返回：

```text
Err(InvalidArtifact(
  "Actor continuation attempted to suspend without an execution token"
))
```

selector：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
```

实际结果：

```text
running 1 test
test program_db::tests::lease::db_actor_lease_claim_pending_uses_one_actor_segment ... FAILED

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 293 filtered out
```

这是非零 selector 中的行为失败，不是编译失败、fixture 缺失、零测试或外部服务失败。

## 2. production 因果

1. `EvalContext` 的 `LinkedExprIr::DbLeaseClaim` arm 在调用 lease evaluator 前无条件执行
   `suspend_actor_segment()`，移走当前 Actor execution token。
2. `eval_program_db_lease_claim` 随后把真实 claim future 交给
   `program_db::wait::await_operation`。
3. claim 首 poll 返回 Pending 后，`await_operation` 通过同一 frame 的
   `await_if_pending` 再次请求 suspend。
4. frame 已由 expression arm suspend，第二次 suspend 因没有 execution token 而 fail closed，
   所以无法到达竞争 Actor acquire、claim terminal、binding、renew/lost/release 矩阵。

`DbLeaseRead` expression arm具有相同的 outer suspend 与 `await_operation` 组合；依“最小失败证据后
停止”合同，本节点没有再扩展第二个重复 RED。

## 3. 未完成矩阵

任务要求至少 8 个非零 `db_actor_lease_*` 测试。当前只保留 1 个最小 RED，因此不能声明矩阵完成，
也没有继续实现 claim Ready、binding、renew、lost、release、read decode/drop 等后续用例。
production owner修复真实 expression 入口的单一 suspension ownership 后，本节点需重新执行完整矩阵。

## 4. 静态验证与边界

以下命令通过：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo fmt --check

git diff --check
```

`cargo check` 只有仓库既有 warning。没有运行完整 eval/stage gate，没有连接 stable/live/network/Mongo，
没有 merge、rebase 或 push。
