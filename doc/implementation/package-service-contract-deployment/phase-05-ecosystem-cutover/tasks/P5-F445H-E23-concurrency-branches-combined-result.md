# P5-F445H-E23 Concurrency branches combined acceptance result

状态：`ACCEPTED`。

执行时间：2026-07-28 CST。

## 1. 结论

E1、E2、E3 在固定 production 候选 `95bbf884` 上的组合接口通过独立审查和完整
`skiff-runtime-eval` suite。没有发现 production defect，也没有发现要求 E4 新增公共
capability、error、IR 或复制 scheduler / Actor 状态机的范围扩张。

本结论只解除 F445H-E4 的合流前置，不表示 F445H 已完成。四个 evaluator bridge arm 仍按合同
明确 fail closed；timeout、concurrent、catch、stream 与 actual-Pending 的 evaluator 接线仍属于
E4。

## 2. 精确候选与隔离

| 项 | 值 |
| --- | --- |
| production 候选 | `95bbf884` |
| production tree | `d55db0f2c66d9862efbc92ee946bb9a39f97ea2d` |
| 验收分支执行 HEAD | `7a34db02c9e6e6288b52393b698865a2cdd2f255` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e23-combined` |
| branch | `codex/p5-f445h-e23-combined` |
| 独立 Cargo target | `/Users/geek/workspace/skiff-p5-f445h-e23-combined/build/cargo-target` |

执行 HEAD 相对固定候选的唯一变化是新增任务文档：

```text
A doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-E23-concurrency-branches-combined.md
```

因此本次测试和审查的 production 与 `95bbf884` bit-identical。没有运行 stable、live、network
或其它仓库测试；没有修改 production、既有测试、父结果、配置或 lockfile。

## 3. 七项组合合同

### 3.1 Lane current scope 与 parent 委托

结论：PASS。

- scheduler 从 outer 的 current `OwnedExecutionControl` 和 current `ExecutionScope` 建立
  baseline；
- 每个 ready lane 都通过独立 `ExecutionScopeLease` 获得独立 child scope 和 child
  cancellation；
- `LaneExecutionControl` 的 scope、deadline、nested derive 和 terminal check 使用该 lane
  current scope，instruction units 与 request budget再委托 parent；
- child cancellation只取消该 lease child；parent cancellation / deadline由 child scope和
  scheduler 持续 poll 的 lease waiter观察，lane cancellation不会反向取消 parent。

对应完整 suite 中的 scope、distinct-control、outer-cancel wake、nested owner 与生命周期测试
全部通过。

### 3.2 Baseline、handoff 与失败原子性

结论：PASS。

- `ConcurrentBaseline` 一次冻结 parent `Env + RequestHeap`，每个 lane只操作自己的 clone；
- dependency import只深拷贝声明过的 direct-let export，不合并 predecessor env、heap或 sibling
  slot；
- dependency import失败只污染随后被丢弃的 lane-local heap；
- winner error materialization和 value tail handoff都先建立 parent heap checkpoint，任何深拷贝
  或 materialize失败均回滚 nodes 与 stats；
- statement normal completion不写 parent env / heap，late value/error不进入 parent。

特别复核了 `8fcad8f4` 的 correction：checkpoint位于 winner error materialization之前，失败分支
先 rollback再返回稳定 `InvalidArtifact`；成功分支保留正确物化的 local carrier。

### 3.3 Winner 后取消与 late result 隔离

结论：PASS。

- ready batch按 `source_order` 排序，outer checkpoint优先于 lane result；
- 同 poll turn 的多个 lane error只选择最小 source order；
- winner确定后不再进入下一轮 launch；
- `cancel_ready`、`cancel_running`、`cancel_evaluated` 均先 drop scope waiter，使 lease child
  cancellation生效，再 drop仍存活的 lane future；
- scheduler被直接 drop时，`RunningLane` 的字段 ownership顺序同样先释放 waiter，再释放 future；
- same-turn loser、pending loser与后继 lane均不再被接纳。

### 3.4 E2 lane 与 E3 Actor continuation

结论：PASS。

- `ActorConcurrentContinuationBridge` 为每个 lane创建独立 `ActorSuspensionState` 和独立 lease
  mutex；只共享不可变 continuation metadata；
- lane通过真实 `ActorInstanceStore::acquire_execution` 获取同步 segment，store scheduler继续
  保证同一 incarnation内同步 segment串行；
- `await_if_pending` 先 poll一次：`Ready` 保留当前 segment，只有实际 `Pending` 才 commit并释放；
- Pending外部 future可以同时存活，恢复时重新通过真实 store scheduler串行取得 segment；
- E4 可以按 lane source order直接消费 `bridge.lane(index)` 和
  `LaneExecutionState::program_context`，不需要共享 lease slot或复制 acquire / fence逻辑。

### 3.5 完成、放弃、drop 与 gate 收束

结论：PASS。

- child normal completion提交最后一个 segment并幂等关闭；
- abandon、lane drop与 bridge drop均 take/drop当前 lease并幂等关闭 child；
- acquiring future被取消时，resume permit把 child从 acquiring恢复到 suspended；已被 abandon时
  后续 acquisition不会安装 lease；
- scheduler normal路径 complete lease，error/cancel/drop路径先取消 lease child；
- nested bridge各自持有 gate，直接 parent child在内层结束前保持 open；
- outer `resume_parent` 在 remaining child非零时同步 fail closed，只在全部 child关闭后重新取得
  scheduler；
-完整 suite中的 store guard、scope lease、waiter和timer生命周期断言全部通过。

### 3.6 E1 current scope / clock 接缝与 E4 可接线性

结论：PASS。

- `ProgramExecutionContext` clone和 owned round-trip同时保留 current
  `OwnedExecutionControl` 与同一 `ExecutionClock`；
- `LaneExecutionState::program_context` clone parent context后只替换为 lane owned control，因此
  不会退回 request-root control，也不会重建 clock；
- E2已经提供 plan projection、lane executor、scheduler result和 lane context seam；
- E3已经提供 begin / lane / resume / complete / abandon / resume-parent seam；
- E4 只需实现真实 evaluator lane executor、安装对应 Actor child frame并替换四个 bridge arm，
  不需要新增公共 API或复制 scheduler / Actor状态机。

### 3.7 Fail-closed 与反向搜索

结论：PASS。

`runtime/eval/src/eval_context.rs` 中以下四个精确 arm仍返回稳定
`F445H-E4 evaluator integration is required ...` 的 `InvalidArtifact`：

- `LinkedStmtIr::Timeout`
- `LinkedStmtIr::Concurrent`
- `LinkedExprIr::Timeout`
- `LinkedExprIr::ConcurrentValue`

没有 wildcard、顺序执行 fallback或占位 normal value。对 E2/E3 新增 production执行：

```text
rg 'may_suspend|maySuspend|native_call_suspends|suspend_actor_segment' \
  runtime/eval/src/env/concurrent_plan.rs \
  runtime/eval/src/env/concurrent_scheduler.rs \
  runtime/eval/src/env/concurrent_scheduler/batch.rs \
  runtime/eval/src/env/lane_control.rs \
  runtime/eval/src/env/lane_state.rs \
  runtime/eval/src/actor_executor/actor_concurrent_continuation.rs \
  runtime/eval/src/actor_executor/actor_concurrent_continuation/bridge.rs
