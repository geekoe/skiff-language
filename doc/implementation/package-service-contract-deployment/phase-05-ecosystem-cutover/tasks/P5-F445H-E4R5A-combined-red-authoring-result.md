# P5-F445H-E4R5A combined RED authoring result

状态：`COMBINED_RED_AUTHORED / NOT_A_FINAL_VERDICT`。

本节点已在冻结的 R1-only production 语义上建立可编译、非零执行的 E4R combined RED。
tests commit 为
`69bb9c57a6b0ae4174f511626b36307207fbf5ca`；result 文档位于后续独立提交。

本状态只表示 R5A tests-only authoring 完成，不关闭 E4R、F445H 或 Phase 05，也不替代 R2/R3/R4
合流后的独立 R5B 验收。

## 1. Frozen base 与写集

任务指定的 R1 production base 为
`b1faea534654c2ee2109f444a6cad6b1168b8445`。本 worktree 开始 authoring 时 HEAD 为
`9def437a3b120f0fa28a1a9e676c01d3a7adc84b`；`b1faea53..9def437a` 只新增 R1 result 和
R1S/R2/R3/R4/R5A task 文档。对 `runtime/eval/src`、Cargo manifest 与 lockfile 的 diff
为空，因此执行语义仍精确等于 R1 implementation。

实际写集只有：

- `runtime/eval/tests/f445h_e4r_combined.rs`
- 本 result 文档

没有修改 production、既有 test/fixture、Cargo/manifest/lockfile，也没有 merge、rebase 或
push。

## 2. Combined matrix

新增 selector `f445h_e4r_combined` 下精确 5 个 Rust 测试：

| owner | 测试 | public / production 入口与断言 |
| --- | --- | --- |
| R1 | `f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable` | 通过 `ActorMethodExecutor` 执行真实 linked Actor method；`std.time.sleep(0)` 覆盖 first-Ready，`sleep(20)` 覆盖 actual-Pending，返回 `11`，并确认真实 evaluator checkpoint instruction units 至少为 `8`。 |
| R2 | `f445h_e4r_combined_r2_timeout_statement_and_expression_execute` | 同一真实 Actor executable 依次包含 timeout statement 与 timeout expression；期望最终返回 `1`。 |
| R3 | `f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute` | 同一真实 Actor executable 包含 concurrent statement 与 concurrent value；期望 Actor frame 内最终返回 `2`。 |
| R4 activation | `f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment` | 通过公开 compiler authoring、canonical artifact store、runtime assembly resolver 和 linker test-support 取得真实 `ActivationRelativeServiceCall`；故意不给 RuntimeAssembly target，使 production operation first-poll Ready 且 fail closed，再用同 Actor competitor 验证 Ready 路径不应预释放 segment。 |
| R4 stream | `f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end` | 通过公开 `Interpreter::exec_program_stream_for_in` 进入真实 pending `next()`；终结当前 child scope 后要求 wait 结束，并在非-End drop/terminal 上确认 consumer cleanup cancel 精确一次。 |

测试只构造 linked IR、能力 adapter、确定性 gate 和 public artifact/linker fixture；没有复制
E1/E2/E3/O1–O6 production 状态机，没有直接调用 private child helper，也没有
`assert!(false)`、ignored test、文本搜索伪造或 compile-fail RED。

该 integration 文件为 `2015` 行，主要体积来自唯一写集约束下所需的 capability adapters 与
真实 Actor/program/artifact wiring；所有五条测试共用一个 runtime/context 和 Actor harness，
没有第二套 suspension、activation 或 stream lifecycle。后续若需要继续扩大此 matrix，应另开
test-structure 节点评估公开 integration harness，而不应继续复制 wiring。

## 3. Listing

任务指定的 listing 命令 exit `0`，实际输出：

```text
f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable: test
f445h_e4r_combined_r2_timeout_statement_and_expression_execute: test
f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute: test
f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment: test
f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end: test

5 tests, 0 benchmarks
```

因此 selector 非零，且没有用 helper-only unit 充数。

