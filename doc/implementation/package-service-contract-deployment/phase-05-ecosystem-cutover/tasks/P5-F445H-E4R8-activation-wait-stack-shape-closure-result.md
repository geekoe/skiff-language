# P5-F445H-E4R8 activation wait stack-shape closure result

状态：

```text
ACTIVATION_WAIT_STACK_FIXED
FOCUSED_GREEN = YES
TASK_SCOPE_EXPANDED = NO
FULL_LIB_RUN = NO
```

activation-relative unary wait 进入 generic actual-Pending 链时的默认线程栈 blocker 已关闭。
目标 exact test 在默认 test threads 和显式单线程下均为 `1/1`，public 对照为 `1/1`；
activation unary 为 `3/3`，service-error consumer 模块为 `5/5`，E4R spine 为
`23 listed`、`23/23`。本节点没有运行完整 lib；它仍由 E4R7/E4R8 合流后的唯一 gate owner
执行一次。

## 1. 提交、候选与环境

| 项 | 值 |
| --- | --- |
| worktree 起始 HEAD | `e1cfcb4f633f4467513945be089bc6410a505cf8` |
| 任务冻结 production/tests commit | `464a3319b153527d5d33093d52ea6af97b6f997b` |
| implementation commit | `da49b0667713d3371d3fc5b46159e843a4724df2` |
| implementation tree | `32b220afa9aade06432b784fb1e343c1b0fbd236` |
| result commit | 本文件所在独立提交；精确 hash 由交付消息记录 |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r8-fix` |
| branch | `codex/p5-f445h-e4r8-fix` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target` |

所有 Cargo 验证均显式使用：

```text
env -u RUST_MIN_STACK -u RUSTFLAGS
CARGO_NET_OFFLINE=true
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target
```

因此没有提高 worker 栈、注入编译 flags 或访问 network。没有启动 stable/live、没有访问
MongoDB 或其它仓库，也没有 merge、rebase、push 或派子 Agent。

## 2. 唯一实现 diff

唯一 production diff 位于：

```text
runtime/eval/src/eval_context/actual_pending/activation.rs
```

`EvalContext::eval_activation_relative_service_call` 现在把同一个 wait future 放入 private pinned
heap box，再交给原 generic actual-Pending owner：

```rust
let wait = Box::pin(operation.wait());
let completed = self.await_actual_pending(wait).await?;
completed.finalize(self).map(Into::into)
```

implementation commit 的统计为：

```text
1 file changed, 2 insertions(+), 1 deletion(-)
```

相对 worktree 起始 HEAD，production diff 只有上述 call-site。prepared activation/provider
owner、service error channel、catch、tests/fixtures、通用 `actual_pending.rs` / E3、Cargo
manifest、lockfile和公共 API 均无修改。

private heap indirection使 `await_actual_pending` 的 generic future 参数只携带 pointer-sized
`Pin<Box<_>>`，不再把 concrete activation wait state 直接嵌入每层 async state；wait/finalize
owner和结果类型没有变化。

## 3. 默认栈 focused GREEN

目标 exact 与 public 对照使用直接父结果冻结的 fully-qualified test 名。任务文本中的短名与
`--exact` 组合实际匹配 `0 tests`，不能作为验收证据；同一 binary、环境和 target 随即以完整名称
重跑并取得下表真实计数。没有修改 test 名、attribute、断言或 fixture。

| 验证 | exit | 实际结果 |
| --- | ---: | --- |
| 目标 exact，默认 test threads | `0` | `1 passed / 0 failed / 394 filtered` |
| 目标 exact，`--test-threads=1` | `0` | `1 passed / 0 failed / 394 filtered` |
| linked-public 对照 exact，`--test-threads=1` | `0` | `1 passed / 0 failed / 394 filtered` |
| `f445h_e4r_stream_activation_unary`，单线程 | `0` | `3 passed / 0 failed / 392 filtered` |
| `assembly_execution::ordinary::tests::service_error_consumer`，单线程 | `0` | `5 passed / 0 failed / 390 filtered` |
| `f445h_e4r_spine -- --list` | `0` | `23 tests, 0 benchmarks` |
| `f445h_e4r_spine -- --nocapture` | `0` | `23 passed / 0 failed / 372 filtered` |
| `cargo check -p skiff-runtime-eval --tests --locked` | `0` | PASS |
| `cargo fmt --check` | `0` | PASS |
| `git diff --check` | `0` | PASS |

Cargo 输出只有候选已有的 compiler-source unused/dead-code、runtime-linker dead-code、
ordinary test-only unused import和 service-error channel unreachable-pattern warnings；本次
call-site diff没有新增 warning。

## 4. 语义保持

- target/contract 解析、argument prepare和 `ready_result` 分支均未改；同一个 operation 只构造
  一个 wait future，并只交给 `await_actual_pending` 一次。
- `completed.finalize(self)` 仍紧邻同一个 wait 之后执行一次，没有改动 prepared owner或重放
  unary invocation。
- first-Ready 仍由未改动的 E3/Actor owner保留当前 segment；activation Ready test通过。
- first-Pending 仍只由同一个 E3 owner释放 segment，完成后先 reacquire/fence，再返回本 call-site
  finalize；activation Pending test通过。
- provider failure仍先由 provider finalize固化一次，再由 caller import一次；activation failure
  test和 service-error consumer模块通过。
- linked public exact catch仍命中；unlinked public catch仍 miss并保持相同 opaque fixed bytes；
  private provider error仍固定为 `std.service.InternalError`并被 exact catch命中。目标 exact和
  service-error consumer `5/5`覆盖这些路径。
- three-hop rethrow 的 raw bytes、source、local stack和 correlation 路径未改，模块内
  `restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks`通过。
- finalize仍发生在 wait完成以及可能的 Actor恢复之后；caller heap failure atomicity、request
  owner和 drop生命周期均由未改动 owner保持。

## 5. 边界与未决项

没有触发 `TASK_SCOPE_EXPANDED`：call-site boxing足以关闭默认栈 failure，不需要唯一写集外改动。
任务文本的两个短 exact selector会得到零匹配；本节点按冻结父结果使用完整名称取得有效证据，
且因任务禁止修改其它文档，没有改写任务文件。该命令形状差异不影响 production结果。

按合同没有运行完整 `--lib`、完整 eval、combined、stable、live、network或 MongoDB 验证。
E4R7/E4R8 合流后的完整 lib仍由新 gate owner执行一次，本结果不签发该 gate。result提交前
tracked状态仅包含本文件；提交后再次确认 worktree clean。
