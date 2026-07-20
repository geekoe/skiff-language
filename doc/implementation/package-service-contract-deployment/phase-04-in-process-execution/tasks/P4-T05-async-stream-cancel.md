# P4-T05：Async / Stream / Cancel Execution

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.10、§6.2、§7、§12、§14。
- 风险/验收组：高风险async/stream/cancel lane；T04–T06合流后由R02分别验收。
- 当前成熟度：R01 shared kernel；完成后是async/stream/cancel lane checkpoint。
- 有效证据：本任务clean commit及exact R01 checkpoint。owned context、stream registry、cancel/lifetime hook、
  spawn call graph或测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R01 PASS；与T04/T06并行。
- 解锁：R02。
- branch：`codex/p4-t05-async-stream-cancel`。
- worktree：`/Users/geek/workspace/skiff-p4-t05-stream`。
- 五分钟内真实edit；async不能和stream/cancel再拆为争同一owner的并行任务。不得修改T03中央wiring、ordinary或callback模块。

## 写入范围

独占T03/T02预留的async/stream/cancel lane模块、`runtime/capability-context` stream/cancellation owner、host concrete
stream runtime的必要最小改动，以及
`runtime/host/src/loader/assembly_admission/tests/execution/async_stream_cancel.rs`。不得修改ordinary/callback、
fixture root、router/compiler/shared kernel API。

## 完成态

1. Rust future/implicit suspension无条件持有显式provider RequestActivationContext；不靠`may_suspend`分支或TLS。
2. stream producer始终provider context，consumer始终receiver context；producer task只从owned context capture启动，
   每个item按canonical item plan detached materialize。
3. stream存在时按contract延长request callback lifetime；end/close/early break/cancel使producer、stream registry和
   对应lifetime exact-once终止。
4. cooperative cancellation传播到pending provider unary、suspend点与stream next；NotCancellable/Unsupported按
   descriptor fail closed，不经router cancel relay。
5. pre-cancel、cancel/start race、duplicate terminal、owner exit不泄漏task/table/lease；稳定错误在receiver context返回。
6.不扩旧atomic-flag compatibility allowlist，不启用被禁测试模块来伪造production owner；必要测试放在真实模块。

## 最早探针与唯一验证 ownership

```bash
cargo test -p skiff-runtime-capability-context cancellation
cargo test -p skiff-runtime-host stream_runtime
cargo test -p skiff-runtime-eval in_process_stream
cargo test -p skiff-runtime-eval activation_context_across_suspend
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
git diff --check
```

探针覆盖provider/receiver owner、item alias隔离、backpressure、early break、cancel race、registry清理。不得运行完整gate。
host lane测试必须复用T03 typed full-chain fixture，不手写resolved target。

## 回报

提交一个commit，回报context传播图、terminal状态机、spawn枚举、命令与自验收矩阵。
