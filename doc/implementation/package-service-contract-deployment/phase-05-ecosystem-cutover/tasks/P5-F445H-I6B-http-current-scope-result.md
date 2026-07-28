# P5-F445H-I6B HTTP current-scope consumer result

状态：

```text
TASK_SCOPE_EXPANDED
I6_B_HTTP_COMPLETE = NO
I6_J_HTTP_CASE_UNBLOCKED = NO
READY = NO
```

本任务在有界 production 调用链探查后停止。没有实现 HTTP current-scope consumer，没有修改
production/test，也没有使用 task-local、global registry 或其它 side channel 绕过 I6-A carrier owner。

## 1. 候选身份与实际写集

| 项 | 值 |
| --- | --- |
| 合同固定 base commit | `8db08c539acaf0b3fc41733365f06e9883bdbdd8` |
| 合同固定 base tree | `71123064dd0948d5946ad8c6312df909670794e0` |
| 任务开始 HEAD | `baf2547d37e2f9103a360c9615fb29a9bb6584c9` |
| 任务开始 tree | `f936f711eba2bd2ca73ce7b59e8d404004b6923f` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6b-http` |
| branch | `codex/p5-f445h-i6b-http` |

任务开始 HEAD 是固定 base 之上的任务发行文档提交。实际写集只有本 result；production/test 写集为空，
因此没有 implementation commit。

## 2. 被证伪的合同前提

合同预期允许写集内的 HTTP lower consumer 可以取得 I6-A invocation-time execution carrier。实际调用链为：

```text
runtime/eval/src/native_capability.rs
  RuntimeNativeCapabilityProjectionSource::http_client
    -> RuntimeNativeHttpClientCapabilityContext::new(http context, invocation carrier)

runtime/eval/src/capabilities.rs
  impl NativeHttpClientCapability for RuntimeNativeHttpClientCapabilityContext
    -> clone self.context
    -> into_effect_context()
    -> HttpClientCapabilityContext::{dispatch_http_request,dispatch_http_stream,dispatch_http_sse}

runtime/host/src/host/http_client_runtime.rs
  HttpEffectRequest::new
    -> frozen HttpEffectContext.deadline_ms
    -> frozen HttpEffectContext root cancellation token
```

精确代码证据：

1. `runtime/eval/src/native_capability.rs:130-135` 把同一次 invocation carrier 传入
   `RuntimeNativeHttpClientCapabilityContext::new`。
2. `runtime/eval/src/capabilities.rs:711-736` 是 HTTP native context 和 carrier 的唯一共同 owner；
   carrier 保存在 private `invocation_execution` 字段，并由 crate-private accessor 暴露。
3. `runtime/eval/src/capabilities.rs:1099-1150` 的三个 native dispatch 只 clone
   `self.context` 并调用 `into_effect_context()`，没有把 `self.invocation_execution` 传给 Host lower
   consumer。
4. `runtime/host/src/host/http_client_runtime.rs:75-181` 因而只能构造带旧
   `deadline_ms` / root token 的 `HttpEffectRequest`。该文件及其它合同允许文件没有
   `RuntimeNativeInvocationExecutionControl` 或 `OwnedExecutionControl` 的取得路径。

因此 carrier 在进入合同允许写集之前已经丢失。仅修改允许文件无法满足“operation 开始时读取 full current
scope”，也无法用 current scope 的全部 signals 与 absolute deadline 驱动 pending waiter。

## 3. 为什么必须停止

安全接线至少需要修改 `runtime/eval/src/capabilities.rs`，而该文件不在 I6-B 允许写集。若不修改该 owner，
只能新增 task-local/global mutable state、从 frozen deadline/token 反推 scope，或另建 carrier；这些方案会违反
I6-A 的唯一共享 carrier、调用时读取和无影子实现约束。

未发现需要修改 public std/native schema、E4 actual-Pending、router、HTTP ingress、Cargo manifest 或
lockfile。范围扩张是一个精确的漏列 consumer bridge owner，不是公共语义或架构设计缺口。

## 4. 最小后继合同

重新发行 I6-B 时，最小增量是：

1. 将 `runtime/eval/src/capabilities.rs` 加入 production 允许写集，但只授权
   `impl NativeHttpClientCapability for RuntimeNativeHttpClientCapabilityContext` 的 unary、body-stream
   open 与 SSE open 三个 dispatch 接线。
2. 三个 dispatch 从已冻结的 `self.invocation_execution()` 取得 owned control/full
   `ExecutionScope`，并通过 crate-private Host 方法传入 `http_client_runtime.rs`；不得修改 public native
   trait 或业务输入。
3. Host lower consumer继续拥有 scope lease、`min(current, primitive)`、winner settlement、lower future
   drop、late finalize 隔离和 existing response/stream best-effort cleanup。
4. 原合同其余 production/test 写集、RED/GREEN、反向搜索和聚焦验证命令保持不变。

不需要把 `runtime/eval/src/capabilities.rs` 整体重构，也不需要扩大到 I6-C/D、E4、router 或 public API。

## 5. 未运行项与证据状态

由于 production 实现无法在授权范围内开始，以下合同验证均未运行：

```text
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
cargo check -p skiff-runtime-host --locked
cargo fmt --check
```

RED/GREEN、非零 listing/execution 计数、owner 矩阵和合同反向搜索均未建立完成证据。没有运行完整 crate/stage
gate，没有连接或启动 network、stable/live、MongoDB，也没有访问本机 stable instance。

result-only staged diff 已运行 `git diff --cached --check` 并通过。

I6-B 仍阻塞 I6-J HTTP case；本 result 只解除“合同为何不可执行”的诊断，不解除任何实现或验收节点。
