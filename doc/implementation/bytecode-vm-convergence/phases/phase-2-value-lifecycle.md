# Phase 2：value lifecycle and writable path

> Status: active; activation authorized by the accepted Phase 1 result
>
> Semantic Closure: exact aggregate value lifecycle, alias isolation and two-phase writable path
>
> Depends on: [`phase-1.md`](../results/phase-1.md) accepted and merged into `main`
>
> Unblocks: Phase 3 outcome and unwind

本文把 review findings VM-01/VM-02（[`bytecode-vm-architecture-review.md`](../../../architecture/bytecode-vm-architecture-review.md)）
的 required convergence 收敛为一个可逐阶段证明的 Semantic Closure。若本文与 Phase 1 accepted receipt 冲突，以 receipt
为准并先修订本文；leaf Agent 不能自行选择。

## 1. 目标

Phase 2 完成时必须同时具备：

1. 每个 slot/result/capture/generated-temporary 的 value transfer 只有**一个 exact plan 权威**：source/lowering 产出，
   emitter 原样透传，缺失 plan 在 bytecode emission 前稳定拒绝；
2. emitter 不再有任何类型猜测、`SnapshotRelease` fallback 或启发式（`is_std_duration_type`/`is_type_param_type`/
   `is_never_type`/`is_ordinary_structural_type`/`is_stream_with_package_symbol_item`/`is_authoritative_stream`/
   `is_record_aggregate` 全部删除）；
3. VM 有**唯一** linked-plan lifecycle executor，所有 move/share/drop/overwrite/argument/return/tail/unwind/frame-exit
   都经过它；删除与之竞争的事后 `reconcile_frame_slots_at`；
4. heap mutator 消费 owned root，并按 snapshot owner count 执行真实 COW；shared snapshot 的写入不改变任何 alias；
5. `SetWritablePath` 改为两阶段协议：`prepare -> evaluateRhs -> commit -> slot = replacementRoot`；
6. 声明支持面（记录/数组）有 frozen-candidate VCP 与独立验收；其余 aggregate 能力保持 `disabled` 且 fail closed。

## 2. 非目标

Phase 2 不：

- 开启 map、string、bytes、representation、ResourceRef、stream、host effect、tail call 或 exception；
- 实现 GC、cross-owner heap、snapshot 跨 owner 传输或 request compaction；
- 实现 throw/catch/unwind 语言语义（lifecycle executor 只保证 frame-exit drop 协议，供 Phase 3 复用）；
- 新增 observation 事件种类；Phase 1 的 11 事件序与 queue 上限不变。

## 3. Authority 和精确协议

```text
SourceValueTransferFacts.plan(SourceValueTransferPlanInput)
  -> derive_bytecode_value_transfer_plans(facts, admitted units)  [缺失 plan => typed emission error]
  -> artifact FunctionValueTransferPlans { slot_plans, result_plans }
  -> linker per-slot LinkedValueTransferPlan
  -> VM lifecycle executor（唯一消费者）
  -> VmHeap 同步原语 { snapshot_share, transfer_owner, release_snapshot, release_resource,
                       prepare_writable_path, commit_writable_path }
```

### 3.1 emitter 侧（VM-01 第 1/2 条）

- `compiler/driver/pipeline/bytecode_lane.rs` 必须把 source 的 `SourceValueTransferFacts` 传入 emission；
- `compiler/emission/src/bytecode/plans.rs` 的 `derive_bytecode_value_transfer_plans` 改为消费
  `source_value_transfer_plan(facts, ...)` 的 exact plan；无法取得 exact plan 时返回稳定 typed
  `BytecodeEmissionError`（不发明 `SnapshotRelease`、不按类型外形猜）；
- artifact 已发布的 plan 不得在 linker/VM 侧被“宽松 merge”或替换；exact link 失败即拒绝。

### 3.2 VM lifecycle executor（VM-01 第 3/5 条）

