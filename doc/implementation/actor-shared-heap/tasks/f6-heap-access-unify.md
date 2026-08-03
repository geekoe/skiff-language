# F6 — 统一 HeapAccess 为单一 arena+guard 机制

> 直接父节点：[interfaces.md](../interfaces.md)（§3–§5 由本任务更新为统一形态）。
> 引用链：权威设计 [actor-shared-heap-design.md](../../architecture/actor-shared-heap-design.md)（v4，
> 用户已确认“求值器对所有执行只使用一个 heap 访问机制，不再有 Exclusive/Shared 双模式”）。
> 冲突时以权威设计为准；本文件只补充执行信息与实现决策，不改变设计语义。

## 1. 任务边界

- 基线：`main 4e9cfeb4aa8fd4044b1d90844999afe359ae95a8`。
- worktree：`/Users/geek/workspace/skiff-heap-unify`，分支 `feat/actor-heap-access-unify`。
- 集成目标：`/root/integration_heap_unify`（只报告，不合并、不 push）。
- 目标：删除 `HeapAccess::Exclusive` 与 `is_shared()`，`HeapAccess` 变成单一 struct
  `{ arena: Arc<tokio::sync::Mutex<RequestHeap>>, guard: Option<OwnedMutexGuard<RequestHeap>> }`；
  所有执行（普通 request / actor / callback owner / provider / producer / spawn）走同一条
  poll-once → Pending 时 release → await → reacquire 的漏斗协议。

## 2. 预检结论（基线 4e9cfeb4，git objects 只读）

- `HeapAccess::Exclusive` 生产构造点：`program_invocation.rs`（owned invocation heap）、
  `assembly_execution/ingress.rs`（caller `&mut RequestHeap` 适配器）、`runtime_http_gateway.rs`
  （guard/pre/handler/server-stream）、`runtime_websocket_connect.rs`、`runtime_websocket_jsonrpc.rs`、
  `spawn_ops.rs`（canonical spawn）、`program_stream.rs`（producer task owned heap）、
  `async_stream_cancel.rs`（provider task owned heap）、`prepared_unary.rs`（provider owned heap）、
  `callback_native/prepared.rs`（callback owner owned guard）。
- `HeapAccess::Shared` 构造点：`actor_executor.rs` 两处（actor 段 guard）、各 actor 测试 fixture。
- 漏斗分支（`is_shared()`）：`eval_context/actual_pending.rs`、`program_db/wait.rs`、
  `program_stream/current_scope.rs`。
- 测试文件：`heap_access/tests.rs`、actor/driver/host 等 20+ 文件直接构造 `Exclusive`/`Shared`。
- 闭合性确认：所有 `&mut RequestHeap` 到求值器的入口都在本任务转换范围内；`runtime/request` 仅
  `assembly_ingress.rs` 一处经 `dispatch_ingress_via_in_process_boundary` 传入 `&mut RequestHeap`。
  无 router/compiler/artifact-model 依赖。

## 3. 实现决策（最小一致选项，均记录于此）

1. **单一 struct**：`HeapAccess` 无生命周期参数；字段 `arena` / `guard` 保持私有。
   构造器：
   - `HeapAccess::private(RequestHeap)`：普通执行专用私有 arena（新建 `Arc<Mutex<..>>`，立即
     `try_lock_owned` 获取 guard；新建 arena 无竞争，允许 `expect`）。
   - `HeapAccess::with_guard(Arc<Mutex<RequestHeap>>, OwnedMutexGuard<RequestHeap>)`：actor 段 /
     callback owner 等已持有 guard 的共享 arena 构造点。
   - `into_owned_heap()`：先 drop guard，再 `Arc::try_unwrap` 取回 `RequestHeap`，用于保持
     `execute_*_runtime_value` 等公共返回类型不变（私有 arena 的 Arc 在执行期间无其它强引用：
     producer/provider 任务各自拥有独立私有 arena，不 clone 调用方 arena）。
2. **执行状态持有 arena**：`PreparedProgramInvocation.heap` 从 `RequestHeap` 改为 `HeapAccess`
   （私有 arena）；`prepare_eval_invocation_with_heap` 等入口保持 `RequestHeap` 进出参数，
   内部 `HeapAccess::private` 包裹、返回时 `into_owned_heap` 还原——http_adapter 的 guard/pre/
   handler heap 穿梭结构因此不变，且每个执行段都是“执行状态持有私有 arena”。
