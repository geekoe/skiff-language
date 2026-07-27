# P5-F445H-E2 Lane-local DAG scheduler

状态：Ready。F445H 的高风险并发调度叶子；完成后与 E3 汇合到 E4。

## 直接父节点

- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-eval-concurrency-owner-preflight-result.md`

父节点已沿引用链连接唯一权威设计。本任务不得重新解释 timeout、concurrent、错误 owner 或
Actor 语义。

## 当前检查点与依赖

production prerequisite 为 Skiff integration `648627fe`：

- R0 已提供 `ExecutionScopeLease::child_execution_scope()`；
- E1 已提供 current owned execution control、child context、scope terminal 与统一 checkpoint；
- linked File IR v9 已提供 strict `LinkedConcurrentPlanIr`；
- E3 尚未提供 Actor continuation bridge，因此本任务不得执行 Actor lane；
- E4 尚未接 evaluator dispatch，因此本任务只交付 crate-private scheduler 与 hermetic fake
  executor，不修改四个 fail-closed bridge arm。

当前状态是实现检查点，不是稳定候选。本节点完成后只解除 E4 的 lane scheduler 前置。

## 生产目标

### 1. Lane-local env、heap 与 current scope

在 `env` 的 child module 中实现 lane-local state：

- concurrent 入口冻结一份 baseline `Env + RequestHeap`；
- 每个 lane 都从同一 baseline 独立 clone，禁止从已完成 lane 或共享可变 env/heap 派生；
- 每个 lane 获取独立 scope lease，并把
  `lease.child_execution_scope()` 安装为该 lane 的 current execution scope；
- lane control 继续共享 parent instruction/request budget，但
  `execution_scope()`、nested `derive_scope()` 与 owner-aware checkpoint 必须读取 lane current
  scope；
- normal completion 精确 complete lease；error、winner cancel、future drop 均取消 child
  scope；parent scope不受反向污染；
- scheduler 返回前，所有 lane lease/waiter/timer lifecycle counter归零。

若 current execution control 无法在本写集内以 crate-private wrapper 组合 child scope，停止并
返回 `TASK_SCOPE_EXPANDED`；不得修改 E1、capability-context、request 或 host。

### 2. Dependency-only handoff

增加窄的 eval-private slot snapshot/import API：

- export 由 `{source_order, slot, source_heap, carrier}` 明确描述；
- lane 只导入 `plan.dependencies` 指定的 normal predecessor export，按 source order处理；
- carrier 必须用 `deep_clone_runtime_value_carrier_between_heaps` 进入 destination lane heap；
- 不允许把 sibling 的临时 slot、未声明 dependency、整份 lane env 或 lane heap 合回；
- statement concurrent normal completion不修改 parent env/heap；
- value tail normal result由 scheduler深拷贝回 parent heap，late result永不导入；
- out-of-range slot、缺失 predecessor、重复 import或跨 heap handle异常一律
  `InvalidArtifact`。dependency只表示顺序时可以没有 export；此时合法地不导入 slot。

不得把 `SlotStore` 公开成任意 mutation API。

### 3. Runtime plan projection

消费 linked plan，不重算 source dependency：

- `source_order` 必须从 0 连续；
- dependencies 必须 strict sorted、unique、只指向 prior lane；
- `Statement` lane 的 body 必须解析为精确一个 direct statement；只有该 statement为直接
  `LinkedStmtIr::Let { slot, .. }` 时产生 sibling-visible export，其它合法 statement无 export；
- `Serial` lane永不 export；
- statement concurrent不得含 tail；value concurrent必须有且只有一个最终 tail，且 tail
  dependency closure已经包含全部前序 lane；
- impossible shape在启动任何 lane前 fail closed，不能退化为顺序执行。

允许复用 `program_ir` 的现有 block/statement lookup；不得新增第二套 linked IR resolver。

### 4. DAG 调度与确定性 winner

提供可由 E4 注入真实 lane evaluator future 的 crate-private scheduler seam。具体 Rust 类型由本
节点决定，但必须让 E4 无需复制 ready queue、winner、scope lease 或 heap handoff逻辑。

行为固定为：

1. 只有 dependencies 全部 normal 的 lane 才进入 ready queue；
2. 同一批 ready lane按 `source_order` 启动，彼此 future同时存活；
3. 每次接纳 lane结果前先执行 outer owner-aware checkpoint；
4. 一个 poll turn中收集全部 ready error，outer terminal优先；否则选最小 `source_order`；
5. winner确定后不再启动新 lane，先取消所有 running child scope，再丢弃/收束 running future；
6. winner后的 late value、late error、env/heap mutation全部丢弃；
7. value tail只有全部前序 lane normal后，且经过独立 tail-start checkpoint，才能启动；
8. statement plan全部 normal时只产生 normal completion；value plan只由 tail产生最终 carrier。

不得串行 `await` 每个 ready lane。不得通过线程并行证明 overlap；同一 task 内多个真实 Pending
future同时存活即可。

### 5. 结构

`env.rs` 已超过 400 行。production scheduler、plan projection、lane control和 tests必须按职责
放入 `env/**` child modules；root只允许 module/re-export和窄 `Env` 转发。

## Test-first 与验收

先新增真实 RED，再实现。至少覆盖：

- 两个无依赖 lane 都到达 barrier 后才统一 release，证明不是串行 await；
- dependency lane在 predecessor normal前不启动；
- 只导入 declared dependency，未声明 sibling slot和临时 heap值不可见；
- direct let export按 source order深拷贝，serial/普通 statement无 export；
- malformed body、slot、dependency、tail shape在零 lane启动时 fail closed；
- 同一 poll turn两个 error按 source order选 winner；
- outer cancel/deadline与 lane error同时 ready时 outer terminal优先；
- winner阻止尚未启动 lane，running child scope观察 cancel；
- late value/error/heap write不导入，drop/cleanup probe exactly once；
- tail fence和 tail result回传 parent heap；
- normal/error/cancel/drop后 scope lifecycle全部为零，parent current scope未改变；
- lane内再 derive timeout时继承 lane cancellation与 deadline owner。

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target \
  cargo test -p skiff-runtime-eval concurrent_scheduler -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target \
  cargo test -p skiff-runtime-eval program_execution_scope -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果文档必须记录每个 filter实际执行的测试数；零测试不算证据。完整 eval suite留给 E2/E3
合流后的 combined probe，不在本分支重复运行。

## 写集、非目标与停止规则

只允许：

- `runtime/eval/src/env.rs`
- `runtime/eval/src/env/**`
- 本 result

不得修改 `Cargo.toml`、`eval_context.rs`、`program_execution/**`、`error/**`、Actor、stream、
host/native、artifact/compiler/linker或 Router。

非目标：

- 不接四个 evaluator IR arm；
- 不物化 TimeoutError；
- 不迁移 maySuspend call sites；
- 不实现 Actor lane；
- 不运行 stable/live/network。

从启动到第一次测试代码修改不超过 5 分钟。若需要写集外 production owner、无法形成一个窄的
E4 consumer seam，或 scoped control 组合需要公共 API 变更，立即停止并精确报告
`TASK_SCOPE_EXPANDED`，不得继续研究或吞并 E4。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler
branch   codex/p5-f445h-e2-lane-scheduler
```

先提交 implementation，再只新增并提交
`P5-F445H-E2-lane-local-DAG-scheduler-result.md`。最终 clean；不得 merge/rebase/push。

任务合同已可执行，不应为填槽派子 Agent。只有出现一个会阻止正确实现的具体未知量时，最多派
一个只读有界子 Agent；该子 Agent不得再派 Agent，且探查后仍不明确必须停止上报。