```

结果为空。测试 fixture里的 `may_suspend` 字段不是 production suspend决策，也没有进入新增模块。

## 4. 审查过的关键文件

- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/execution_scope.rs`
- `runtime/eval/src/error.rs`
- `runtime/eval/src/error/scope_terminal.rs`
- `runtime/eval/src/env.rs`
- `runtime/eval/src/env/concurrent_plan.rs`
- `runtime/eval/src/env/concurrent_scheduler.rs`
- `runtime/eval/src/env/concurrent_scheduler/batch.rs`
- `runtime/eval/src/env/lane_control.rs`
- `runtime/eval/src/env/lane_state.rs`
- `runtime/eval/src/env/slot_store.rs`
- `runtime/eval/src/actor_executor.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation/bridge.rs`
- `runtime/eval/src/eval_context.rs`
- E1/E2/E3 对应 scope、scheduler、Actor continuation测试文件。

## 5. 验证结果

所有 Cargo 命令均使用任务指定的独立 target。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked` | PASS：264 unit、10 integration、1 doc-test；共 275，0 failed、0 ignored |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

完整 eval suite的两个 integration binaries分别执行 4/4 与 6/6；doc-test执行 1/1。没有零测试
filter被计入证据。

输出只包含既有 compiler/source unused import、runtime/linker dead-code和
`service_error_channel.rs` unreachable-pattern warning；没有测试失败或本节点新增 warning。

## 6. 后继条件

F445H-E4 可以基于本结果继续 evaluator接线。E4 完成前不得把当前 fail-closed状态描述为
timeout / concurrent运行时已可用；本验收也不替代 E4 自身的 catch、stream、actual-Pending 与
完整生命周期测试。
