# P5-F445H-E4R6 callback stack-shape closure result

状态：

```text
CALLBACK_STACK_FIXED / FULL_LIB_BLOCKED_BY_DEADLINE_OWNER
ADDITIONAL_FULL_LIB_BLOCKER = ORDINARY_SERVICE_ERROR_CONSUMER_STACK_OWNER
LIB_GREEN = NO
TASK_SCOPE_EXPANDED = NO
E4R_COMPLETE = NO
```

callback actual-Pending 的默认 worker 栈 blocker 已关闭。两个 callback exact tests 在默认栈下
分别为 `1/1`，R1 spine 为 `23 listed`、`23/23`；唯一一次串行完整 lib 中，这两个 callback
tests 也都通过，没有再次 stack overflow。

完整 lib 仍不是 GREEN：preflight 已观察到的五条 `async_stream_cancel` deadline tests 全部失败，
随后写集外的 ordinary service-error consumer test 在默认栈发生另一个 stack overflow 并
`SIGABRT`。进程因此没有合法的 395-test 汇总。本节点保留 scoped callback 修复，不修改这两个
外部 owner，也不签发 E4R 完成。

## 1. 提交与候选身份

| 项 | 值 |
| --- | --- |
| 开始时 HEAD | `162cca24b553931f5c64b0c0616af820a6715c8d` |
| 冻结 production/tests commit | `da49c17cb6e3c479ea649b936aab8614d3beface` |
| implementation commit | `9a0d3c0be90ede6c55fe6170282606c814667cb7` |
| implementation tree | `2b8d8821e3e97fecc7987ca9705eee79fb485c1a` |
| result commit | 本文件所在独立提交；精确 hash 由交付消息记录 |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r6-fix` |
| branch | `codex/p5-f445h-e4r6-fix` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target` |

完整 lib 的唯一执行发生在 initial implementation commit
`761d1e893deed0772a0aa2452005ed84b06bdb13`、tree
`8f52ad220c9f37d6d84fc02dd0d1cf0aaef85eaa`。其后的首次 `cargo fmt --check` 只要求把
同一个 `await_actual_pending(wait).await?` 从三行收成一行；implementation commit 随即仅以该
rustfmt layout 变化 amend 为上表的 `9a0d3c0b`。`git diff 761d1e89..9a0d3c0b` 只有这一处
换行变化。合同限制完整 lib 只运行一次，因此没有重跑；两个 exact、spine listing/execution、
locked test check 和 fmt 均在最终 implementation tree 上重新通过。

## 2. 实现与语义保持

唯一 production diff 位于：

```text
runtime/eval/src/eval_context/actual_pending.rs
```

`EvalContext::eval_callback_interface_call` 现在先执行：

```rust
let wait = Box::pin(prepared.wait(&interpreter));
let completed = self.await_actual_pending(wait).await?;
```

因此 callback 的 concrete `PreparedCallbackInvocation::wait` future 位于 private pinned heap
box 中，通用 `await_actual_pending` / E3 future 链只携带 pointer-sized future。没有改变
callback owner 或公共类型。

语义保持依据：

- `prepared.wait(&interpreter)` 仍只构造一次；`PreparedCallbackInvocation` 被同一次调用消费，
  pinned wait 只被 `await_actual_pending` await 一次；
- `CompletedCallbackInvocation::finalize(self.heap)` 仍只在 wait 完成后执行一次；
- first-Ready 仍由未改动的 `ActorExecutionFrame::await_if_pending` 保留当前 segment；
- first-Pending 仍只由该通用 owner 释放，完成后先 reacquire/fence，再回到 caller heap
  finalize；
- drop pinned box 会 drop 同一个 wait future及其 owner guard；error output、request
  generation检查和 owner guard生命周期均未改；
- callback prepared owner、通用 E3、Actor frame、栈环境、tests、fixtures、Cargo和 lockfile
  均无 diff。

implementation commit 的实际统计为 `1 file changed, 2 insertions(+), 3 deletions(-)`；删除项仅是
原三行的 direct call，新增项是 private `Box::pin` 和格式化后的 await。相对冻结 production
commit，对 `runtime/eval/src`、`runtime/eval/tests`、Cargo manifests和 lockfile的唯一命中仍是
上述 call-site 文件。

## 3. 默认环境与 focused GREEN

开始时环境中没有 `RUST_MIN_STACK` 或 `RUSTFLAGS`。所有 Cargo 验证都显式使用
`env -u RUST_MIN_STACK`，没有设置自定义 test stack；同时设置 `CARGO_NET_OFFLINE=true`，确保
不访问 network。实际 target固定为上表独立目录。

最终 implementation tree上的结果：

