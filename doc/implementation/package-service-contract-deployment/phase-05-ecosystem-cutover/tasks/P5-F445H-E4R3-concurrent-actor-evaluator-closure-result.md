# P5-F445H-E4R3 concurrent and Actor evaluator closure result

状态：`READY_FOR_E4R5_CONCURRENT_ACTOR_INPUT`。

本节点已闭合 statement/value concurrent 的真实 evaluator lane executor，以及 outer Actor
存在时的 E3 continuation bridge。它只提供 E4R5 的 concurrent/Actor 输入，不代表 E4R、
F445H 或 Phase 05 完成。

## 1. 精确提交与写集

| 项 | 值 |
| --- | --- |
| production base | `b1faea534654c2ee2109f444a6cad6b1168b8445` |
| task branch base | `9def437a3b120f0fa28a1a9e676c01d3a7adc84b` |
| implementation commit | `57422ab1cd8a9b89cc45283a42506b0def006f32` |
| implementation tree | `7f02e7c9acac639804d70d655bfd5fd843953885` |
| result commit | 本文件独立提交；实际 hash 由交付消息记录，避免 commit 自引用 |

implementation commit 相对 task branch base 精确只修改：

- `runtime/eval/src/eval_context/concurrent.rs`
- `runtime/eval/src/eval_context/concurrent/tests.rs`
- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_concurrent.rs`

没有修改 root/module declaration、E2/E3 owner、timeout、actual-Pending、stream、program DB、
Cargo、manifest、lockfile或公共 API。

## 2. Test-first RED

在修改 production 前，先在 R1 fail-closed child 上新增真实 linked evaluator 测试并执行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r3-concurrent/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked \
  f445h_e4r_concurrent_statement_root_executes_direct_lane_body -- --nocapture
```

命令实际退出 `101`；测试从 `LinkedStmtIr::Concurrent` root arm 进入有效
`LinkedConcurrentPlanIr`，得到：

```text
InvalidArtifact("F445H-E4 evaluator integration is required for statement concurrent")
1 failed; 351 filtered out
```

这是真实 RED，不是编译失败、helper-only 测试或 malformed-plan 预期错误。production 接线后，
该场景扩展为两个无依赖 lane 的显式 Pending 重叠测试。

## 3. E2 / evaluator / E3 唯一消费序列

终态严格按以下顺序组合既有 owner：

1. statement/value root 分别调用 E2 `project_concurrent_plan`，不存在顺序执行 fallback；
2. outer context 有 Actor frame 且 plan 非空时，调用 E3 `begin_concurrent`；
3. E2 `run_concurrent_scheduler` 按 ready DAG 调用真实
   `EvaluatorConcurrentLaneExecutor::start_lane`；
4. executor 持有 owned `LaneExecutionState`，用
   `LaneExecutionState::program_context` 安装 lane-local current scope/control；
5. Actor lane 按 source order claim `bridge.lane(index)`，在 evaluator 前用 lane-local heap
   `resume`，再把唯一 child frame 安装到 child `ProgramExecutionContext`；
6. `Statement` 只执行 plan 已验证的唯一 direct statement，`Serial` 执行完整 block，
   `Tail` 求值 expression；
7. normal/value Actor lane 先用 lane-local heap clone 调用 E3 `complete`，再把原
   `LaneExecutionState` 及 outcome 交回 E2；
8. error、non-continue、resume/complete error 和 future drop 由 `abandon`/RAII 关闭 child，
   不伪造 normal；
9. scheduler winner/error/cancel 后，executor按 source order claim并 abandon所有未启动
   Actor child；running child由被 drop 的 lane future关闭；
10. 全部 child关闭后调用 `resume_parent`。resume/fence/outer terminal结果先于
    `scheduler_result` 返回，late lane result不会被接纳。

`ConcurrentLaneFuture` 保持既有
`Pin<Box<dyn Future<Output = LaneCompletion> + Send + 'a>>`。future只持有 owned lane state、
owned Actor lane和不可变 evaluator引用，不捕获 outer `&mut Env` / `&mut RequestHeap`，也没有
`?Send`。

production 没有复制 `acquire_execution` / `commit_execution`、ready queue、winner、
dependency import或 heap handoff状态机；这些仍分别由 E3/E2 owner唯一拥有。

## 4. Statement、value 与 Actor 语义

### 4.1 Statement / value

- statement 所有 normal lane结束后只返回 `Flow::Continue`；
- statement/serial lane出现 return、break、park、loop-continue或 continue-consumer时稳定
  `InvalidArtifact` fail closed；
- value concurrent 只接收最终 closed tail 的 carrier，E2负责深拷贝到 parent heap；
- malformed plan保持 E2 `InvalidArtifact`，测试确认 sink start 为零；
- serial dependency只在 prerequisite normal后启动，并执行 block内全部 statement；
- 同一 poll turn 的 error仍由 E2按 source order选择；
- outer cancellation与 lane completion同 turn时，outer checkpoint结果获胜；
- winner后 dependent lane不启动，running loser future被 drop，late gate和 heap write均不能
  进入 parent；
