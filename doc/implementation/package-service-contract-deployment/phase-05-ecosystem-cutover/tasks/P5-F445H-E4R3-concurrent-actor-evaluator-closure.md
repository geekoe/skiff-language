# P5-F445H-E4R3 concurrent and Actor evaluator closure

状态：Ready。E4R 第二波 concurrent 叶子；可与 R2/R4 并行。完成后只提供 R5 的
concurrent/Actor 输入，不代表 E4R/F445H 完成。

## 直接父节点与精确代码状态

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`
- `P5-F445H-E23-concurrency-branches-combined-result.md`

本任务文件完整描述本节点需求。production base 为 R1 implementation
`b1faea534654c2ee2109f444a6cad6b1168b8445`。R1 已把 statement/value concurrent 两个 root
arm 薄转发到唯一 child，并预声明两个测试 child；不得再编辑 root/module declaration。

E2 已冻结 linked plan projection、lane-local env/heap/current scope、DAG scheduler、确定性
winner和失败原子的 heap handoff；E3 已冻结每 lane独立 Actor continuation、actual-Pending
suspend、complete/abandon/drop和 outer gate。本节点只消费这些 seam，不复制或修改 owner。

## 唯一写集

Production：

- `runtime/eval/src/eval_context/concurrent.rs`

Tests：

- `runtime/eval/src/eval_context/concurrent/tests.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_concurrent.rs`

交付文档：

- 新增 `P5-F445H-E4R3-concurrent-actor-evaluator-closure-result.md`

不得修改：

- `eval_context.rs` 或其它 child/module declaration；
- E2/E3 production/tests；
- timeout、actual-Pending、stream、program DB；
- outer fixture root或 R1 actual-Pending测试；
- Cargo、manifest、lockfile、公共 API或其它任务/result。

## Production 唯一消费序列

`concurrent.rs` 定义真实 `ConcurrentLaneExecutor`，必须按以下顺序消费既有 owner：

1. `project_concurrent_plan` 解析真实 linked plan；
2. `run_concurrent_scheduler` 在 lane ready 时调用真实 executor `start_lane`；
3. executor 从 `LaneExecutionState::program_context` 构造 child context；
4. outer context有 Actor frame时，按 source order claim `bridge.lane(index)`，在真实 evaluator
   前 `resume` 并安装 child frame；
5. `Statement` lane执行 plan指定的唯一 direct statement body；
6. `Serial` lane按既有顺序执行完整 block；
7. `Tail` lane求值 expression；
8. normal/value后 E3 `complete`，再把 E2 `LaneCompletion` 交回 scheduler；
9. error、非 continue flow、cancel、winner loser和drop使用 `abandon`/RAII，不伪造 normal；
10. scheduler全部结束、child关闭后，outer `resume_parent`；resume/fence/outer terminal优先于
    接纳 late lane result。

lane future 必须持有 lane-local state并满足既有
`Pin<Box<dyn Future<Output = LaneCompletion> + Send + 'a>>`，不能捕获 outer `&mut Env` /
`&mut RequestHeap`，不能降级 `?Send`。

Actor normal lane 可以从 lane-local heap clone commit snapshot给 E3，同时把原
`LaneExecutionState` 交 E2 handoff；不得共享旧单 frame/lease slot，也不得复制 store acquire、
identity fence、ready queue、winner、dependency import或 heap handoff状态机。

## Statement/value 语义

- statement concurrent 全部 normal 后返回 `Flow::Continue`；
- lane return/break/park等非-continue flow不得静默丢失；若 linked contract禁止，应稳定
  fail closed；
- value concurrent 只在 tail normal完成后返回 tail carrier；
- malformed plan保持 E2 `InvalidArtifact`，不得顺序执行 fallback；
- dependency lane只有 prerequisites成功后启动；
- 同一 turn错误选择保持 source order；
- winner阻止未启动 lane并终止 running loser；
- winner后 late result、late heap write或Actor commit不得进入 caller；
- outer cancel/timeout/terminal优先于同刻 lane completion；
- outer current scope与Actor frame在所有 child关闭后精确恢复。

同步 Actor segment仍串行；只有 lane内部真实 external `Pending` 可以释放并使多个 lane外部等待
重叠。第一次 poll `Ready` 不释放。

## Test-first 与最低矩阵

先在 R1 fail-closed child上新增真实 RED，再实现。selector：

```text
f445h_e4r_concurrent
```

listing/execution 至少有 **9 个实际 Rust 测试函数**，分别覆盖：

1. statement direct body与两个无依赖 lane真实 Pending重叠；
2. serial dependency gating；
3. value tail求值、tail fence和返回值；
4. 同 turn错误 source-order；
5. outer terminal优先；
6. winner阻止未启动 lane并取消 running child；
7. loser late result/heap write隔离，outer恢复；
8. Actor Ready不切 segment、同步段不重叠；
9. Actor Pending lane可重叠，每 lane独立 frame/store，结束后 parent恢复；
10. error/cancel/drop的 complete/abandon/lease归零，以及 malformed无顺序 fallback可参数化补齐。

测试必须穿过 statement/value真实 root arm、E2 scheduler、真实 lane evaluator和 E3 real-store
bridge；不能只调用 E2 fake executor或 child helper。Actor测试可复用 descendant private fixture，
不得开放 production visibility。

必须断言 lane start/completion顺序、pending重叠上限、winner、heap handoff、late写隔离、
E3 frame/lease计数和 parent恢复。不得以固定 sleep代替明确 gate。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_concurrent -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_concurrent -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际非零数；少于 9 不算完成。不运行完整 eval、其它 E4R selector、E2/E3完整 owner gate、
stable、live、network或 MongoDB。

result 必须记录 implementation/result commit、E2/E3消费序列、statement/value/Actor矩阵、
winner/late结果/parent恢复证据、实际测试数和验证结果。

## 停止条件

出现任一情况立即返回 `TASK_SCOPE_EXPANDED`，不得越界或派子 Agent：

- E2 无法同时保留 lane state与完成 E3 commit，且现有 heap clone不能保持语义；
- `ConcurrentLaneFuture + Send` 只能依赖 `?Send`、outer mutable borrow或修改 E2公共 API；
- E3 bridge无法在 scheduler error/drop后收束 child或恢复 parent；
- 正确实现需要修改 root、E2/E3 owner、公共契约或其它 production owner；
- 一次有界探查后仍有多个改变实现方向的未知量。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent
branch   codex/p5-f445h-e4r3-concurrent
```

不得派子 Agent。先提交 production/tests implementation，再单独提交 result；返回两个 commit、
矩阵、未决问题和 clean worktree。不得 merge、rebase或 push。

风险：高。开发自验收不替代 R5 combined acceptance；R1 root、E2/E3或 shared fixture变化会
使证据失效。
