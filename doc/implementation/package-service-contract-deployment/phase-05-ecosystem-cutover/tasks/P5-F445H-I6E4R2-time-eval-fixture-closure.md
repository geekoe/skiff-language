# P5-F445H-I6E4R2 Eval time fixture closure

状态：Ready。I6E4 production time current-scope consumer 与 I6E4R native prepared fixture 已形成
可保留检查点；剩余 blocker 仅是 Eval receipt fixture 把 current absolute deadline 固定为测试
起点后 5ms，导致 projection、prepare 与首次真实 poll 的墙钟开销可能先使 scope terminal。本节点只稳定
该 test-only Pending-before-deadline 边界，不改变 production 或 time 语义。

## 直接父节点与追溯

- `P5-F445H-I6E4R-time-prepared-fixture-closure-result.md`
- `P5-F445H-I6E4-time-current-scope-resume-result.md`

两个直接父结果分别固化 prepared fixture 检查点与 current-scope time consumer 检查点；其对应任务继续
引用 E1 shared carrier、I6D host operation 与 I6E invocation preflight，最终追溯到本目录
`AGENTS.md` 指定的唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。本任务只补充执行事实，不修改设计语义。

## 固定输入与 DAG

```text
baseline commit  c0967ace89d1c835f7f53d498ea7f95a48beadbb
baseline tree    1e281e03880ee182693c5fc79fb7ab1ddfd9079d
branch           codex/p5-f445h-i6e4r2-time-eval-fixture
worktree         /Users/geek/workspace/skiff-p5-f445h-i6e4r2-time-eval-fixture
integration      /root/phase05_integration_steward
```

当前为 I6 time 闭环的 test-only blocker 修复节点；前置 I6E4/I6E4R scoped implementation 均已在
baseline。完成后解除 I6 time combined integration probe，候选仍由集成 owner 决定是否推进，不能由本
节点自行冻结或验收。

## 实际 owner、写入范围与非目标

实际 fixture owner：

```text
runtime/eval/src/program_execution/execution_scope_tests.rs
```

任务与 result 证据 owner：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6E4R2-time-eval-fixture-closure.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6E4R2-time-eval-fixture-closure-result.md
```

只允许机械稳定
`f445h_i6_time_projection_to_pending_reaches_real_sleep_owner` 的 absolute-deadline fixture：
首次真实 poll 必须可靠观察 `Pending` 与 `1 lease / 1 waiter / 1 timer`，随后仍由 current scope
deadline owner 产生 internal cancellation、保留 `LocalDeadlineExceeded` 并归零 lifecycle。

不得修改 production、公共 API/ABI/wire/artifact、time sleep 语义、native prepared fixture、
其它测试、Cargo manifest/lockfile或兄弟 owner；不得新增 polling、fallback、全局/task-local side
channel或外部状态。

## RED、验证 owner 与完成标准

本开发 Agent 是下列聚焦证据的唯一 owner。先在未改 fixture 的 baseline 状态记录真实 RED，再修复并
运行：

```text
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

完成要求：Eval selector listing 非零且 GREEN；prepared-time 既有测试 `1/1`；native time selector
listing 非零且 GREEN；locked two-crate check、fmt 与 diff check 全部通过。证据只对最终
implementation commit/tree 有效；fixture、production time/scope、Cargo/lockfile或构建环境变化会使
相应动态证据失效。

## 风险、停止与交接

风险为低：test-only absolute deadline fixture。若正确闭合需要修改 production、公共契约、
Cargo/lockfile、native fixture、兄弟 owner、time/scope语义，或需要 stable/live/network/Mongo/full
gate，立即以 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE` 停止并上报。

task + fixture/tests 与 result 分开提交；result 必须记录 implementation/result commit/tree、真实
RED/GREEN、selector 计数、实际写集、自验收矩阵和 `I6_TIME_COMPLETE = YES/NO`。完成后 worktree
保持 clean，直接交接 `/root/phase05_integration_steward`；不得 merge、rebase、push 或清理一级
worktree。