## 4. R1 GREEN 与逐 owner RED

最终 exact execution 命令整体按预期 exit `101`：

```text
test result: FAILED. 1 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out
```

### R1 GREEN

```text
test f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable ... ok
```

该测试实际完成 first-Ready、actual-Pending/reacquire 和 checkpoint 路径，证明 selector 不是
“所有表面都坏”的假 RED。

### R2 timeout RED

精确失败：

```text
R2 expected timeout statement + expression success; production returned Actor method execution failed:
F445H-E4 evaluator integration is required for statement timeout
```

production 原因是 R1 frozen `timeout` child 对 statement 仍保留稳定 fail-closed diagnostic。
测试已经进入真实 Actor evaluator；不是 fixture、编译或环境失败。R2 实现 statement 后会继续执行
同一 executable 的 timeout expression，因此该测试也保留 expression 验收。

### R3 concurrent RED

精确失败：

```text
R3 expected concurrent statement + value inside a real Actor frame; production returned Actor method execution failed:
F445H-E4 evaluator integration is required for statement concurrent
```

production 原因是 R1 frozen `concurrent` child 对 statement 仍保留稳定 fail-closed
diagnostic。测试已经进入真实 Actor frame；R3 实现 statement 后会继续执行同一 executable 的
concurrent value。

### R4 activation RED

精确失败：

```text
R4 expected first-Ready activation failure to retain the Actor segment;
R1 pre-suspend let the queued competitor run first
```

在该断言前，真实 compiler/linker 构造已成功，activation operation 也已返回预期
`no runtime assembly target` fail-closed error；因此这不是 artifact/fixture panic。R1
activation child 在第一次 poll 前先释放 Actor segment，queued competitor 先取得 segment；
first-Ready activation 必须等待 competitor 后才能恢复，触发上述顺序 RED。

### R4 stream RED

精确失败：

```text
R4 expected current child scope to terminate pending next() before cleanup;
harness timeout while stream next remained pending; next received 1 cancellation token(s)
```

真实 `next()` 已进入 Pending；终结 current child scope 后，R1 stream consumer 仍只观察到一个
旧 cancellation token，未消费完整 current execution scope，所以 bounded wait 超时。随后
drop 的 non-End cleanup `cancel` 计数精确为 `1`，该 cleanup 断言已先通过；RED 来自 production
scope propagation，而不是悬挂测试或缺失 cleanup fixture。

四个失败均由目标 production 行为或冻结 diagnostic 触发；没有 fixture、compile 或环境 owner。

## 5. 合同命令

所有 Cargo 命令均使用独立 target
`/Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red/build/cargo-target`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list` | PASS；精确 `5 tests, 0 benchmarks`。 |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture` | 预期 RED；`1 passed; 4 failed; 0 ignored`，R2/R3/R4 各有上述精确 production 失败。 |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS，exit `0`。 |
| `cargo fmt --check` | PASS，exit `0`，无输出。 |
| `git diff --check` | PASS，exit `0`，无输出。 |

Cargo 仍输出写集外 compiler/linker 的既有 unused/dead-code warning、写集外 ordinary test 的
unused import，以及写集外 service error channel 的 unreachable-pattern warning；新增 test
文件无 warning。

按合同未运行完整 eval suite，未运行 stable、live、network 或 MongoDB，也未启动任何本地
服务。

## 6. Handoff

- R2 应让 timeout statement/expression 通过真实 evaluator，而不是修改本 RED 的期望。
- R3 应让 concurrent statement/value 在 Actor frame 中通过，而不是绕过 fail-closed
  diagnostic。
- R4 应消除 activation first-poll 前的预释放，并让 stream wait 观察 current child scope，
  同时保持非-End cleanup exactly once。
- R2/R3/R4 合流后必须由新的 R5B Agent 重跑同一 selector；本测试作者不得担任该独立验收。

任何生产入口或 linked IR shape 变化若使本 matrix 需要调整，应由独立 test-fix 节点处理，不能在
R5B 中顺手放宽断言。
