# Actor 共享 heap 批次：实现接口契约与任务组织

> 权威设计：[actor-shared-heap-design.md](../../architecture/actor-shared-heap-design.md)。
> 本文是执行层文档：接口契约、批次 DAG、文件所有权、任务文档约定。
> 与设计冲突时以设计为准；本文件不得改变设计语义。

## 1. 任务文档约定

- 本目录为批次任务文档根目录；开发 Agent 的叶子任务文件放在
  `doc/implementation/actor-shared-heap/tasks/<任务名>.md`；
- 叶子任务文件必须引用直接父节点（本文件或设计对应章节），整条引用链可追溯；
- 任务文件只补充执行信息（代码范围、依赖、命令、worktree、证据 owner），不得改变设计语义。

## 2. 批次 DAG

- **F1（在途）**：`db transaction` 同包禁令（compiler execution_semantics + 本地 helper 可达性）+
  移除 runtime actor 事务回滚路径（`with_transaction_live_fields`、rollback actor 分支）+
  删除 actor 事务测试、补普通 request 事务回归测试。
- **Wave 1（并行）**：
  - **F2（求值器核心）**：`HeapAccess` 双模式（§3）+ `EvalContext.heap` 改造 + 漏斗
    release/reacquire（§4）+ `Interpreter` 入口签名（§5）+ provider-stream 边界修复 +
    关联测试；
  - **F5（router，独立）**：`inMemoryRegistryStore` idle 逐出与 upgrade 互卡竞态修复 +
    回归测试。
- **Wave 2（F1 + F2 合流后）**：
  - **F3（actor 层 + model）**：共享 arena store / frame / executor 重写（§6）+ arena epoch
    （§7）+ active/suspended 计数 + per-instance limits + quiescence 压缩 + 失败段部分写入
    语义测试。基线包含 F1（事务路径已移除）与 F2（HeapAccess API 已存在）。
- **批末**：集成 Agent 将 `integration/actor-shared-heap` 合入 `main` 一次。

## 3. `HeapAccess`（新文件 `runtime/eval/src/heap_access.rs`，F2 拥有）

```rust
pub(crate) enum HeapAccess<'a> {
    Exclusive(&'a mut RequestHeap),
    Shared {
        arena: Arc<tokio::sync::Mutex<RequestHeap>>,
        guard: Option<tokio::sync::OwnedMutexGuard<RequestHeap>>,
    },
}

impl HeapAccess<'_> {
    pub fn heap_mut(&mut self) -> &mut RequestHeap;   // Shared: guard 必须 Some，否则 invariant 错误
    pub fn release(&mut self);                        // Shared: guard.take() 并 drop；Exclusive: no-op
    pub async fn reacquire(&mut self);                // Shared: guard = Some(arena.lock_owned().await)；Exclusive: no-op
    pub fn is_shared(&self) -> bool;
}
impl Deref / DerefMut for HeapAccess（Shared 经 guard；Exclusive 直接）
```

- 普通 request / 未共享路径 = `Exclusive`，语义与现状完全一致，release/reacquire 为 no-op；
- actor 实例 arena = `Shared`；guard 不得跨 `Pending` 存活；release/reacquire 只发生在漏斗内。

## 4. 漏斗契约（F2 实现；F3 只经 `ActorExecutionFrame` 使用，不直接依赖漏斗签名）

- `actual_pending::await_operation`、`program_db::wait::await_operation`、
  `program_stream::current_scope::next_with_actor`、`callback_native::prepared` 的 wait：
  对 `Shared` 执行 poll-once（`Ready` 不释放；`Pending` → `release()` → await → `reacquire().await`），
  对 `Exclusive` 保持现状直接 await；
- `ActorExecutionFrame::await_if_pending` 语义（F3）：poll-once；`Pending` →
  `access.release()` → await future → `access.reacquire().await` → 校验 instance fence / arena epoch；
  F3 通过 `HeapAccess` 的公开方法实现，不依赖 F2 漏斗内部。

## 5. `EvalContext` 与 `Interpreter` 入口（F2）

- `EvalContext.heap` 从 `&'a mut RequestHeap` 改为 `HeapAccess<'a>`；所有内部 heap 访问走
  `self.heap.heap_mut()`；`heap_mut()` 定义在 `HeapAccess` 上，不在 `EvalContext` 上
  （避免同语句借用 `self.env` / `self.context`）；
- 共享模式下，任何能返回 `Pending` 的路径不得持有 guard；
- `call_program_executable*` 系列的 heap 参数由 `&mut RequestHeap` 改为 `&mut HeapAccess`
  （或等效）；普通 request 调用点传 `Exclusive`；actor 调用点传 `Shared`；
- 同步函数（deep clone、codec、materialize、error promote）继续收 `&mut RequestHeap`。

## 6. Actor store 契约（F3）

- `ActorInstanceState { fields: Vec<ActorFieldValue>, arena: SharedArena }`，字段根指向 arena 节点；
- `acquire_segment(handle) -> SegmentLease`（含 guard + fence/epoch 快照）；release/commit 无复制；
- active / suspended 续体计数（create、段、恢复中、放弃、提交）；升级 / 逐出要求计数 == 0；
- per-instance arena limits；`compact_if_quiescent()`（计数 == 0 且无 upgrade/discard 时触发）；
- 失败段不保证字段原子性（设计 §3.4）。

## 7. Arena epoch（runtime/model，F3）

- `RequestHeap` 增加 epoch（默认 0；`new_with_epoch(u32)`；`epoch()`）；
- `HeapHandle` 增加 epoch；`slot()` / `slot_mut()` 校验 handle.epoch == heap.epoch；
- `alloc_*` 以当前 heap epoch 盖章；压缩创建新 arena 时 epoch + 1；
- `runtime_values_equal` 的 handle 相等快路径因 epoch 入 handle 而安全。

## 8. Router（F5，独立）

- `router/src/actor/inMemoryRegistryStore.ts`：进入 `upgrading` 时取消/清理 pending idle eviction；
  upgrade 完成容忍 owner 丢失；补 router 回归测试。无跨模块接口依赖。

## 9. 文件所有权（并行写集，互不重叠）

- F2：`heap_access.rs`（新）、`eval_context.rs`、`eval_context/actual_pending.rs`、
  `eval_context/timeout.rs`、`program_db/wait.rs`、`program_stream/current_scope.rs`、
  `program_stream.rs`（如需）、`callback_native/prepared.rs`、`program_execution.rs`、
  `db_eval.rs`（如需）、`spawn_ops.rs`（如需）、`async_stream_cancel.rs` +
  `prepared_unary.rs`（provider 边界）及关联测试；
  `actor_concurrent_continuation.rs` 仅限 `await_if_pending` 签名/访问适配（最小机械改动）；
- F3：`runtime/model/src/value.rs`、`runtime/model/src/request_heap.rs` 及测试；
  `actor_instance.rs`、`actor_executor.rs`、`actor_concurrent_continuation.rs` 及关联测试
  （基线含 F1，事务路径已移除）；
- F5：`router/src/actor/inMemoryRegistryStore.ts` 及 router 相关测试；
- F1（在途）：compiler execution_semantics、`program_db/rollback.rs`、`program_db/tests/transaction.rs`。

## 10. 集成与验收

- 集成 Agent 串行合入 F1/F2/F5/F3 到 `integration/actor-shared-heap`；
- F2 与 F3 各自合入后必须先通过合并 HEAD 的 `cargo check`（F2+F3 交叉接口）；
- 全部合流后冻结候选，按设计 §10 验收矩阵验收；最后合入 `main` 一次。
