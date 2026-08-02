# TASK — TODO-3: fix skiff runtime stack overflow (isolated service test blocker)

父文档：`/Users/geek/workspace/.task-contracts/HANDOFF-20260802.md` TODO-3、
`/Users/geek/workspace/.task-contracts/TAKEOVER-20260802.md` §5.4 第 3 项。

## 目标

隔离 service 测试（`agine.ai/api`，复现点在 `agent_bridge.chat_config` / `agent_bridge_host_wake`）
不再因 runtime `thread 'main'` stack overflow 崩溃导致 503 AssemblyParticipantsUnavailable /
Runtime disconnected。

## 证据与根因（基线 55922196，2026-08-02）

- 复现：`SKIFF_ROOT=/Users/geek/workspace/skiff-runtime-stack-overflow
  SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages
  node /Users/geek/workspace/internals/scripts/test-isolated-service.mjs agine.ai/api`
  → `[isolated runtime stderr] thread 'main' has overflowed its stack`，`[skiff-instance] runtime
  exited with SIGABRT; restarting`，随后 `control request returned typed HTTP 503
  AssemblyParticipantsUnavailable: Runtime disconnected before responding`。
- crash report：`~/Library/Logs/DiagnosticReports/skiff-runtime-2026-08-02-120812.ips`，主线程栈：
  `runtime::main → Runtime::block_on → run_forever → run_connected_session_with_bootstrap →
  RouterSessionChildTasks::next (FuturesUnordered) → begin_actor_owner_invoke →
  execute_actor_owner_invoke_inner → ActorMethodExecutor::execute →
  call_program_executable_with_self_direct → eval_program_call / eval_program_expr /
  eval_program_expr_ref 递归`。
- 回归点：6dee837b/04743f6a 合流后，`actor.owner.invoke/control` 从 `tokio::spawn`
  （worker 线程，`RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES` = debug 192 MiB / release 64 MiB）
  改为 session-owned 内联 child task（`RouterSessionChildTasks`，在 main 线程上轮询）。
  默认 main 线程栈约 8 MiB，debug 下 eval 每层约 1 MiB，递归在到达 128 层 program-call
  深度守卫前就已击穿 native 栈。

## 修复边界

- 仅 `runtime/driver/main.rs`：把 `runtime.block_on(run())` 移到
  `RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES` 栈大小的专用 driver 线程上执行；
  不改 session 内联语义、不改 eval/actor 代码、不改 worker 线程栈配置。
- 同步更新 `doc/architecture/tail-call-execution.md` 中关于 driver 线程栈的说明。

## 验证

1. 基线复现（已记录，见上）。
2. 修复后同一 isolated service 测试命令通过 `agent_bridge.chat_config` 崩溃点，
   且 runtime 不再 SIGABRT；记录剩余独立失败（若有）与本修复无关。
3. `cargo check -p runtime` + 相关 Rust selector（`runtime` driver 相关）与
   `cargo fmt --check -p runtime`。

## 交接

- branch `fix/runtime-stack-overflow`；worktree
  `/Users/geek/workspace/skiff-runtime-stack-overflow`；集成 Agent
  `skiff_integration_todo1`。