- 新 `runtime/vm/src/lifecycle.rs` 是唯一 lifecycle executor；`fiber.rs` 的
  `execute_copy_slot`/`execute_load_slot`/`execute_dup`/`execute_drop`/overwrite/argument transfer/
  `execute_return`/tail frame-exit/`begin_unwind` 全部改为调用它；
- executor 对每个 slot 转换按 `LinkedValueTransferPlan` 选择物理原语；`VmFirstInstructionDispatched`、
  budget、terminal、cleanup 观察语义不变；
- 删除 `reconcile_frame_slots_at` 及其特判（`AffineResource`/`ExplicitCloneLease` 只走 executor）；
- heap error 不改变逻辑 ownership/share state；重试安全；错误路径零 observation。

### 3.3 owned-root COW（VM-01 第 4 条）

- `VmHeap::snapshot_share` 必须记录第二 owner；`transfer_owner`/`release_snapshot` 精确增减；
- `commit_writable_path` 在 owner count == 1 时可原地写，>1 时必须 COW 并返回新 root；
- `write_array_element`/`write_record_field` 等既有 in-place 路径只允许被 commit 路径在 exclusive owner 时调用；
- 释放外层 aggregate 必须经 executor 递归释放内嵌 aggregate 的 snapshot owner（本 Phase 无真实
  ResourceRef，递归 resource drop 由 heap spy 单测证明协议，真实资源到 Phase 5）。

### 3.4 两阶段 writable path（VM-02）

`runtime/model/src/vm_heap.rs` 删除 `set_writable_path`，改为：

```rust
fn prepare_writable_path(
    &mut self,
    root: &ValueSlot,
    segments: &[VmHeapPathSegment],
    selectors: &[ValueSlot],
) -> Result<WritablePathPreparation, VmHeapError>;

fn commit_writable_path(
    &mut self,
    prepared: WritablePathPreparation,
    value: ValueSlot,
) -> Result<ValueSlot, VmHeapError>; // replacement root
```

- `prepare` 在 RHS 求值**之前**完成：pin intermediate path 事实、校验 liveness/owner/segment 形状；失败时
  heap state 不变，**RHS 的 host effect 或其他 observable side effect 尚未发生**；
- `commit` 原子完成 path COW 与 leaf 写入，返回 replacement root；失败不留下半写入；
- `fiber.rs::execute_set_writable_path` 的顺序固定为 `prepare -> evaluateRhs -> commit -> slot = replacementRoot`；
  RHS 按 RHS 位置的 linked plan 转移进入，被替换 leaf 按 leaf plan drop；
- `WritablePathPreparation` 是 opaque、non-Clone 的 model 类型，VM 只持有不检查。

### 3.4a Amendment 1（MAP2 Revision 3；2026-08-13）

当前 `SetWritablePath` 是单指令形状：selectors 与 RHS 由**前序指令**在 operand stack 上求值，VM handler 再从栈
取 RHS。因此在 Phase 2 的纯值支持面（RHS 只能是 number/boolean/null 与嵌套 record/array 构造，不含任何宿主效应）
内，`prepare` 仍先于 `commit` 且 commit 原子；但“RHS 宿主副作用不提前发生”这一条无法由该 opcode 形状证明，也无需
证明——支持面内 RHS 没有宿主副作用。

本条记为该条的 Phase 5 前置：在把任何可含宿主效应的 RHS（host effect/ResourceRef/stream）接入
`SetWritablePath` 之前，必须先把发射形状改为 `prepare -> evaluateRhs -> commit` 的显式三阶段（否则回到 VM-02 的
原始失败模式）。Phase 2 不回退该义务，也不为纯值表面发明第二个执行器。

同时定案：`ArrayPushOwned`/`MapPutOwned` 在 Phase 2 保持 exclusive-owner-only；对 shared container 调用 fail closed
（`OwnershipViolation`），不做隐式 COW push。共享容器 push 的 COW 语义若进入后续 Phase，需单独决策。

## 4. 精确支持面

