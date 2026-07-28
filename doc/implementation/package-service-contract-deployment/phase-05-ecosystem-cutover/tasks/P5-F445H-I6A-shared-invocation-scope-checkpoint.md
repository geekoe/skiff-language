# P5-F445H-I6A shared invocation-scope checkpoint

状态：Ready。I6 的关键路径共享检查点；完成后解除 HTTP、WebSocket request与
time/file/Actor/Host response-source三个互不重叠的 consumer节点。

## 直接父节点

- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`

父 result 已引用 E4R、D1、D2与唯一权威设计。本任务只实现父 result §3.3 / §8.1冻结的
Rust内部 carrier；不得重新读取顶层设计后扩大语义。

## 固定输入与 DAG 位置

```text
base commit  07392f1a1b01f3cafb27c7882b76e6646758444c
base tree    675041dcea6c1f868ccdcd79f8e05b14a54be964

I6-A
  ├─ unblocks I6-B HTTP
  ├─ unblocks I6-C WebSocket request
  └─ unblocks I6-D time/file/Actor/Host response-source
```

当前是实现检查点，不是稳定候选。Service dependency/callee timeout的用户决策不阻塞本任务；
本任务不得为它新增字段、metadata或默认解释。

## 当前事实与第一处修改

`runtime/host/src/eval_capability_adapter/execution.rs` 的 borrowed/owned adapter仍未显式转发
`execution_scope()` / `derive_scope(...)`，因此 full scope走 `Unavailable`。开始后第一处实际
production修改必须在该文件完成，不得先跑测试或重新做开放式审计。

`RuntimeNativeCapabilityProjectionSource` 当前每次调用会重新构造，但 HTTP、WebSocket、time、file、
Actor getter仍返回request构造时冻结的context。共享终态是每次native invocation精确读取一次
`context.execution().owned()`，由该次projection的suspending capability consumer共享；I6-B/C/D只
消费这个内部carrier，不再各自发明投影。

## 实现要求

### 1. Capability façade

- borrowed与owned adapter都无损转发已有full `ExecutionScope`；
- `derive_scope(...)`保留deadline source、site、nesting、全部ancestor/local signals；
- `ExecutionScopeDeriveError`继续通过已有转换保留为
  `ExecutionScopeAccessError::Derive`，不得降级成 `Unavailable`；
- `deadline()`或单一root token不得用来反推scope；
- `owned.borrow()`与borrowed adapter观察同一scope。

### 2. Invocation-time carrier

- native projection构造时从当前 `ProgramExecutionContext`精确读取一次owned execution control；
- 同一次projection内的HTTP、WebSocket、time、file、Actor与response-stream consumer可以取得同一个
  internal carrier；
- carrier只携带现有execution control/scope语义，不新增公开native参数、Skiff类型、artifact字段或
  metadata；
- 本节点不实现任何lower wait、timer、transport deadline、registry settlement或resource cleanup；
  这些分别属于I6-B/C/D；
- 普通WebSocket send等Ready operation不获得人为lease、timer、yield或suspension point；
- E4 actual-Pending、timeout/catch、concurrent、stream与canonical service owner保持不变。

### 3. Lifecycle边界

若内部API需要clone/borrow，必须保持同一scope lifecycle，不得创建额外root或从child反向停止parent。
本任务只证明carrier可用；`acquire_lease()`、pending winner、late result隔离由consumer任务实现。

## 允许写集

Production：

```text
runtime/host/src/eval_capability_adapter/execution.rs
runtime/eval/src/native_capability.rs
runtime/eval/src/capabilities.rs
runtime/capability-context/src/http.rs
runtime/capability-context/src/file.rs
runtime/capability-context/src/actor.rs
runtime/capability-context/src/stream.rs
runtime/host/src/eval_capability_adapter/http.rs
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/actor.rs
```

实际diff必须是该集合的最小子集。后九个文件只允许内部carrier签名的机械跟随；不得提前实现I6-B/C/D
行为。

Tests / fixtures：

```text
runtime/host/src/eval_capability_adapter/execution.rs
runtime/eval/src/program_execution/execution_scope_tests.rs
runtime/eval/src/program_execution/execution_scope_tests/evaluator_checkpoint.rs
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/tests/f445h_e4r_combined/capability_harness.rs
runtime/eval/tests/f445h_e4r_combined/imports.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/actor_dispatch.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
```

除新增I6A聚焦断言外，E4 fixture只允许constructor/trait机械跟随；不得删除、忽略、改名或放宽E4断言。

## 禁止写集

- `runtime/eval/src/eval_context/actual_pending*`、timeout、concurrent、program/response/source stream；
- canonical service wait、legacy outbound/service relay；
- HTTP transport、WebSocket request registry、file runtime、Actor outbound等I6-B/C/D lower；
- public std/native签名、artifact/schema/compiler/router；
- service timeout设计、deployment policy；
- Cargo manifests、`Cargo.lock`。

## 任务内并行

父任务Agent保留统一实现和自验收责任，可以使用最多两个有界子Agent，且子Agent不得继续委派：

1. 一个只读子Agent在五分钟内枚举carrier constructor/fixture机械跟随点，只返回精确路径与签名，
   不修改文件；
2. 父Agent冻结内部carrier签名后，可让一个test-only子Agent在独立worktree中实现不与父Agent
   production写集重叠的聚焦fixture/断言并提交；父Agent负责接收、集成和统一验证。

不得让多个子Agent重叠扫描或修改同一文件。若carrier需要父result未授权的公共API或production owner，
立即返回 `TASK_SCOPE_EXPANDED`，不得继续派发。

## Test-first与验证

先保留真实RED：

- borrowed、owned或`owned.borrow()`对derived scope返回 `Unavailable`；
- derived invocation中的fake capability观察root而不是child；
- derive overflow/error variant被错误折叠。

GREEN至少证明：

- inner-earlier、outer-earlier、equal保留outer owner；
-deadline source/site/nesting与全部signals无损；
- borrowed、owned、`owned.borrow()`一致；
- 每次native invocation只读一次current control；
- 机械fixture继续编译，Ready operation没有虚假suspension。

聚焦命令：

```bash
cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --list
cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --list
cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --nocapture
cargo check -p skiff-runtime-host -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

两个listing必须非零，执行数量必须与listing一致。不得运行完整crate gate、E4完整411、
network/stable/live/MongoDB。

反向搜索：

```bash
rg -n "context\\.(time_context|file_source_stream_context|http_client_context|websocket_context|actor_context)\\(\\)" runtime/eval/src/native_capability.rs
rg -n "fn (execution_scope|derive_scope)" runtime/host/src/eval_capability_adapter/execution.rs
rg -n "\\$/cancelRequest|-32800|CancelError|yield" runtime/{eval,host,capability-context}/src
```

第一条不得再表示suspending consumer直接采用冻结snapshot；第二条同时覆盖borrowed/owned；第三条不能因
本任务增加公开cancel、peer cancel或yield。

## 交付

提交顺序：

1. production/tests实现提交；
2. 新增
   `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
   的result提交。

Result必须记录精确commit/tree、实际写集、RED/GREEN、非零计数、自验收矩阵、反向搜索与
I6-B/C/D是否全部解除。若范围扩大或某一consumer接口仍不确定，如实返回精确blocker，不得把本任务
伪装成完整checkpoint。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i6a-scope
branch   codex/p5-f445h-i6a-scope
```

工作树最终clean；不得merge、rebase或push。启动五分钟内完成第一处production修改；不得在修改前跑测试。
