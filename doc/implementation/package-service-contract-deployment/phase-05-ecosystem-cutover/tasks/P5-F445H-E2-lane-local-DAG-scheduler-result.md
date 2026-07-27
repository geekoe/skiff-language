# P5-F445H-E2 Lane-local DAG scheduler result

状态：`IMPLEMENTATION_COMPLETE / E2_GREEN`。

E2 已交付 crate-private linked-plan projection、lane-local execution state、dependency-only handoff
与 DAG scheduler seam。实现没有接入 evaluator 的四个 bridge arm；真实 evaluator future 仍由 E4
注入。本节点没有实现 Actor lane，也没有修改 capability-context、program execution、error、
stream、host、request、artifact、compiler、linker 或 Router。

## 1. 输入与提交

| 项 | commit |
| --- | --- |
| production prerequisite | `648627fe` |
| task document | `141653da` |
| implementation | `bbb42e30` |

implementation 写集精确为：

- `runtime/eval/src/env.rs`
- `runtime/eval/src/env/**`

原 `env.rs` 中的 slot storage/layout 实现等价迁移到 `env/slot_store.rs`；公开的
`Env`、`SlotStore` 与 `SlotDebugBinding` 形状没有扩成任意 mutation API。`env.rs` 现在为
212 行，只保留 module/re-export、既有公开类型与窄转发。

## 2. Test-first 证据

先加入 scheduler contract test 与 fake executor，再运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target \
  cargo test -p skiff-runtime-eval concurrent_scheduler -- --nocapture