| 验证 | exit | 结果 |
| --- | ---: | --- |
| Pending callback完整名称 + `--exact --nocapture` | `0` | `1 passed / 0 failed / 394 filtered`，约 `0.03s` |
| Ready callback完整名称 + `--exact --nocapture` | `0` | `1 passed / 0 failed / 394 filtered`，约 `0.00s` |
| `f445h_e4r_spine -- --list` | `0` | `23 tests, 0 benchmarks` |
| `f445h_e4r_spine -- --nocapture` | `0` | `23 passed / 0 failed / 0 ignored / 372 filtered` |
| `cargo check -p skiff-runtime-eval --tests --locked` | `0` | PASS |
| `cargo fmt --check` | `0` | PASS，无输出 |
| `git diff --check` | `0` | PASS，无输出 |

两个 exact tests 的完整名称为：

```text
actor_executor::tests::actor_concurrent_continuation::
evaluator_actual_pending::callback_matrix::
f445h_e4r_spine_callback_pending_reacquires_before_finalize

actor_executor::tests::actor_concurrent_continuation::
evaluator_actual_pending::callback_matrix::
f445h_e4r_spine_callback_ready_keeps_actor_segment
```

这两个结果分别证明 first-Pending 在 caller-heap finalize 前已 reacquire，以及 first-Ready
没有释放 Actor segment。spine 的其余 21 项也保持 GREEN。

## 4. 唯一一次串行完整 lib

命令为：

```bash
env -u RUST_MIN_STACK \
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib -- \
  --nocapture --test-threads=1
```

exit 为 `101`。libtest 打印 `running 395 tests`；本节点修复的 Pending/Ready callback tests
均打印 `ok`，没有 callback stack overflow。随后精确出现五个 preflight 已知 deadline
failure：

```text
assembly_execution::async_stream_cancel::tests::
  pending_provider_unary_wakes_from_deadline_and_cancels_provider_request
  provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout
  stream_item_deadline_remains_typed_through_provider_terminal
  stream_terminal_item_and_publication_deadlines_remain_typed
  terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout
```

实际失败分别为：

- `async_stream_cancel.rs:1140` 未得到预期的
  `ExecutionBudgetExceeded(DeadlineExceeded)`；
- `async_stream_cancel.rs:1410` 得到 `Cancelled`，而不是 typed deadline terminal；
- `async_stream_cancel.rs:1462` 得到 `End`，而不是 typed item deadline terminal；
- `async_stream_cancel.rs:1278` terminal不匹配 typed `DeadlineExceeded`；
- `async_stream_cancel.rs:1508` publication error不匹配
  `ExecutionBudgetExceeded(DeadlineExceeded)`。

这五项的唯一 owner 是写集外
`runtime/eval/src/assembly_execution/async_stream_cancel.rs` 的 deadline
terminal/publication语义。本任务没有修改或继续调查该 owner，故主状态为
`CALLBACK_STACK_FIXED / FULL_LIB_BLOCKED_BY_DEADLINE_OWNER`。

五项 failure 之后又出现：

```text
assembly_execution::ordinary::tests::service_error_consumer::
ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses
```

该 test 在默认 worker 栈发生 stack overflow，随后进程以 signal 6 `SIGABRT` 终止。当前证据只把
额外责任面冻结为 ordinary service-error consumer 的独立 default-stack closure；没有 backtrace、
栈阈值或 focused preflight，不能诚实地进一步归因到某个 production frame。它位于本 callback
call-site、callback prepared owner和 stream deadline owner之外，需要独立诊断。本任务没有运行
额外 full lib、没有提高栈，也没有修改该 test或 owner。

由于最后的 abort，不能把本次结果写成 `390/395` 或任何其它推算汇总；唯一合法结论是 inventory
为395、已观察到五个 deadline failures、随后一个额外 stack abort，完整 lib没有 summary。
abort之后的 tests也没有完整执行证据。

## 5. 边界、warnings 与未决项

静态 diff确认以下路径均未改：

- `runtime/eval/src/assembly_execution/callback_native/prepared.rs`；
- `ActorExecutionFrame` / 通用 `await_actual_pending` owner；
- callback及其它 tests/fixtures；
- Cargo manifests、`Cargo.lock`、公共 API；
- stack配置、Tokio/libtest属性。

`cargo check` 只报告候选已有 warnings：compiler source的 unused/dead-code、runtime linker的
dead-code、ordinary test-only unused import，以及 service error channel的 unreachable pattern。
本 call-site没有新增 warning。

未决 blockers：

1. 五条 `async_stream_cancel` deadline tests仍由 stream deadline owner处理；
2. ordinary service-error consumer 的 default-stack overflow需要独立 preflight；
3. 因完整 lib没有合法汇总，`LIB_GREEN`、`E4R_COMPLETE`和后续独立 R5C验收均不得签发。

本任务没有访问 stable/live、network、MongoDB或其它仓库，没有运行 combined或含
integration/doc的完整 eval gate，没有派子 Agent，也没有 merge、rebase或 push。result提交前
tracked状态只包含本文件；提交后再次检查 worktree为 clean。