| Dimension | Accepted target |
| --- | --- |
| value shapes | `record`、`array`（递归含 `number`/`boolean`/`null` 与嵌套 record/array） |
| flow | construction、dense field/index load、两阶段 writable path mutation、copy/argument/container/return transfer、overwrite/return drop |
| authority | source exact plan -> emitter exact pass-through -> linker exact plan -> VM lifecycle executor |
| result | deterministic scalar/record/array JSON payload via canonical boundary carrier |

默认不支持且 fail closed：`map`、`string`、`bytes`、representation、ResourceRef、stream、host effect、tail call、
throw/catch、generic、`InOut`、task/service/Actor/interface/callback、request GC、cross-owner heap。admission（compiler +
K1 linker capability）只放行上表，其余在唯一边界拒绝。

## 5. VCP-2

```text
real .skiff fixture（嵌套 record/array 聚合）
  -> production compiler（exact plan 通过）
  -> canonical artifact publication
  -> production deployment load/link/image
  -> production request entry（注入 production-composition 的 heap spy）
  -> 同步 VM lifecycle executor
  -> deterministic response
```

最小外部结果：`b = a` 后 `b.inner.x = 2`，`a.inner.x` 保持原值、`b.inner.x == 2`、响应包含两者；同时证明
container/argument/return 各发生一次 exact transfer。内部事实由经 `drive_runtime_bytecode_request` 的
`heap: Option<Box<dyn VmHeap + Send>>` 注入的记录型 heap spy 证明：`snapshot_share`（share）、COW commit
（owner>1 时产生新 root）、`release_snapshot`（drop）的精确调用序列。missing-plan negative：一个无法取得
exact plan 的 source 在 emission 稳定拒绝且不发布 artifact。

## 6. Development / Proof 双线与 Gate

首日并行三条 lane（写集详见 MAP2）：central kernel K2（model trait + VM executor + request heap + linker admission，
单 owner）、compiler lane C2（pipeline facts + emission exact pass-through + 缺失 plan 拒绝）、Proof+Gate lane
P2G（expected-red VCP harness、missing-plan negative、Phase 2 Gate selector/self-tests，含 Phase 1 全量回归）。

Gate 从首日起就包含本 Phase 全部 required scenario（含 expected-red 的 VCP 与 negative）；任何 producer 由红转绿
时必须**同一 join** 把对应 scenario 收进 Gate 矩阵——这是 join 条件，不等到 Acceptance 再补。

## 7. Acceptance checklist

Phase 2 只有全部成立才为 `accepted`：

- [ ] 每个已发布 plan 可追溯到 exact source plan；emitter 无启发式/fallback 残留（反向搜索为空）；
- [ ] 唯一 lifecycle executor 消费所有 slot 转换；`reconcile_frame_slots_at` 已删除；
- [ ] 两阶段 writable path 顺序固定，RHS 副作用在 prepare 成功后、中间 selector 失败时 RHS 不执行；
- [ ] owned-root COW：shared snapshot 的 mutation 不改变 alias，exclusive owner 可原地写；
- [ ] 递归 snapshot/resource drop 协议经 heap spy 证明；
- [ ] VCP-2 走 production composition 并返回预期响应；
- [ ] missing-plan negative 在 emission 稳定拒绝；
- [ ] Phase 1 的 11 事件序、budget、cleanup 回归全绿；record/array 场景不改变 Phase 1 观察语义；
- [ ] 支持面之外的所有 aggregate lane 在唯一边界 fail closed；
- [ ] canonical Phase 2 Gate 聚合全部 required evidence class 并拒绝 dirty/stale/missing/zero/skip/tampered；
- [ ] frozen candidate 由全新 Acceptance Agent 给出 PASS；
- [ ] Phase 2 result 合入 `main` 后才把记录/数组能力标为 `accepted`，Phase 3 才解禁。

## 8. Stop and escalation

- exact plan 无法覆盖声明支持面的某个 shape，只能缩小支持面或回到 Design，不能回退 emitter 猜测；
- 两阶段协议无法在真实 heap 上原子实现（需要第二账本或事后 reconciliation）时停止并上报；
- 任何共享设计选择经独立 review 后仍无法由现有 authority 消解时停止并上报用户。
