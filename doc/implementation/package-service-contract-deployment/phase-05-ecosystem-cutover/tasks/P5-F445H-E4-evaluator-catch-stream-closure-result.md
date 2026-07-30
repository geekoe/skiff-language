# P5-F445H-E4 Evaluator, catch and stream closure result

状态：`TASK_SCOPE_EXPANDED`。

本节点在开始 production/test 修改前发现 E3 actual-Pending seam 无法安全承载 E4 必须迁移的
heap-borrowing 外部调用。该问题命中任务文件停止规则中的“E2/E3 seam 不足、需要公共 API 或写集外
production owner”，因此本节点没有实现部分 timeout/concurrent/stream 语义，也没有保留旧
pre-suspend 路径作为临时 fallback。

## 1. 固定输入与检查范围

| 项 | 值 |
| --- | --- |
| production prerequisite | `7a69b7e3` |
| task document | `6d324555` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4-evaluator-closure` |
| branch | `codex/p5-f445h-e4-evaluator-closure` |

已完整读取任务合同及四份直接父结果：

- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-E2-lane-local-DAG-scheduler-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`
- `P5-F445H-E23-concurrency-branches-combined-result.md`

只检查了 E4 写集内现有调用点及其直接调用接口，以判断 E3 seam 是否可接线；没有从更高层文档扩大
需求。

## 2. 精确阻塞

E3 提供的唯一通用 actual-Pending helper 是：

```rust
pub(crate) async fn await_if_pending<F>(
    &self,
    heap: &mut RequestHeap,
    execution: &ExecutionControl<'_>,
    future: F,
) -> Result<F::Output, RuntimeError>
where
    F: Future;
```

它先 poll `future` 一次；若得到 `Pending`，再调用 `self.suspend(heap)`，等待 future 完成，最后
调用 `self.resume(heap, execution)`。这个接口仅适用于 future 不借用同一个 `heap` 的场景。
`program_stream.rs` 中 stream `next()` 即属于这种可接线场景：next future 自身不持有 caller
`RequestHeap`。

但 E4 必须移除 pre-suspend 的以下 production 调用均在 future 内持有同一个 live heap，部分还持有
包含该 heap 的整个 `&mut EvalContext`：

| E4 调用点 | 被调用接口中的冲突 borrow |
| --- | --- |
| `LinkedExprIr::{DbOperation, DbQuery, DbTransaction, DbLeaseClaim, DbLeaseRead}` | `Interpreter::eval_program_db_* (..., heap: &mut RequestHeap, env: &mut Env, ...)` |
| remote interface dispatch | `call_outbound_service_operation(..., heap: &mut RequestHeap, env: &Env, ...)` |
| callback interface dispatch | `dispatch_callback_capability(context: &mut EvalContext<'_>, ...)` |
| activation-relative service call | `dispatch_service_call(context: &mut EvalContext<'_>, ...)` |
| Actor method dispatch | `dispatch_actor_method(context: &mut EvalContext<'_>, ...)` |
| legacy service dependency dispatch | `call_outbound_service(..., heap: &mut RequestHeap, env: &Env, ...)` |
| native call | `dispatch_resolved_native_call(..., heap: &mut RequestHeap)` |

例如 native 路径若直接消费 E3 seam，形状必然成为：

```rust
let future = native_dispatch.dispatch_resolved_native_call(..., self.heap);
frame.await_if_pending(self.heap, &self.execution, future).await
```

构造 `future` 时第一个 `&mut RequestHeap` borrow 已进入 future，并持续到 future 完成或被 drop；
`await_if_pending` 随后要求第二个同时存活的 `&mut RequestHeap`。service/callback/Actor dispatch
借用整个 `&mut EvalContext`，因而冲突范围还包含 `self.heap`。这不是可通过缩短局部变量生命周期
解决的表面 borrow-checker 问题：E3 必须在第一次真实 `Pending` 之后读取同步段的最终 heap 来提交
Actor fields，而 operation future 此时仍拥有修改该 heap 的唯一权限。

## 3. 已排除的绕行方案

- **第一次 poll 前 clone heap**：不正确。future 第一次 poll 可以同步修改 heap，或修改通过 Actor
  field / argument alias 可达的节点；旧 clone 不是该同步 segment 在真实 `Pending` 点的终态。
- **第一次 poll 后 clone heap**：无法安全实现。future 仍持有 live `&mut RequestHeap`；此时再次读取
  heap 正是 Rust alias 规则禁止的并发访问。
- **raw pointer / `unsafe` 绕过 borrow**：会在 future 保有唯一可变引用时制造别名访问，破坏 heap
  mutation、Actor commit 和 cancellation/drop 的内存安全与失败原子性，不能成为 runtime seam。
- **drop 后重建/restart future**：第一次 poll 可能已经发送请求、写 DB、登记 waiter 或执行其它外部
  副作用；重建会重复副作用，也无法恢复原 future 的协议状态。
