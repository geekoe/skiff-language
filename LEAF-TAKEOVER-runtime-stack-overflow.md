# LEAF-TAKEOVER — dev_todo3_stack_overflow（2026-08-02）

父文档：`/Users/geek/workspace/.task-contracts/HANDOFF-20260802.md` TODO-3、
`/Users/geek/workspace/.task-contracts/TAKEOVER-20260802.md` §5.4 第 3 项。
任务文件：本 worktree `TASK-runtime-stack-overflow.md`。

## 任务范围与结论

- 目标：修复 skiff runtime stack overflow，解除隔离 service 测试
  （`agine.ai/api`，复现点 `agent_bridge.chat_config`）的 503/Runtime disconnected blocker。
- 结论：**根因确认并已最小修复**。修复后同一 isolated service 测试不再 stack overflow、
  不再 SIGABRT、不再 Runtime disconnected；runtime 存活到 `agent_bridge.chat_config`
  测试点并返回独立已知基线失败（`409 AssemblyActivationRejected: test effect sequence
  exhausted ... std.http.request`），该失败与长命令 LEAF 记录的 SSE/effect 基线问题一致，
  不属本任务写集。

## Branch / worktree / commit

- branch：`fix/runtime-stack-overflow`
- worktree：`/Users/geek/workspace/skiff-runtime-stack-overflow`
- 基线：`55922196`（main HEAD，TODO-1 fixture 已合入）
- 实现 commit：`fix(runtime): run driver loop on configured worker stack`

## 根因（证据链）

1. 基线复现（`55922196`）：
   ```bash
   cd /Users/geek/workspace/internals
   SKIFF_ROOT=/Users/geek/workspace/skiff-runtime-stack-overflow \
   SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages \
   RUST_BACKTRACE=1 node scripts/test-isolated-service.mjs agine.ai/api
   ```
   → `[skiff-instance] runtime exited with SIGABRT; restarting`；
   `suite execution stopped at internal.agent_bridge.chat_config.__test::send ...:
   control request returned typed HTTP 503 AssemblyParticipantsUnavailable:
   Runtime disconnected before responding`；`[isolated runtime stderr] thread 'main'
   has overflowed its stack`。日志：`/tmp/todo3-baseline-repro.log`。
2. macOS crash report（`~/Library/Logs/DiagnosticReports/skiff-runtime-2026-08-02-120812.ips`）
   主线程栈：`runtime::main → Runtime::block_on → run_forever →
   run_connected_session_with_bootstrap → RouterSessionChildTasks::next (FuturesUnordered)
   → begin_actor_owner_invoke → execute_actor_owner_invoke → ActorMethodExecutor::execute →
   call_program_executable_with_self_direct → eval_program_call / eval_program_expr /
   eval_program_expr_ref 反复递归 → 原生栈耗尽。
3. 回归点：6dee837b/04743f6a 合流后，`actor.owner.invoke/control` 从 `tokio::spawn`
   （tokio worker，栈 = `RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`：debug 192 MiB /
   release 64 MiB）改为 session-owned 内联 child task（`RouterSessionChildTasks`，
   在 `runtime.block_on(run())` 所在的进程 main 线程上轮询，默认约 8 MiB）。
   debug 下 eval 每层约 1.04 MiB（`runtime/driver/config.rs` 注释与
   `doc/architecture/tail-call-execution.md` 记录），递归在到达 128 层
   `MAX_PROGRAM_CALL_DEPTH` guard 前已击穿 main 线程原生栈。

## 实现（写集）

- `runtime/driver/main.rs`：把 `runtime.block_on(run())` 移到
  `RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES` 栈大小的专用 `skiff-runtime-driver` 线程上，
  并 join 其结果；保持 router session 内联 child task 生命周期语义不变。
- `doc/architecture/tail-call-execution.md`：补充 router session driver 线程使用同一
  配置栈预算、不依赖 OS 默认 main 线程栈的说明。
- `TASK-runtime-stack-overflow.md`、`LEAF-TAKEOVER-runtime-stack-overflow.md`：
  本任务文档（baseline 已有 TODO-1 的 `TASK.md`/`LEAF-TAKEOVER.md`，故使用独立文件名，
  未覆盖他人文档）。

未改动：eval/actor/request 代码、`RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`、
session 内联语义、stable instance。

## 验证矩阵

| 验收项 | 命令 | 结果 |
| --- | --- | --- |
| 基线复现 | isolated `agine.ai/api`（见上，日志 `/tmp/todo3-baseline-repro.log`） | ❌ `thread 'main'` stack overflow → SIGABRT → 503 Runtime disconnected（证据 12:08 crash report） |
| 修复后不再崩溃 | 同一 isolated 命令（日志 `/tmp/todo3-fixed-verify.log`） | ✅ 无 stack overflow / SIGABRT / 503；runtime 存活到 `agent_bridge.chat_config`，仅剩已知基线 409 effect-sequence 失败 |
| 无新 crash report | `ls ~/Library/Logs/DiagnosticReports/` | ✅ 12:08 之后无新 skiff-runtime .ips |
| 编译 | `cargo check -p runtime` | ✅ 0 错误 |
| 相关 Rust selector | `cargo test -p skiff-runtime-host --lib '...package_direct_stream_producer_argument_real_gateway'` | ✅ 1 passed（真实 gateway 路径，4.33s） |
| 相关 Rust selector | `cargo test -p skiff-runtime-eval --lib '...ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses'` | ✅ 1 passed |
| rustfmt | `cargo fmt --check -p runtime` | ✅ clean（workspace 全量 fmt 存在未触碰文件的存量 diff，见下） |

备注：
- workspace 全量 `cargo fmt --check` 在未触碰文件（如
  `runtime/eval/src/actor_executor/tests.rs`）有存量 diff，与本改动无关，未顺手修复。
- 修复后同一测试点暴露的 409 `test effect sequence exhausted ... std.http.request`
  与 `/tmp/agine-main-exact-chat-config.log` 及长命令 LEAF 记录的 SSE/effect
  基线问题一致，建议作为独立平台/测试夹具节点处理。

## 交接清单（给 /root/skiff_integration_todo1）

- 分支 `fix/runtime-stack-overflow` @ 实现 commit（父 = `55922196`）。
- worktree `/Users/geek/workspace/skiff-runtime-stack-overflow`（含 `build/` 缓存，
  可随合并清理；集成探针建议：跑一次上面 isolated 命令确认无 SIGABRT/503，即可
  判定 blocker 解除；完整 suite 的 409 基线失败不属于本提交）。
- 本任务未改 stable instance、未 push、未动 main 工作树。
