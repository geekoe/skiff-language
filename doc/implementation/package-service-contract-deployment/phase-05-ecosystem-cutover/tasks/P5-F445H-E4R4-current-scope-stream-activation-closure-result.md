# P5-F445H-E4R4 current-scope stream and activation closure result

状态：`IMPLEMENTATION_COMPLETE / E4R4_GREEN`。

本节点已闭合 activation 第九组 actual-Pending，并把 `program_stream`、两个 invocation
response-stream loop和 provider stream task/publication wait切到调用时的 current
`ExecutionScope`。它为 R5 提供 stream/activation 输入；不代表 E4R 或 F445H 整体完成。

## 1. 输入、提交与写集

| 项 | commit |
| --- | --- |
| R1 production base | `b1faea534654c2ee2109f444a6cad6b1168b8445` |
| 本 worktree integrated start | `9def437a` |
| E4R4 implementation | `bb64a182e28378854faeb1dc1046dc8c507e1d4c` |
| E4R4 result | 本文独立 result-only commit；精确 hash 由最终交付消息记录，避免 commit 自引用 |

implementation 严格位于任务唯一写集：

- activation：
  - `runtime/eval/src/eval_context/actual_pending/activation.rs`
  - `runtime/eval/src/eval_context/actual_pending/activation_tests.rs`
- program stream：
  - `runtime/eval/src/program_stream.rs`
  - `runtime/eval/src/program_stream/current_scope.rs`
  - `runtime/eval/src/program_stream/current_scope_tests.rs`
- invocation stream：
  - `runtime/eval/src/program_invocation.rs`
  - `runtime/eval/src/program_invocation/current_scope.rs`
  - `runtime/eval/src/program_invocation/current_scope_tests.rs`
  - `runtime/eval/src/program_invocation/stream_cleanup_tests.rs`
- provider stream：
  - `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
  - `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope.rs`
  - `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs`

没有修改 capability-context core、E1/E3/O3公共 API、request/host/native adapter、Cargo、
manifest、lockfile或其它 task/result。

## 2. Test-first RED

首先只加入真实
`f445h_e4r_stream_for_in_materializes_current_local_deadline_owner_before_wait`，让它穿过
`exec_program_stream_for_in`，使用已经过期且由 current child scope拥有的 deadline，以及真实
Pending stream wait。随后在旧 production上运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --nocapture
```

得到预期 RED，exit `101`，实际 `0 passed / 1 failed`：

```text
current local deadline must terminate before entering a pending stream wait
```

旧实现只传 generic request cancellation token，未在 wait入口读取 current scope，因此不能伪装
通过该测试。

## 3. Activation 第九组 actual-Pending

activation-relative service 现在按一个协议完成：

1. 当前同步 segment内解析 target、记录 internal dispatch、执行 current checkpoint并准备操作；
2. unary复用 O3 `PreparedProviderUnary`，owned wait仍为 `Send + 'static`，不借 caller
   heap/env/context/Actor frame；
3. `Ready` outcome直接 finalize，不调用 suspend/resume，也不释放 Actor segment；
4. unary第一次真实 `Pending`才进入 E3 `await_actual_pending`，由 E3 commit/释放；
5. E3恢复、重新 acquire/fence和 current checkpoint完成后才把 provider outcome finalize进 caller；
6. serverStream同步执行 `start_provider_stream`并返回 stream handle，不进入 unary wait，不预释放；
7. provider fixed failure在恢复后的 caller segment导入一次，保持原 internal-service-call错误语义。

旧 activation child中的显式 pre-suspend/resume已删除；没有静态 `maySuspend`判断、调用种类猜测或
新增 yield。drop/internal stop继续依赖 O3 request owner与 E3 continuation RAII。

测试通过真实 linked activation evaluator覆盖：

- unary first-poll `Ready`时排队的 Actor competitor保持 Pending；
- 显式 oneshot gate把 unary owned wait固定在真实 `Pending`，此时 competitor取得 segment；gate
  释放后，错误 finalize返回前原 frame已经重新取得 segment；
- provider failure只在实际 evaluator finalize路径导入；
- linked serverStream call第一次 poll同步 `Ready`，Actor competitor保持 Pending，并实际消费到
  runtime `End`。

## 4. 三处 current-scope 组合

三个新增 `current_scope.rs`只负责薄组合现有 scope、stream future和 checkpoint：

- `program_stream/current_scope.rs`
  - `exec_program_stream_for_in`每次 `next()`前读取
    `ProgramExecutionContext::execution_scope()`；
  - Actor frame下把组合后的 wait交给 `await_if_pending`，buffered/first-poll Ready不切 segment；
  - producer drain也使用相同 current scope，但不增加 instruction unit。