3. **漏斗单一路径**：`await_shared_with_release` 更名 `await_with_release`（pub(crate)），
   三个漏斗删除 `is_shared()` 分支，非 actor-frame 路径一律走该函数。
4. **`EvalContext` / `DbIrEvaluator`**：去掉 `'h` 生命周期参数（`EvalContext<'a>`、
   `DbIrEvaluator<'a>`）；全部 `EvalContext<'_, '_>` 改 `EvalContext<'_>`。
5. **callback owner**：`InProcessCallbackAdapter` 新增 `owner_heap_arena()`（返回
   `Arc<Mutex<RequestHeap>>` 克隆），`prepare_interface_call` 用
   `HeapAccess::with_guard(arena, try_lock_owner_heap_owned()?)`；`CallbackOwnerWait` 存储
   `HeapAccess`，`wait`/`finalize` 经 `heap_mut()` 访问。原因：`try_lock_owner_heap_owned` 只返回
   guard，统一 struct 需要 arena 才能 reacquire；这是直接引起的机械 API 扩展。
6. **行为保持**：私有 arena 锁无竞争；不重新引入任何双模式快速路径；同步函数
   （deep clone/codec/materialize/error promote）继续收 `&mut RequestHeap`（经 `heap_mut()` 取）。

## 4. 写入范围

- 生产：`runtime/eval/src/heap_access.rs`、`eval_context.rs`、`eval_context/actual_pending.rs`、
  `db_eval.rs`、`program_db.rs`、`program_db/wait.rs`、`program_db/transaction.rs`、
  `program_execution.rs`、`program_invocation.rs`、`program_stream.rs`、
  `program_stream/current_scope.rs`、`spawn_ops.rs`、`runtime_http_gateway.rs`、
  `runtime_websocket_connect.rs`、`runtime_websocket_jsonrpc.rs`、
  `assembly_execution/ingress.rs`、`assembly_execution/async_stream_cancel.rs`、
  `assembly_execution/async_stream_cancel/prepared_unary.rs`、
  `assembly_execution/callback_native/prepared.rs`、`actor_executor.rs`、
  `actor_executor/actor_concurrent_continuation.rs`、`runtime/native/src/callback_adapter.rs`
  （仅新增 `owner_heap_arena()`）、`runtime/request/src/assembly_ingress.rs`。
- 测试：上述 eval 模块关联测试、`runtime/driver/eval` 两处、`runtime/host` 四处 assembly_admission
  执行测试、`runtime/eval/tests/f445h_e4r_combined/r4_stream_case.rs`。
- 文档：`doc/implementation/actor-shared-heap/interfaces.md` §3–§5、本任务文件。
- 禁止：router/、compiler/、artifact-model/linked-program schema、.github/workflows、router-rust
  相关 worktree；不改变 wire/artifact ABI；不改 actor 语义与普通 request 语义。

## 5. 验证（owner：F6）

- `cargo check --workspace --all-targets` 绿。
- `cargo test -p skiff-runtime-eval`（lib + integration）。
- 视触碰情况：`cargo test -p skiff-runtime-request -p skiff-runtime-host -p skiff-runtime-loader`
  （至少编译其测试）。
- `cargo fmt --check`（触碰 crate）。隔离 target：worktree 内 `build/cargo-target`
  （`.cargo/config.toml` 相对路径，天然按 worktree 隔离）。

## 6. 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 单一 HeapAccess struct，无 Exclusive | `runtime/eval/src/heap_access.rs` | `rg 'Exclusive|is_shared'` 无命中（生产+测试） | `cargo test -p skiff-runtime-eval` |
| 所有执行走同一漏斗协议 | 三个漏斗文件 | 无 `is_shared()` 分支 | heap_access/tests.rs 私有+共享 pending 测试 |
| 普通 request 私有 arena，执行状态持有 | `program_invocation.rs` 等 | 无 `&mut RequestHeap` 跨越 Pending 的生产路径 | 既有普通 request 全套测试 |
| actor 共享 arena 同代码路径 | `actor_executor.rs` | `HeapAccess::Shared` 无残留 | actor executor/instance/db/stream 测试 |
| 文档 §3–§5 统一形态 | interfaces.md | — | 文档自检 |
