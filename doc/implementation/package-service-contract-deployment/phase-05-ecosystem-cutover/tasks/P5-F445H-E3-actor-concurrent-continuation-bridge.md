# P5-F445H-E3 Actor concurrent continuation bridge

状态：Ready。F445H 的高风险 Actor composition叶子；与 E2并行，完成后汇合到 E4。

## 直接父节点

- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-eval-concurrency-owner-preflight-result.md`

父节点已沿引用链连接唯一权威设计。`maySuspend` 只是静态 summary；只有 future真实返回
`Pending` 才能释放 Actor executor。

## 当前检查点与依赖

production prerequisite 为 Skiff integration `648627fe`。

当前 `ActorExecutionFrame` clone共享一个
`Mutex<Option<ActorInstanceExecutionLease>>`，适合单 continuation，但不能让同一 Actor方法的
多个 concurrent lane共用：第一个 lane suspend后，另一个会遇到“without an execution
token”。`await_if_pending` 已正确执行 poll-once：ready不释放，真实 Pending才 suspend/resume。

本节点只建立多个独立 child continuation frame及 outer恢复栅栏。E2负责 lane env/heap/scope，
E4负责 evaluator call site与所有 preemptive suspend迁移。

## 生产目标

### 1. Outer 与 child continuation

在 `actor_executor` child module 中提供 crate-private bridge：

- concurrent入口把 parent当前同步 segment精确 commit/suspend一次；
- 为每个 lane产生独立 suspended child frame；它们共享同一个 store、handle、incarnation fence
  和 field type plans，但每个 frame有自己的 lease slot；
- child在执行同步 Actor代码前独立 acquire lease；
- Actor store现有 scheduler继续保证同一 incarnation任一时刻最多一个同步 segment；
- 一个 child真实 async wait进入 Pending并提交 segment后，另一个 child才可 acquire；两个外部
  async operation可以同时 pending；
- child normal completion提交最后同步 segment；error/cancel/drop释放其 lease，不泄漏
  scheduler guard；
- 所有 child同步 segment都已释放后，outer才允许 reacquire并继续；
- outer恢复读取最新 committed Actor fields，同时保留 outer continuation-local heap。

bridge API应让 E4能够按 lane索引取得 child frame、完成/放弃 child并恢复 parent；E4不得复制
store acquire、incarnation fence或 active-child栅栏逻辑。

### 2. Actual-Pending 合同

保留并复用现有 poll-once语义：

- buffered/ready future在当前同步 segment内完成，不 commit、不释放、不 reacquire；
- 首次真实 `Pending` 才 commit/suspend；
- pending future ready后必须先 reacquire且通过 incarnation、execution budget/cancel检查，才可
 继续同步 Actor代码；
- 静态 `maySuspend` 不能参与本 bridge的运行时释放决定。

本节点不修改 `eval_context.rs` 中旧的 preemptive suspend call sites；E4统一迁移。

### 3. Drop、winner 与 fence

- lane handle/drop必须是 fail-safe：持有 lease时 drop只 rollback该未提交 segment并释放
  scheduler guard；已经提交/悬挂时不制造第二次 commit；
- lane error/cancel后 active-child计数归零；
- outer在仍有 child持有同步 segment时恢复必须 fail closed，不能等待中偷偷共享 parent lease；
- stale epoch / instance replacement继续返回既有错误，且不得重新安装 lease；
- child不能绕过 Actor field codec或直接共享 persistent heap；
- 不改变 `ActorInstanceStore`、`ActorInstanceExecutionLease` 或公开 Actor ABI。

如果证明必须修改 `actor_instance.rs`，本节点立即以 `TASK_SCOPE_EXPANDED` 结束，保留已有有界提交
并报告所需 owner；不得越界实现。

### 4. 结构

`actor_executor.rs` 已超过 1200 行。新 production和测试责任必须分别进入
`actor_executor/**` child module；root只允许 module声明、薄转发和必要 crate-private re-export。
不得把新大段测试继续追加到现有 inline test module。

## Test-first 与验收

先新增真实 RED，再实现。至少覆盖：

- parent只 suspend一次，多个 child不共享 lease slot；
- 两个 child依次持有同步 lease，但各自进入 Pending后两个外部 future同时存活；
- buffered/ready operation不产生额外 commit/reacquire；
- pending resume前重新校验 budget/cancel和 incarnation；
- child normal completion提交字段，后续 child/outer看到最新值；
- child error、cancel、持 lease drop、已 suspend drop都释放资源且不双重 commit；
- outer在 active child segment存在时 fail closed；全部 child结束后可恢复；
- winner式取消所有 child后 outer可恢复，不出现“without an execution token”；
- stale epoch失败且 frame保持无 lease；
- outer continuation-local heap值在 suspend/resume后仍存在；
- 现有单 continuation `await_if_pending` 测试不回归。

测试必须使用真实 `ActorInstanceStore` scheduler和可控 oneshot/barrier future；只检查 mock计数不足以
证明同步 segment互斥与 async wait overlap。

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target \
  cargo test -p skiff-runtime-eval actor_concurrent_continuation -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target \
  cargo test -p skiff-runtime-eval actor_executor::tests -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果文档必须记录 filter实际测试数；零测试不算证据。完整 eval suite留给 E2/E3合流后的 combined
probe。

## 写集、非目标与停止规则

只允许：

- `runtime/eval/src/actor_executor.rs`
- `runtime/eval/src/actor_executor/**`
- 本 result

明确禁止修改：

- `runtime/eval/src/actor_instance.rs`
- `runtime/eval/src/eval_context.rs`
- env、program execution、stream、capability/request、host/native、artifact/compiler或 Router。

非目标：

- 不实现 lane scheduler或 heap dependency handoff；
- 不接 evaluator IR；
- 不迁移 service/DB/native/interface await call sites；
- 不改变 Actor公开语义、ABI或持久化格式；
- 不运行 stable/live/network。

从启动到第一次测试代码修改不超过 5 分钟。若 bridge需要 ActorInstance新公共能力、无法在现有
store/handle/lease契约内实现，或发现多个会改变实现方向的未知量，立即停止并上报
`TASK_SCOPE_EXPANDED`，不得顺手改 `actor_instance.rs`。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e3-actor-continuation
branch   codex/p5-f445h-e3-actor-continuation
```

先提交 implementation，再只新增并提交
`P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`。最终 clean；不得
merge/rebase/push。

任务合同已可执行，不应为填槽派子 Agent。只有出现一个具体未知量时，最多派一个只读有界子
Agent；该子 Agent不得再派 Agent，且探查后仍不明确必须停止上报。