- `program_invocation/current_scope.rs`
  - runtime response loop和 binary/HTTP response loop每次真实 wait都重新读取 current scope；
  - 进入时保留原每 item一个 instruction unit，恢复后执行零 unit current checkpoint。
- `assembly_execution/async_stream_cancel/current_scope.rs`
  - unary/provider terminal、terminal publication和item publication从 borrowed/owned execution
    control读取其精确 current scope；
  - provider wait不再从 request-start token或 generic budget error反推 owner。

每个组合均使用：

- 完整 `cancellation_signals()`，包括 request、ancestor/local和 E2 lease-child signal；
- `effective_deadline()`保留 deadline及其精确 owner；
- wait前和 winner后通过 `terminal_at(Instant::now())`形成 internal carrier；
- biased顺序先 cancellation再 deadline，因此 ancestor/internal cancel与同刻 deadline竞争时
  cancel优先；
- 传给 stream runtime的额外 generic cancellation token数量为零，避免 request-root fallback；
- current scope不可取得时 fail closed，不降级为 root scope。

local/inherited deadline继续保留 `ScopeTerminalCarrier`。provider terminal识别该 carrier为 deadline，
不会把它错误导出成普通 service failure。

## 5. End / 非 End cleanup 矩阵

| terminal | `reached_end()` / disarm | 本地结果 |
| --- | --- | --- |
| runtime natural `StreamPoll::End` | 是，唯一 disarm路径 | cleanup count `0` |
| for-in `break` | 否 | guard drop，本地 cancel initiation恰好 `1` |
| for-in `return` | 否 | guard drop，本地 cancel initiation恰好 `1` |
| ordinary evaluator error | 否 | guard drop，本地 cancel initiation恰好 `1` |
| local/inherited timeout | 否 | 精确 scope carrier返回，guard drop恰好 `1` |
| ancestor/internal lease-child stop | 否 | cancel优先，guard drop恰好 `1` |
| caller future drop | 否 | drop同步发起 cleanup恰好 `1`，不等待远端 ack |
| binary HTTP logical `End` item | 否 | 它不是 runtime End；退出时 cleanup恰好 `1` |

future-drop fixture使用 owned oneshot gate：drop后 late internal item连同独立 provider heap无法发送给
caller，caller heap checkpoint/stats不变；late error同样无法进入已销毁的 wait。lease-child
terminal测试同时确认 lifecycle snapshot回到零 waiter/零 timer/零 active lease。

这里的“一次”仅指本地 cleanup initiation、terminal lease和本地计数不重复。异常 internal
stop/drop不承诺远端 cleanup acknowledgement、精确完成时刻或跨进程 exactly-once；没有新增业务
cancel API、request-id cancel或可 catch取消错误。

## 6. 实际测试矩阵

最终 selector listing为 **22个实际 Rust tests，0 benchmarks**，覆盖：

| 入口 | 实际覆盖 |
| --- | --- |
| activation evaluator | unary Ready、unary真实 Pending/reacquire、failure import、serverStream同步 Ready |
| `exec_program_stream_for_in` | local owner、natural End、break、return、ordinary error、future drop/late隔离、cancel-vs-deadline、Actor Ready、lease-child Pending |
| invocation runtime loop | local deadline owner、item后natural End |
| invocation binary loop | natural End disarm、logical HTTP End非natural cleanup |
| provider wait | terminal lease child、publication local owner、cancel优先、item publication lease child、完整 provider failure publication路径 |

所有 selector测试使用明确 oneshot gate、already-expired deterministic deadline或手动 first poll；没有
用固定 sleep决定测试行为。

## 7. 验证结果

所有 Cargo命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --list` | PASS：实际 `22 tests, 0 benchmarks`；其它两个 test binary各0个匹配测试，不计证据 |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --nocapture` | PASS：实际 `22 passed / 0 failed` |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 compiler-source/linker dead-code与unused-import、ordinary tests unused import和
`service_error_channel.rs` unreachable-pattern warning；本节点没有新增 warning。

added production diff反向搜索没有新增：

```text
suspend_actor_segment
resume_actor_segment
.suspend(
.resume(
maySuspend
yield_now
request_root / request-root fallback
cancellation_token() generic stream fallback
```

没有运行完整 eval suite、其它 E4R selector、O3完整 owner gate、stable、live、network或 MongoDB。

## 8. 收尾与后继

没有触发 `TASK_SCOPE_EXPANDED`停止条件；无需修改 capability-context cleanup core、远端 ack契约、
host/I6或 request-root fallback。implementation commit后仅新增本文 result；没有 merge、rebase或
push。R5仍需在 combined acceptance中验证 R1/E1/E3/O3/R4集成状态，本节点自验收不替代该验收。
