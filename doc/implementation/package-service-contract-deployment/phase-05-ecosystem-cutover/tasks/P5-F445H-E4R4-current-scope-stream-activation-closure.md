# P5-F445H-E4R4 current-scope stream and activation closure

状态：Ready。E4R 第二波 stream/activation 叶子；可与 R2/R3 并行。完成后提供 R5 的 stream
以及第九组 actual-Pending 输入，不代表 E4R/F445H 完成。

## 直接父节点与精确代码状态

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`
- `P5-F445H-O3-activation-relative-service-prepared-operation-result.md`

本任务文件完整描述本节点需求。production base 为 R1 implementation
`b1faea534654c2ee2109f444a6cad6b1168b8445`。R1 已把 activation-relative service 路由到
`actual_pending/activation.rs`，但明确保留旧 pre-suspend；R4 是该第九组和
`async_stream_cancel.rs` 的唯一 E4R 终态 owner。

E1 current `ExecutionScope` 已提供完整 cancellation signals、effective deadline/owner和
`terminal_at`；O3 已提供 activation-relative prepared wait/finalize与 stream lifecycle。
不得修改这些 owner或用 request-root fallback。

## 唯一写集

Production：

- `runtime/eval/src/eval_context/actual_pending/activation.rs`
- `runtime/eval/src/program_stream.rs`
- 新增 `runtime/eval/src/program_stream/current_scope.rs`
- `runtime/eval/src/program_invocation.rs`
- 新增 `runtime/eval/src/program_invocation/current_scope.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- 新增
  `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope.rs`

Tests：

- 新增 `runtime/eval/src/eval_context/actual_pending/activation_tests.rs`
- 新增 `runtime/eval/src/program_stream/current_scope_tests.rs`
- `runtime/eval/src/program_invocation/stream_cleanup_tests.rs`
- 新增 `runtime/eval/src/program_invocation/current_scope_tests.rs`
- 新增
  `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs`

交付文档：

- 新增 `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`

不得修改：

- `eval_context.rs`、R1 actual-Pending owner、timeout、concurrent；
- capability-context `StreamConsumerCleanup` / supervised lease core；
- E1/E3/O3公共或核心 API；
- request/host/native adapter、I6；
- Cargo、manifest、lockfile、其它任务/result。

## Activation 第九组 actual-Pending

`activation.rs` 必须原子完成：

1. activation-relative service prepare；
2. operation `Ready` 时留在当前 Actor segment，直接 finalize；
3. owned wait第一次真实 `Pending` 时才经 E3 `await_if_pending` commit/释放；
4. resume/acquire/fence和 current checkpoint 后 finalize；
5. error、internal stop、drop依靠 O3/E3 RAII，不复制状态机；
6. serverStream创建保持同步 `Ready`，不预释放。

删除该 child中的旧 `suspend_actor_segment` / `resume_actor_segment` 和静态
`maySuspend`/调用种类判断。不得影响 R1 已闭合的其它八组，也不得加入 yield。

## Stream current scope

`program_stream.rs`、`program_invocation.rs` 和
`assembly_execution/async_stream_cancel.rs` 的每个真实 stream wait必须读取调用时
`ProgramExecutionContext::execution_scope()`：

- 使用完整 `cancellation_signals()`，包括 request、ancestor/local和 E2 lease child；
- 使用 effective deadline及其精确 owner；
- 通过既有 `terminal_at` 保持 ancestor cancel优先于同刻 deadline；
- local/inherited deadline保留 internal carrier，交给对应 timeout/request owner；
- 不能从 generic budget error、request-start token或 request-root scope猜 current owner；
- Actor frame下 buffered/first-poll Ready `next()`不释放，只有真实 Pending才释放；
- wait进入/恢复使用既有 current checkpoint，不新增调度 yield。

三个新增 `current_scope.rs` 只组合现有 scope、stream future和 cleanup guard；root文件保留
dispatch/lifecycle和薄转发。四个 production root均已很长，不得继续堆叠大型 helper区。

## Stream terminal 与 cleanup

- natural `End` 是唯一调用 `reached_end()` / disarm 的路径；
- break、return、ordinary error、timeout、ancestor/internal stop和future drop都不得
  disarm，而应由既有 `StreamConsumerCleanup` / supervised lease / stream lifetime RAII触发
  cleanup；
- winner后 late item、late error或late heap write不能进入 caller；
- 用户可见 terminal不能无限等待不配合停止的producer；
- local owner恢复、waiter/timer/lease的本地状态必须收束。

“一次”只指本地 cleanup initiation、terminal lease和计数状态不重复；异常 internal stop/drop
不承诺远端 cleanup acknowledgement、精确完成时间或跨进程 exactly-once。不得新增业务层
cancel API、request-id cancel、`CancelError` 或可 catch 的取消错误。

需要覆盖：

- `exec_program_stream_for_in`；
- `program_invocation.rs` 两个 response-stream loop；
- provider stream task/publication wait；
- activation-relative unary与 serverStream创建。

`connection.send` 等 future第一次 poll Ready时不让出执行权；若底层真实 Pending，才走统一
E3逻辑。

## Test-first 与最低矩阵

先新增真实 RED，再实现。selector：

```text
f445h_e4r_stream
```

listing/execution 至少有 **8 个实际 Rust 测试函数**，必须穿过真实 activation evaluator call、
`exec_program_stream_for_in`、两个 invocation response loop和 provider wait中的对应入口；不能
只 drop cleanup guard或只测 child helper。

最低覆盖：

1. activation unary Ready不切 segment；
2. activation unary Pending释放并在 finalize前恢复；
3. activation serverStream setup同步 Ready；
4. buffered stream `next()` Ready不切，真实 Pending消费完整 child scope；
5. natural End唯一 disarm；
6. break与return各自触发本地 cleanup一次且不等待远端 ack；
7. ordinary error与timeout owner正确、cleanup状态收束；
8. ancestor cancel/内部 stop与同刻deadline竞争，cancel优先；
9. future drop触发 RAII，late item/error/write隔离；
10. invocation/provider current local/lease child scope的 signals、deadline owner可见。

End 与每类非-End terminal必须留下可区分的 cleanup断言，不能用一个“任一错误 drop”测试合并。
使用明确 gate/scripted clock，不依赖固定 sleep。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4-stream/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际非零数；少于 8 不算完成。不运行完整 eval、其它 E4R selector、O3完整 owner gate、
stable、live、network或 MongoDB。

result 必须记录 implementation/result commit、activation Ready/Pending/serverStream、三处
current-scope组合、End/非-End cleanup矩阵、本地一次语义与异常无ack保证、实际测试数和验证
结果。

## 停止条件

出现任一情况立即返回 `TASK_SCOPE_EXPANDED`，不得越界或派子 Agent：

- 完整 current signals/deadline无法用现有
  `ExecutionScope::{cancellation_signals,effective_deadline,terminal_at}` 组合；
- 本地单 terminal/cleanup initiation必须修改 capability-context core；
- 正确实现需要远端 cleanup ack或新增业务取消契约；
- provider stream正确性要求 host/I6或 request-root fallback；
- 正确实现需要修改 root、O3/E1/E3公共 API或其它 production owner；
- 一次有界探查后仍有多个改变实现方向的未知量。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r4-stream
branch   codex/p5-f445h-e4r4-stream
```

不得派子 Agent。先提交 production/tests implementation，再单独提交 result；返回两个 commit、
矩阵、未决问题和 clean worktree。不得 merge、rebase或 push。

风险：高。开发自验收不替代 R5 combined acceptance；R1 root、E1/E3/O3或 stream cleanup owner
变化会使证据失效。