```

实现前该命令按预期 RED，exit `101`：测试无法解析尚不存在的
`super::concurrent_scheduler`。RED 来自缺失的 E2 production seam，不是既有 exhaustive match、
环境或写集外失败。随后才加入 production modules。

## 3. 实现结果

### 3.1 Lane-local state 与 current scope

`ConcurrentBaseline` 在入口冻结一次 `Env + RequestHeap`，每个 lane 都从该 baseline 独立
clone。每个 ready lane 获取独立 `ExecutionScopeLease`，并把
`child_execution_scope()` 与 child cancellation 安装到 lane-private control。

lane control 的 scope、deadline、nested `derive_scope()` 和 terminal check 都读取 lane current
scope；instruction units 与 request budget 委托给 parent control。`LaneExecutionState` 提供 E4
可用的 crate-private env、heap、control 与 `ProgramExecutionContext` child-context seam。

每个 running lane 同时持有 lane future 和 `lease.wait()` future。scheduler 在同一 poll turn
轮询两者，因此 outer cancel/deadline 即使遇到永远 Pending 的 lane 也会唤醒 scheduler。
normal path 先 `complete()` 再释放 waiter；error、winner cancel、scheduler drop 均先释放 waiter
取消 child scope，再丢弃 lane future。测试覆盖 active lease/waiter/timer 清零与 parent scope
不受反向污染。

### 3.2 Dependency-only handoff

normal direct-let export 明确保存：

```text
{ source_order, slot, source_heap, carrier }
```

consumer 只按 strict source-order dependency list 导入 normal predecessor 的 export，并使用
`deep_clone_runtime_value_carrier_between_heaps` 克隆到 destination lane heap。整份 predecessor
env/heap、未声明 sibling slot和临时 heap值都不会合并。重复 destination slot、越界 slot、缺失
direct-let export、错误 owner 与 stale cross-heap handle 全部 fail closed 为
`InvalidArtifact`。

statement normal completion不写 parent env/heap。value tail carrier 只在所有 lane normal后深拷贝
回 parent heap；clone 失败回滚 parent heap checkpoint。winner 后同 poll turn 的 late value/error
与 pending lane heap write 均被丢弃。

### 3.3 Linked plan projection

projection 直接消费 `LinkedConcurrentPlanIr`，并复用既有 `program_ir` block、statement 与
expression resolver。它验证：

- `source_order` 从 0 连续；
- dependencies strict sorted、unique 且只指向 prior lane；
- statement body 是精确一个 direct statement；
- 只有 direct `LinkedStmtIr::Let` 产生已校验的 export slot；
- serial 永不 export；
- statement plan 无 tail；
- value plan 有且只有最终 closed tail，且依赖全部 prior lane。

scheduler 在启动前再次验证 projected order/dependency/export/tail shape；impossible shape 不会
启动 fake 或后续真实 evaluator。

### 3.4 DAG、winner 与 tail fence

同一批 ready lane 按 source order启动，所有 future 同时存活，由单个 batch poll 收集当前
turn 的全部 ready result；没有逐 lane 串行 await，也没有线程并行。每次接纳 result 前先执行
outer `LaneEnd` checkpoint。

outer terminal 优先于 lane result；否则 ready error 中最小 `source_order` 获胜。winner 决定后
不再进入下一轮 launch，先取消 ready/running/evaluated child scope，再 drop loser future。
tail 只有依赖的所有 prior lane normal后才通过独立 `TailStart` checkpoint 启动。

E4 可直接消费以下 crate-private seam，无需复制 ready queue、winner、lease 或 heap handoff：

- `project_concurrent_plan`
- `ConcurrentPlan` / `ProjectedLane` / `LaneEvaluation`
- `ConcurrentLaneExecutor` / `ConcurrentLaneFuture`
- `LaneExecutionState` / `LaneCompletion`
- `run_concurrent_scheduler` / `ConcurrentSchedulerResult`

## 4. Contract test matrix

| 合同 | focused evidence |
| --- | --- |
| ready future overlap | 双 lane barrier；串行 await 会超时 |
| dependency gating | predecessor yield/normal 前 consumer 不启动 |
| dependency-only handoff | declared heap export deep clone；sibling/temp 不可见 |
| projection | direct let、ordinary statement、serial、tail与 malformed fixtures |
| fail closed | forward/duplicate dependency、tail shape、slot、missing export、duplicate import、stale handle |
| deterministic winner | 同 turn 双 error 选择最小 source order |
| outer priority | outer cancellation 与 ready lane error 同时发生时 outer 获胜 |
| pending wake | outer cancellation 唤醒永远 Pending 的 lane scope waiter |
| cancel/drop order | winner 与 scheduler drop 均让 running future先观察 child cancel，drop exactly once |
| late result isolation | loser error、same-turn late heap value与 pending heap write不进入 parent |
| tail fence | tail最后启动，结果深拷贝到 parent heap，checkpoint 顺序固定 |
| current control | lane scope/cancel flag各自独立，parent budget共享 |
| nested scope | lane内 derive 的 scope继承 lane cancellation与 deadline owner |
| lifecycle | normal/error/cancel/drop 后 lease/waiter/timer 全部为零 |

## 5. 验证

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e2-lane-scheduler/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval concurrent_scheduler -- --nocapture` | PASS：20/20 |
| `cargo test -p skiff-runtime-eval program_execution_scope -- --nocapture` | PASS：9/9 |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 compiler-source unused、linker dead-code、
`service_error_channel.rs` unreachable-pattern 和 ordinary test unused-import warnings；本节点没有
新增 warning。按任务合同，完整 eval suite 留给 E2/E3 合流后的 combined probe，本分支未重复
运行。

## 6. 边界与反向检查

`141653da..bbb42e30` 的 production/test diff 只包含 `runtime/eval/src/env.rs` 与
`runtime/eval/src/env/**`。对 `runtime/eval/src/eval_context.rs` 的 diff 为空；以下四个 E1
fail-closed arm 原样保留：

- `LinkedStmtIr::Timeout`
- `LinkedStmtIr::Concurrent`
- `LinkedExprIr::Timeout`
- `LinkedExprIr::ConcurrentValue`

没有 Actor/continuation、TimeoutError 物化、maySuspend migration、stable/live/network 验证，
也没有 public capability 或 error owner 变更。E2 只解除 E4 的 lane scheduler 前置；E4 仍负责
把真实 statement/serial/tail evaluator future注入本 seam，并替换四个 bridge arm。
