# F8：求值器帧减肥（Box 递归 + 上下文装箱 + 去栈上 pin）

> 直接父节点：批次 TODO `todo/actor-shared-heap.md` F8 条目；引用链
> 权威设计 `doc/architecture/actor-shared-heap-design.md`（v4）→
> 实现契约 `doc/implementation/actor-shared-heap/interfaces.md` →
> 本任务文件。
> 基线：`af7aada1db45bc4888ad81d849f57c1927f15c80`（main）。
> 背景事实（父任务给定，不重新争论）：deep-chain 测试 debug 每层 ~1.5 MiB，
> 128 非尾层 ≈ 192 MiB；websocket-connect / stream producers 在旧 192 MiB
> worker 栈上溢出，`5c350fe7` 把 `RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`
> 提到 384 MiB 只是止血，本任务是结构性修复。

## 目标

- 递归求值 future 装箱：递归 async 子 future 不再内联进父状态机，父 future 相对
  递归层数 O(1)（`Box::pin` / `BoxFuture` / `#[async_recursion]`），保持 `Send`；
- 漏斗去栈上 pin：`poll_once`、`await_with_release`、`next_with_actor`（及同模式
  `wait` / `await_if_pending` 等）改为装箱等待，不再 `tokio::pin!` 大 future；
- `ProgramExecutionContext` / `EvalContext` 大字段装箱（保持 clone 语义，安全处
  优先 `Arc`）；
- 语义不变：actor DB-only 事务（`rollback_after_transaction`）、HeapAccess
  单结构协议、release/reacquire 纪律、普通 request 行为全部保持；
- 目标：debug 每层 ≤ 256 KiB（128 层 ≤ ~32 MiB），用 deep-chain 测试
  `SKIFF_NON_TAIL_DEPTH_STACK_KIB` 量化。

## 写集

- `runtime/eval`：`eval_context.rs`（含 `actual_pending.rs`、`timeout.rs`）、
  `program_execution.rs`、`program_stream.rs` + `current_scope.rs`、
  `db_eval.rs`、`program_db.rs` + `wait.rs`、`heap_access.rs`、
  `actor_executor/actor_concurrent_continuation.rs`（`await_if_pending`）、
  `actor_executor.rs`（scope wait 漏斗）、`program_invocation.rs` +
  `current_scope.rs`、`assembly_execution/async_stream_cancel` +
  `current_scope.rs`、`runtime_websocket_jsonrpc.rs`（websocket connect 栈溢出
  路径的栈上 pin）、`env.rs` / `program_execution.rs` 上下文装箱所需最小字段改动；
- `runtime/driver/eval/tests`：deep-chain 测量/回归测试（受限栈断言、size 证据）；
- 本任务文件。

禁止：compiler/、artifact-model/、linked-program schema、router/、
`.github/workflows/`、`runtime/driver/config.rs`（384 MiB 止血保留，不在写集）。

## 执行计划

1. 基线测量：deep-chain 测试 + `SKIFF_NON_TAIL_DEPTH_STACK_KIB` 二分，记录 before
   每层上界；
2. 递归点装箱（先主递归环：`eval_program_call`、
   `Interpreter::exec_program_executable` / `exec_program_block_body` /
   `exec_program_executable_control` / `exec_tail_entry_control`、db/program_db/
   stream/timeout 环），每步重测；
3. 漏斗装箱：`await_with_release`、`poll_once` 调用面、`next_with_actor` 的
   `wait`、`await_if_pending`、scope wait 同模式；
4. 上下文装箱：`ProgramExecutionContext` / `EvalContext` / `Env` 大字段按 size
   证据装箱（`Arc` 优先）；
5. 验证：`cargo check --workspace --all-targets`、`cargo test -p skiff-runtime-eval
   --lib`、`cargo test -p runtime --lib`（tail_call + program_execution）、
   `cargo fmt --check`；deep-chain after 每层 ≤ 256 KiB；
6. 提交并交接 `integration_eval_frame_diet`。

## 验收

- before/after 每层数字来自 `runtime_program_non_tail_recursion_deep_chain_hits_raised_guard`
  （默认 + env 表征）；
- 关键 future / 上下文的 `size_of` 证据；
- 自验收矩阵：任务条款 | 代码证据 | 反向搜索证据 | 测试。