- **在 poll 前 pre-suspend**：这正是 E4 要删除的旧行为。`Ready` future 会错误提交并释放 Actor
  segment，`connection.send` 等 ready 调用也会被静态调用种类强制让出执行权。
- **把 heap move 进 future**：仍不能在 future 返回 `Pending` 且继续存活时取回其中 heap 进行
  Actor commit；只是把同一所有权冲突藏进 future。

因此不能在 E4 写集内通过 helper、clone 或局部重排正确修复。

## 4. 两个可行的前置修正方向

### 4.1 推荐：Actor frame 持有可提交的 detached field snapshot

让 `ActorExecutionFrame` 在同步 segment 内维护与 caller heap 解耦的 canonical field snapshot：

1. frame 创建、resume 和 `ActorSelfField` write 时同步刷新 snapshot；
2. 第一次真实 `Pending` 后，frame 可从 snapshot 提交当前 segment，不再读取仍被 future 借用的
   caller heap；
3. 把现有单体 `await_if_pending(heap, execution, future)` 拆成两阶段 seam：
   - poll once；`Ready` 直接返回；
   - `Pending` 时提交 detached snapshot并返回原 pending future；
   - 调用方消费 future、释放其 `&mut EvalContext` / `&mut RequestHeap` borrow 后，再用 live heap
     resume、执行 checkpoint 和 identity fence。
4. RAII guard 继续负责 pending future drop、cancel和未 resume 路径，不能把 lease/gate cleanup
   下放给各调用点。

该方向保持 native/service/DB/interface API 不变，E4 后继只需在原写集内机械消费统一两阶段 seam，
不会复制 Actor store、field codec、identity fence或 scheduler状态机。

建议先建立一个窄 E3 correction 节点，最小 production 写集为：

- `runtime/eval/src/actor_executor.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation.rs`
- `runtime/eval/src/actor_executor/actor_concurrent_continuation/**`
- 对应 `runtime/eval/src/actor_executor/tests/**`

只有在现有 `ActorInstanceExecutionLease` 无法读取/重建其 lease-owned heap 时，才把
`runtime/eval/src/actor_instance.rs` 作为一个明确的最小 accessor 扩展加入该 correction；不得让
E4 自行修改它。

correction 必须用真实 heap-backed Actor field alias 测试证明：首次 poll 的同步 mutation进入
snapshot、`Ready` 不提交、`Pending`只提交一次、pending drop/cancel清理、resume后 identity fence
和字段导入不回归。

### 4.2 备选：把每类外部调用拆成 prepare / wait / finalize

把 operation owner 改为三阶段协议：

1. `prepare(&mut heap, &mut env)` 完成所有同步编码与状态建立；
2. `wait()` 返回不借用 caller heap/env 的 owned future；
3. `finalize(outcome, &mut heap, &mut env)` 在恢复 Actor segment后解码结果并提交 caller-local
   mutation。

E3 现有 `await_if_pending` 即可包装 heap-free `wait()`。但该方向必须分别重构 service boundary、
outbound service、DB、Actor dispatch、callback与 native dispatch owner，写集至少跨：

- `runtime/eval/src/assembly_execution/**`
- `runtime/eval/src/service_dispatch.rs`
- `runtime/eval/src/program_db.rs`
- `runtime/eval/src/actor_dispatch.rs`
- `runtime/native/src/dispatch/**`
- 各 owner 对应测试

这比推荐方向更大，也更容易让不同 operation 产生不一致的 cancellation/cleanup协议；应拆成多个
独立 prerequisite 节点，而不能并入 E4。

## 5. 用户决策

当前不需要产品或语言语义决策。用户已经确定：

- 只有真实 `Pending` 才释放 Actor segment；
- `Ready` 不让出执行权；
- 不引入 `yield` / `nosuspend`；
- 不保留 pre-suspend兼容路径。

阻塞属于 runtime ownership seam 缺失。建议由主 Agent按推荐方向建立 E3 correction task并做独立
验收；若 correction 探查证明 detached snapshot 需要改变 Actor field alias 的语言语义，届时才应
单独向用户报告具体语义选择。

## 6. 本节点交付状态

- 未修改 production、tests、四份父结果或任务文档；
- 未运行 Cargo suite，因为 test-first RED 会在不可消费的上游 seam 上形成虚假部分实现，不能作为
  E4 接受证据；
- 未运行 stable、live、network、其它仓库测试；
- 未 merge、rebase或 push；
- 本 result 是本分支唯一新增文件。

F445H-E4 仍为未完成。E3 correction 合入新的固定 prerequisite 后，应从该 checkpoint重新建立或
继续 E4，完整执行原任务中的 T05–T12、focused suite、完整 eval suite与反向搜索。
