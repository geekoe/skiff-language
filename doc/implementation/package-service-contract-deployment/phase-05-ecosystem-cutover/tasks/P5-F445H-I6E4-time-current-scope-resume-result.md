# P5-F445H-I6E4 time sleep current-scope consumer resume result

状态：

```text
PARTIAL_IMPLEMENTATION_CHECKPOINT
TASK_SCOPE_EXPANDED = YES
TASK_NOT_EXECUTABLE = YES
I6_TIME_COMPLETE = NO
```

授权写集内的 time consumer 已形成可保留的 scoped implementation checkpoint：非零
`std.time.sleep` 从 E1 owned control 读取调用点 current scope，以一个真实 Tokio sleep future
和一个 scope lease 竞争；normal wake 先通过 completion owner 提交，scope terminal 则 drop
normal sleep 并返回内部 cancellation，让既有 Eval post-await checkpoint继续投影精确 owner。
零时长、decode/clamp 和同步 Date helper保持同步，旧 10ms execution-budget polling 已删除。

但是 E1 对既有 native prepared fixture 的机械跟随只补了 getter，没有让 fixture 提供
`ExecutionScope`。正确 consumer 在第一次真实 poll 读取 current scope 后，该既有相关测试不再
得到 Pending。修复需要修改本合同唯一写集之外的
`runtime/native/src/dispatch/prepared_tests.rs`，因此本节点不能宣称完成，也没有越界修改该文件。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| integration base commit | `e942efa99460ea2b9bf29f07d8dfe855c9715aff` |
| integration base tree | `46abc10c8fbdab6e70f2ea071539382dbf03a1be` |
| task publication HEAD | `8628b37cd0056c550ef62ab40a5aa3e54b06baab` |
| task publication tree | `754b9ac86ead0a6012e06720024a4ee9ced5ece0` |
| scoped implementation commit | `0f250dff41ec91a06c89a4716b029d69e6edc116` |
| scoped implementation tree | `a7a0065dfd6b9911025fe96db0e4aac23e377fa7` |

该 implementation commit 只修改合同授权的两个 production/test 文件，可作为修订合同后的
实现输入；它不是完整 I6-E4 候选。

## 2. 实现语义

`runtime/native/src/dispatch/time.rs::sleep_for_millis` 当前行为：

1. 保留既有一次 operation-start `poll_execution_budget()`；零时长随后立即 Ready，不读取
   scope、不 acquire lease。
2. 非零时长从 `NativeTimeCapability::execution_control()` 取得 E1 owned carrier，并读取唯一
   current `ExecutionScope`。
3. current scope建立一个 lease；requested duration只建立一个 normal Tokio sleep，不 derive
   第二个 scope deadline。
4. biased select把 normal branch放在前面；该 branch在返回前调用
   `ExecutionScopeLeaseCompletion::complete()`。已提交的 normal completion不能被随后 signal覆盖；
   scope已经 terminal时 completion拒绝提交并返回内部 cancellation。
5. scope branch只消费 `ExecutionScopeLeaseTerminal::Control`，drop normal sleep；`Completed`
   保持 completion-owner 不变量并标记 unreachable。
6. 所有 normal、deadline、ancestor/internal stop和零时长证据结束后
   `active_leases/active_waiters/active_timers` 均为零。

native error保持 `RuntimeError::Cancelled` 内部 terminal。Eval纵向 receipt证明真实 native
projection 到真实 sleep Pending；current local deadline后 scope保留
`LocalDeadlineExceeded` 精确 owner，既有 actual-pending post-await checkpoint因此仍是唯一
对外投影 owner。

## 3. RED / GREEN

真实 RED 建立在旧 polling implementation上：

| selector | listing | RED execution |
| --- | --- | --- |
| `f445h_i6_time_scope` | `5 tests` | `1 passed / 4 failed`；zero Ready是既有通过项，normal lifecycle、current deadline、outer deadline、ancestor stop失败 |
| `f445h_i6_time_projection_to_pending` | `1 test` | `0 passed / 1 failed`；projection后的真实 sleep没有 scope lease/timer/waiter |

完成实现与补齐同步/clock-stationary矩阵后的 GREEN：

| selector | listing | GREEN execution |
| --- | --- | --- |
| `f445h_i6_time_scope` | `7 tests` | `7 passed / 0 failed` |
| `f445h_i6_time_projection_to_pending` | `1 test` | `1 passed / 0 failed` |

Eval selector不是 getter test：它经过
`project_runtime_native_capability_context(Time)`、`NativeDispatch::prepare_resolved_native_call`
和 `PreparedNativeCall::ExternalWait`，第一次 poll真实观测 Pending与
`1 lease / 1 waiter / 1 timer`，current deadline后观测 native cancellation、精确 local owner和
全零 lifecycle。

