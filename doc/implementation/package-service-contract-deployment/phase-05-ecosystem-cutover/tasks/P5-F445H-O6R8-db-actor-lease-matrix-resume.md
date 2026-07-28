# P5-F445H-O6R8 DB/Actor lease matrix resume

状态：Ready。O6R6 已消除真实 lease claim actual-Pending 路径的双重挂起，并保留了一条通过的最小
回归。本节点从该检查点继续完成 claim/read/renew/lost/release 行为矩阵；不再修改 production 或
共享 fixture。

## 直接父节点

- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`
- `P5-F445H-O6R5-db-actor-lease-matrix-result.md`
- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

父节点继续沿 `P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md`、O6R 与 D1 引用链追溯到
唯一权威设计。production prerequisite 为 integration commit `2d5df5ae`；其中
`cdc31e54` 是单一挂起 owner 修复，现有
`db_actor_lease_claim_pending_uses_one_actor_segment` 已为 GREEN。

## DAG 位置与 owner

本节点是 O6 combined acceptance 的 lease 测试 owner。完成后解除 lease 侧的 combined probe；
transaction 矩阵由 O6R7 独立拥有。当前候选仍是实现检查点，不是稳定验收候选。

父节点已确认：

- production 真实入口是 linked `DbLeaseClaimIr` 与 `DbLeaseReadIr` expression evaluator；
- `program_db::wait::await_operation` 是 DB actual-Pending 的唯一 Actor segment 挂起 owner；
- claim/read/lost/release 使用冻结 fixture 的 FIFO script、gate、metrics、ordered trace 与真实
  Actor frame；renew 使用冻结 first-poll/drop signal；
- O6R5 的失败由 O6R6 修复，现有最小 GREEN 必须保留并纳入完整矩阵；
- 上游双重挂起曾遮挡 binding、renew、lost、release、read decode 与 drop 行为，本节点必须逐项形成
  可观察证据。

## 唯一写集

- `runtime/eval/src/program_db/tests/lease.rs`
- `P5-F445H-O6R8-db-actor-lease-matrix-resume-result.md`

不得修改：

- `runtime/eval/src/program_db/tests/fixture.rs` 或 `fixture/**`；
- `ordinary.rs`、`transaction.rs`、`program_db.rs`、`eval_context.rs` 或任何 production；
- Actor E3、capability-context、service-db、driver tests；
- Cargo、manifest、lockfile、生成物或其它任务文档。

不得复制 fake、linked program、Actor frame 或 lease lifecycle 来绕过冻结 fixture。

## 必须覆盖

最终 selector 中至少有 8 个非零 `db_actor_lease_*` Rust 测试函数，并保留现有最小 GREEN。组合 case
可以在一个函数内循环，但每个 case 必须打印 phase/variant 并独立断言 ordered event trace、phase
metrics、binding 可见性与竞争 Actor segment。

1. claim `None` 的 Ready 与 actual-Pending 都不导入 binding、不启动 renew、不调用 release；
2. claim success 的 Ready 与 actual-Pending 都只在成功后导入 binding，claim 只启动一次；
3. normal success、body error、显式非法 flow 都按
   `stop/join renew -> lease-lost -> release` 收尾；
4. lease-lost Ready/actual-Pending 与 release error 保持一次调用和 production 既有可见优先级；
5. body DB actual-Pending 期间 drop 会使真实 renew future 被 abort/drop，之后 poll 计数不再增长；
   异常停止时 release 允许为零；
6. release actual-Pending 期间 drop 不重建 release，且无 late terminal；
7. lease read 覆盖 Ready、actual-Pending、None、store error 与受限 heap 触发的 decode error；
8. lease read actual-Pending drop 不重建、不 materialize，也不启动其它 phase。

renew tick 数不作精确合同；只断言 first-poll、正常 stop/join、outer-drop abort 和 drop 后 poll 不增长。
claim/read/lost/release 每个真实方法必须有非零 constructed/poll 证据；禁止 phase 必须为零。decode
error 使用 object/array 结果和受限 `RequestHeapLimits`，不能伪造 store error 代替。Pending success
必须用竞争 Actor acquire 证明单次切段。

每个被选择 phase 都断言：

- `constructed == 1`；
- first-Ready 没有 `pending_returns`；
- actual-Pending 在 gate 放行前至少一次 Pending，放行后同一 future terminal，不重建；
- drop case 为 `dropped_before_terminal == 1` 且 `ready_returns == 0`；
- late sender 不得物化结果、启动 terminal 或重建 operation。

## 完成与停止条件

必须通过真实 expression evaluator 入口。若现有 fixture 缺少完成矩阵所需的纯机械 helper，或任一
case 暴露 production 行为错误，五分钟内停止，保留最小失败证据并返回
`TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`；不得修改 fixture、production、降低矩阵或复制 fake。

从启动到第一次修改 `lease.rs` 不超过五分钟；此前不跑测试、不重做设计。不得派子 Agent。

风险：高（lease 生命周期、renew owner 与 Actor actual-Pending）。本节点只产生开发自验收证据；
独立 combined acceptance 由后续唯一 owner 完成。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r8-lease/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试函数数；少于 8 或零测试不算完成。不运行完整 eval/stage gate、stable、live、network 或
MongoDB。不得 merge、rebase 或 push。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r8-lease
branch   codex/p5-f445h-o6r8-lease
```

先提交 tests，再单独提交 result；返回两个 commit、变更摘要、未决问题和自验收矩阵。worktree 必须
clean。证据仅对以 `2d5df5ae` production 状态为基础的本分支有效；fixture、lease production、
Actor E3 或相关依赖变化会使证据失效。