- outer scope lifecycle在 error/drop后恢复为零 active lease/waiter/timer。

### 4.2 Actor

- Ready emit在 child持有同步 segment时完成；两个无依赖 lane仍按真实 store scheduler串行，
  第二个 lane实际读取并断言第一个 lane提交的 Actor field；
- 只有 lane内部真实 external Pending 才释放 segment；两个 lane的显式 oneshot gate同时
  active，实测最大重叠为 `2`；
- 两个 Pending lane以反向 gate顺序完成，Actor field最终值证明每 lane独立 frame/store
  continuation并按恢复/提交顺序收敛；
- success使用 E3 `complete`；winner error会 abandon running child和未启动 child；
- success/error结束后 parent frame都重新持有 lease；error test在 `frame.finish` 后执行下一次
  真实 Actor method并成功取得 store scheduler，证明没有 child lease/guard泄漏。

所有并发测试使用明确 oneshot gate和首 poll，不用固定 sleep推断重叠。

## 5. 实际测试矩阵

selector实际列出 **11 个 Rust test function**：

| # | 测试 | 证据 |
| --- | --- | --- |
| 1 | `...statement_root_executes_direct_lanes_with_pending_overlap` | 真实 statement root、两个 direct lane，start `10,20`，Pending max `2` |
| 2 | `...serial_dependency_gates_and_runs_the_complete_block` | dependency前只有 lane 0启动；serial两条 statement依次完成 |
| 3 | `...value_tail_waits_for_fence_and_hands_heap_value_to_parent` | 真实 value root、tail fence、array carrier进入 parent heap |
| 4 | `...same_turn_errors_choose_source_order` | 两个 gate同 turn失败，winner固定为 source order 0 |
| 5 | `...outer_terminal_wins_over_same_turn_lane_completion` | gate completion与 outer cancel同 turn，返回 outer terminal |
| 6 | `...winner_stops_unstarted_lane_and_drops_running_loser` | dependent lane零 start，running loser出现明确 drop |
| 7 | `...loser_late_heap_write_isolated_and_outer_scope_restored` | late sender已失效，parent slot/heap不变，scope lifecycle归零 |
| 8 | `...malformed_and_noncontinue_lanes_fail_closed_without_fallback` | malformed零执行；return flow稳定 fail closed |
| 9 | `...actor_ready_lanes_keep_serial_segments_and_restore_parent` | real store串行、Ready send、跨 segment field观察、parent恢复 |
| 10 | `...actor_pending_lanes_overlap_with_independent_frames_and_parent_restore` | 两个 real-store child、Pending max `2`、反向完成、parent恢复 |
| 11 | `...actor_error_abandons_running_and_unstarted_children_without_lease_leak` | winner、running drop、unstarted abandon、下一 Actor调用成功 |

这 11 个测试全部穿过 statement/value真实 root arm、E2 projection/scheduler、真实 lane
evaluator；后三个同时穿过 E3 real-store bridge。没有用 E2 fake executor或 child helper计数。

## 6. 验证结果

所有 Cargo 命令使用任务指定的独立 target：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_concurrent -- --list` | PASS：`11 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_concurrent -- --nocapture` | PASS：`11 passed; 0 failed; 351 filtered out` |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只包含既有 compiler/linker unused/dead-code、ordinary tests unused import和
`service_error_channel.rs` unreachable-pattern warning；本节点 production/test无新增 warning。

反向检查确认：

- implementation commit文件列表只有第 1 节三个允许路径；
- production `?Send|ready_queue|acquire_execution|commit_execution` 搜索为空；
- `project_concurrent_plan -> run_concurrent_scheduler -> program_context -> E3 lane
  resume/complete/abandon -> resume_parent` 调用点均在唯一 `concurrent.rs` consumer；
- 两个测试文件实际函数计数为 `8 + 3 = 11`。

按合同没有运行完整 eval suite、其它 E4R selector、E2/E3 owner完整 gate、stable、live、
network或 MongoDB。

## 7. 未决问题与后继条件

本节点没有未决 blocker，且没有触发任务停止条件。现有 E2同时保留 lane state并完成 E3 heap
clone commit；future保持 `Send`；E3在 scheduler error/drop后可以关闭 child并恢复 parent；
不需要修改 root、E2/E3 owner或公共契约。

E4R5接入时必须以 implementation commit
`57422ab1cd8a9b89cc45283a42506b0def006f32` 作为 R3 输入，并重新验证 R2/R3/R4组合状态。
若 R1 root、E2 scheduler/lane handoff、E3 bridge/store或 shared Actor fixture发生变化，本结果
对应证据失效。未 merge、rebase或 push；未派子 Agent。