## 4. current-scope / clock-stationary矩阵

| case | 触发与 winner | 结果 / owner | terminal lifecycle |
| --- | --- | --- | --- |
| normal wake | requested sleep timer | completion owner先提交 `Ok(())`；随后 ancestor signal不覆盖 | `0 / 0 / 0` |
| current deadline | current derived absolute deadline早于 requested duration | internal cancellation；`LocalDeadlineExceeded` | `0 / 0 / 0` |
| outer deadline | outer absolute deadline被 current scope继承 | internal cancellation；`InheritedDeadlineExceeded` | `0 / 0 / 0` |
| ancestor stop | requested timer不推进，ancestor token直接触发 | 立即 internal cancellation；`AncestorCancelled` | `0 / 0 / 0` |
| internal derived signal，clock stationary | 60s requested timer与 absolute deadline都不推进；直接触发 current deadline owner的共享 local signal | waiter立即醒，证明不依赖10ms poll或normal timer | `0 / 0 / 0` |
| zero duration | 首 poll前同步预算检查后直接 Ready | 无 scope owner | 始终 `0 / 0 / 0` |

每个非零 case 的记录型 budget counter均为 `1`，证明只有既有 operation-start检查，不存在旧
10ms循环。

## 5. 同步 helper 与反向搜索

`f445h_i6_time_scope_decode_clamp_and_sync_date_helper_stay_synchronous` 证明：

- negative duration clamp为零、超过上限 clamp到 `60_000`；
- fractional duration仍返回同步 decode error；
- `core.date.now` 不匹配 `TimeNativeDispatch`，仍由 `NativeRegistry::dispatch` 同步返回。

production反查：

```text
rg 'TIME_SLEEP_POLL_MILLIS|loop \{|yield_now|std::time::Instant::now' \
  runtime/native/src/dispatch/time.rs
  0 hits

rg 'poll_execution_budget\(\)' runtime/native/src/dispatch/time.rs
  1 hit
  runtime/native/src/dispatch/time.rs:109
```

没有新增语言 `yield`、Pending同步 helper、request-root snapshot、10ms timer或第二个 derived
deadline。没有修改 `NativeTimeCapability` getter、`RuntimeNativeInvocation`、artifact/std、
Cargo/lockfile或其它 dispatch。

## 6. 验证证据

| 层级 | 命令 | implementation tree结果 | 覆盖 |
| --- | --- | --- | --- |
| native list | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list` | PASS；`7 tests` | 非零 listing |
| native run | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture` | PASS；`7/7` | normal/current/outer/ancestor/internal/zero/sync |
| Eval list | `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list` | PASS；`1 test` | 非零纵向 listing |
| Eval run | `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture` | PASS；`1/1` | projection到真实 Pending |
| compile | `cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked` | PASS；仅既有 warnings | 两 crate接线 |
| format | `cargo fmt --check` | PASS | Rust格式 |
| diff | `git diff --check` | PASS | scoped写集 |

没有运行 full gate、stable/live/network/MongoDB，也没有 merge/rebase/push。

## 7. 合同 blocker 与最小后继

精确失败：

```text
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending \
  -- --nocapture

running 1 test
dispatch::prepared_tests::prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending ... FAILED
runtime/native/src/dispatch/prepared_tests.rs:242
assertion failed: matches!(poll_external_wait(&mut wait), Poll::Pending)
0 passed / 1 failed / 119 filtered out
```

调用链与原因：

```text
CountingTimeContext::execution_control
-> PreparedTestExecutionControl::owned
-> default OwnedExecutionControlApi::execution_scope
-> default ExecutionControlApi::execution_scope
-> ExecutionScopeAccessError::Unavailable
-> scoped std.time.sleep first poll returns InvalidArtifact instead of Pending
```

最小后继是一个纯机械 fixture任务，只授权
`runtime/native/src/dispatch/prepared_tests.rs`：

1. 让 `PreparedTestExecutionControl` 持有由其现有 cancellation token建立的 request
   `ExecutionScope`；
2. 在 borrowed/owned两个 test API实现中返回该同一个 scope；
3. 保持 `CountingTimeContext`、production trait、getter和 sleep实现不变；
4. 重跑上述失败测试、两个 I6-E4 selector与 locked two-crate check。

这不需要修改 production公共接口、Cargo、artifact/std、增加 yield或恢复 polling。合同未授权该
fixture跟随后，本 task按规则停止。

## 8. 实际写集

scoped implementation commit：

```text
runtime/native/src/dispatch/time.rs
runtime/eval/src/program_execution/execution_scope_tests.rs
```

result commit另增加：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6E4-time-current-scope-resume-result.md
```

```text
I6_TIME_COMPLETE = NO
```
