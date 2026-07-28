# P5-F445H-I6E4R prepared-time fixture closure result

状态：

```text
PARTIAL_FIXTURE_CHECKPOINT
TASK_SCOPE_EXPANDED = YES
I6_TIME_COMPLETE = NO
```

授权的 prepared-time fixture 已闭合：`PreparedTestExecutionControl` 现在由其原有
`CancellationToken` 建立真实 request `ExecutionScope`，borrowed/owned control API 均返回同一个
scope，deadline 也来自该 scope。既有 caller heap ownership 与首次真实 `Pending` 断言保持不变，
目标回归由 RED 变为 GREEN。

但是合同要求重验的 Eval selector 连续两次在首次 `Poll::Pending` 断言失败。该测试已经拥有正确的
current scope；失败来自另一个未授权 fixture 使用测试起点后仅 5ms 的 absolute current deadline，
在首次真实 poll 前已 terminal。修复需要修改
`runtime/eval/src/program_execution/execution_scope_tests.rs`，超出本任务唯一写集，因此未继续修改，
也未运行剩余的 locked check 或 full gate。

## 1. 候选身份与写集

| 项 | commit / tree |
| --- | --- |
| task 起始 commit | `f57738ae2a8aa9a9f624b0aef27ef34d4016d899` |
| task 起始 tree | `ede701824ff065601070990d0ad9cdb44517531f` |
| I6E4 implementation commit | `0f250dff41ec91a06c89a4716b029d69e6edc116` |
| I6E4 implementation tree | `a7a0065dfd6b9911025fe96db0e4aac23e377fa7` |
| fixture commit | `00f7fc31478a4171636b52049566a4264db1ffc2` |
| fixture tree | `68be35ba6464dade20e3e58212c7416a104b0fb0` |

fixture commit 的实际写集只有：

```text
runtime/native/src/dispatch/prepared_tests.rs
```

没有修改 production、E1 shared API、I6E4 sleep、其它 fixture、Cargo/lockfile；没有使用全局或
task-local side channel。

## 2. RED / GREEN

目标既有测试的原始 RED：

```text
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture

FAILED: 0 passed / 1 failed / 119 filtered out
runtime/native/src/dispatch/prepared_tests.rs:242
assertion failed: matches!(poll_external_wait(&mut wait), Poll::Pending)
```

授权 fixture 修复后的 GREEN：

```text
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture

PASS: 1 passed / 0 failed / 119 filtered out
```

这仍通过真实 `PreparedNativeCall::ExternalWait` 首次 poll 观察 `Pending`；caller heap 独立 mutation
断言未删除或改写。真实 scope lease 由 wait future 持有并在 fixture terminal/drop 时释放，没有
恢复 polling。

## 3. selector 与验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list` | PASS；`7 tests`，非零 |
| `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture` | PASS；`7/7` |
| `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list` | PASS；`1 test`，非零 |
| `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture` | FAIL；连续两次均为 `0/1`，首次 poll 未得到 `Pending` |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |
| `cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked` | 未运行；Eval selector 已证明需要未授权 fixture 修改，按停止条件终止 |

native selector 的 normal/current/outer/ancestor/internal/zero cases 均通过，并继续证明 terminal 后
lease/timer/waiter 为零。

## 4. 新 blocker 与最小后继

失败位置：

```text
runtime/eval/src/program_execution/execution_scope_tests.rs:1020
f445h_i6_time_projection_to_pending_reaches_real_sleep_owner
assertion failed: matches!(poll_time_pending(wait.as_mut()), Poll::Pending)
```

fixture 在测试起点记录 `base`，随后将 current deadline 固定为 `base + 5ms`，经过 context、
projection、dispatch prepare 后才首次 poll。当前环境连续两次在该 poll 前已越过 5ms，因此真实
sleep 正确观察 terminal，而测试无法先观察 Pending。该 control 的 borrowed/owned
`execution_scope()` 已返回同一个 current scope，不是本次 prepared fixture 的 getter 缺口。

最小后继应单独授权：

```text
runtime/eval/src/program_execution/execution_scope_tests.rs
```

后继只稳定该 Eval time projection fixture 的 Pending-before-deadline 时间边界，并重跑本合同中的
Eval selector与 locked two-crate check；不得修改 production sleep、scope API 或 native prepared
fixture。

没有运行 full gate、stable/live/network/MongoDB，也没有 merge/rebase/push。

```text
I6_TIME_COMPLETE = NO
```
