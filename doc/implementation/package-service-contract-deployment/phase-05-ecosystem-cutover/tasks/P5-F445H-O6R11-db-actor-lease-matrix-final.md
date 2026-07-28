# P5-F445H-O6R11 DB/Actor lease matrix final

状态：Ready。O6R9 已重新冻结 lease 所需的有效 binding slot、body-create block 与非法-flow block；
O6R8 的 fixture blocker 已解除。本节点只完成 claim/read/renew/lost/release 行为矩阵。

## 直接父节点

- `P5-F445H-O6R9-db-actor-fixture-case-closure-result.md`
- `P5-F445H-O6R8-db-actor-lease-matrix-resume-result.md`
- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`

引用链继续沿 O6R1、O6R 与 D1 追溯到唯一权威设计。production prerequisite 为 `2d5df5ae`，
重新冻结的 test fixture checkpoint 为 `637567f3`。当前已有两个 GREEN：

- `db_actor_lease_fixture_exposes_required_binding_variant`
- `db_actor_lease_claim_pending_uses_one_actor_segment`

不得删除、忽略或弱化它们。

## DAG、入口与 owner

本节点是 O6 combined acceptance 的 lease child owner。完成后解除 lease 侧 combined probe；
transaction 由 O6R10 独立拥有。当前仍是实现检查点。

必须复用唯一 fixture：

- claim/read 使用 production `eval_program_db_lease_claim` /
  `eval_program_db_lease_read` 与 frozen linked IR；
- O6R6 已保留从 `eval_program_expr_ref` 穿过 `LinkedExprIr::DbLeaseClaim` 的 actual-Pending 回归；
  完整生命周期 case 可以 clone frozen claim 后选择 fixture block label，并调用上述 production
  evaluator，不必为每个 body variant复制 linked expression/program；
- claim success 的 Env 使用 `Env::for_program_executable`，让 frozen binding slot 真实可见；
- body DB case 使用 `BODY_CREATE_BLOCK_LABEL`，非法 flow 使用
  `ILLEGAL_FLOW_BLOCK_LABEL`；
- phase script/gate/metrics、renew probe、ordered trace 与真实 Actor frame 均来自 frozen fixture。

不得直接测试 `LeaseRenewOwner`、fake store 或 helper来代替 production claim/read evaluator。

## 唯一写集

- `runtime/eval/src/program_db/tests/lease.rs`
- `P5-F445H-O6R11-db-actor-lease-matrix-final-result.md`

不得修改 fixture、ordinary/transaction child、production、Actor E3、capability-context、service-db、
driver tests、Cargo、manifest 或 lockfile。

## 必须覆盖

最终 selector 至少包含 8 个非零 `db_actor_lease_*` Rust 测试函数；组合 case 可以循环，但每个 case
必须打印 phase/variant 并独立断言 ordered trace、metrics、binding 可见性与竞争 Actor segment。

1. claim `None` 的 Ready 与 actual-Pending 均不导入 binding、不启动 Renew、不调用 Release；
2. claim success 的 Ready 与 actual-Pending 只在成功后导入 binding，Claim 只启动一次；
3. normal success、body error、explicit illegal flow 均按
   `stop/join Renew -> LeaseLost -> Release` 收尾；
4. LeaseLost Ready/actual-Pending 与 Release error 保持一次调用和 production 既有错误优先级；
5. body DB actual-Pending 期间 drop 使真实 renew future 被 abort/drop，之后 poll 计数不再增长；
   异常停止时 Release 允许为零；
6. Release actual-Pending 期间 drop 不重建 Release，且无 late terminal；
7. lease read 覆盖 Ready、actual-Pending、None、store error 与受限 heap 触发的 decode error；
8. lease read actual-Pending drop 不重建、不 materialize，也不启动其它 phase。

Renew tick 数不作精确合同；只断言 first-poll、正常 stop/join、outer-drop abort 与 drop 后 poll 不增长。
Claim/Read/LeaseLost/Release 每个真实方法必须有非零 constructed/poll 证据；Renew 在 body
actual-Pending case 中形成真实 first-poll/drop 证据。decode error 使用 object/array结果和受限
`RequestHeapLimits`，不能伪造 store error代替。Pending success 用竞争 Actor acquire 证明单次切段。

每个选中 phase 断言：

- `constructed == 1`；
- first-Ready 没有 Pending；
- actual-Pending 在 gate 放行前至少一次 Pending，放行后同一 future terminal；
- drop case 为 `dropped_before_terminal == 1`、`ready_returns == 0`；
- late sender不物化结果、不启动 terminal、不重建 operation。

## 停止条件与验证

如果重新冻结的 fixture 仍不足，或任一 case 暴露 production 缺陷，五分钟内停止并保留最小失败证据，
返回 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`；不得改 fixture/production、降低矩阵或复制
fake。启动后五分钟内首次修改 `lease.rs`；此前不跑测试或重做设计。不得派子 Agent。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r11-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r11-lease/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r11-lease/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试函数数；少于 8 或零测试不算完成。不运行完整 eval/stage gate、stable、live、network 或
MongoDB。

风险：高。开发自验收不代替后续独立 combined acceptance。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r11-lease
branch   codex/p5-f445h-o6r11-lease
```

先提交 tests，再单独提交 result；返回两个 commit、自验收矩阵与未决问题。worktree clean；不得
merge/rebase/push。fixture、lease production、Actor E3 或依赖变化会使证据失效。
