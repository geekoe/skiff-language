# P5-F445H-O6R13 DB/Actor combined acceptance

状态：Ready。当前 DB evaluator production、单一挂起 owner、共享 fixture、ordinary/transaction/lease
矩阵与 pending renew stop 修复已全部合流。本节点在冻结的 O6 候选上做一次独立只读组合验收，并拥有
本候选唯一完整 `skiff-runtime-eval` gate。

## 直接父节点

- `P5-F445H-O6R10-db-actor-transaction-matrix-final-result.md`
- `P5-F445H-O6R11-db-actor-lease-matrix-final-result.md`
- `P5-F445H-O6R12-lease-renew-stop-race-result.md`
- `P5-F445H-O6R3-db-actor-ordinary-matrix-result.md`
- `P5-F445H-O6R6-db-eval-context-single-suspension-cut-result.md`

引用链继续沿 O6R1、O6R 与 D1 追溯到唯一权威设计。冻结代码候选为 integration commit
`ce01def6`；本任务文档提交只增加验收合同，不改变代码候选。

已知历史：

- O6R7/O6R8 的 fixture blocker 已由 O6R9 关闭；
- O6R11 的 pending renew normal-stop blocker 已由 O6R12 修复；
- O6R12 在其开发自验收中报告 lease matrix 14/14，但本节点不得预设 PASS；
- transaction 9、lease 14 两个测试文件均超过 800 行；需检查它们是否仍是单一矩阵 owner、是否复用
  frozen fixture，不能只按测试通过接收。

## 候选与角色边界

这是 O6 子阶段的冻结验收候选，不是整个 F445H 或 Phase 05 稳定候选。验收 Agent 只读检查 production
与 tests；唯一允许写入：

- `P5-F445H-O6R13-db-actor-combined-acceptance-result.md`

不得修改任何 Rust、fixture、既有 task/result、Cargo、manifest 或 lockfile；不得顺手修问题。发现
blocker 时给出 `FAIL`、精确路径和最小复现，不创建修复。

风险：高（Actor actual-Pending、transaction/lease 生命周期、outer drop）。

## 独立代码审计

必须从当前代码而非开发总结核对：

1. `program_db::wait::await_operation` 对 Actor frame 使用 E3 `await_if_pending`，只在真实 Pending
   释放/恢复 segment；非 Actor 只 await 原 future；
2. `EvalContext` 五个 DB expression arm 不再外层 pre-suspend/resume；其它 owner 未被顺手删除；
3. raw 与 prepared operation 只构造一次 owned future；prepared finalizer 只在恢复后进入 caller
   heap；query 保持无 I/O direct evaluation；
4. legacy/explicit transaction 都进入同一 `TransactionLifecycle`：
   - begin失败不 abort；
   - body/非法 flow选择一次 abort并保留原错误；
   - commit失败只 commit一次并选择一次 abort；
   - outer drop不启动 detached terminal cleanup；
5. lease：
   - claim None不 binding/renew/release；
   - binding只在 claim success后导入；
   - normal/body-error/illegal-flow先 stop/join renew，再 Lost→Release；
   - pending renew与 stop biased竞争，normal stop drop同一 renew future后 join；
   - outer Drop仍 abort唯一 renew task，之后不 detach、不增加 poll；
   - Release/Read outer drop不重建 future、不物化晚到结果；
6. 没有新增公开 request cancel、`CancelError`、cleanup acknowledgement 或 exactly-once 承诺；
7. fixture 仍只有一个 linked program/executable/fake owner，child files没有复制 production lifecycle；
8. 审查 800+ 行 transaction/lease 测试文件：
   - case driver、断言 helper与行为表是否职责清晰；
   - 是否存在明显重复的 linked program/fake/lifecycle或可导致语义漂移的两套机制；
   - 仅文件较长但仍是单一矩阵 owner不单独判 FAIL；若只是可读性改进，记为 non-blocking follow-up。

## 证据覆盖与 gate

先做命令/selector前置确认，再依次运行。所有命令使用独立 target dir：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r13-combined/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r13-combined/build/cargo-target \
  cargo test -p skiff-runtime-eval db_actor_ -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r13-combined/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r13-combined/build/cargo-target \
  cargo check -p skiff-runtime-eval -p skiff-runtime-service-db \
    -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r13-combined/build/cargo-target \
  cargo fmt --check
git diff --check
```

非零门槛：

- `program_db::tests::` 至少 36 个：fixture 1 + ordinary 12 + transaction 9 + lease 14；
- `db_actor_` 必须包含全部新增命名测试并非零；
- 完整 eval 必须执行 unit、integration 与 doc tests，记录实际数；
- selector 的后续 integration binary 零匹配不影响判定，但主 unit selector不得为零。

不要重复 O6R12 的单个精确 selector；上述组合 selector已覆盖。完整 eval gate只由本节点运行一次。
不得运行 stage gate、stable、live、network 或 MongoDB。

## Verdict

结果必须包含：

- `PASS` 或 `FAIL`；
- blocking issues；
- non-blocking follow-up；
- production 审计路径与反向搜索；
- 每条命令实际测试数、结果和精确候选；
- residual risk；
- 若 PASS，明确只关闭 O6 DB/Actor acceptance，并说明后续 E4R/J1 仍未完成。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r13-combined
branch   codex/p5-f445h-o6r13-combined
```

只提交 result 文档；worktree clean。不得 merge/rebase/push。不得派子 Agent。
