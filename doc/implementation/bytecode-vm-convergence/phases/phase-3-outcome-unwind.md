# Phase 3：outcome and unwind

> Status: active; activation authorized by the accepted Phase 2 result
>
> Semantic Closure: opaque exception envelope、actual catch identity、return/throw/VM failure/platform terminal、region cleanup
>
> Depends on: [`phase-2.md`](../results/phase-2.md) accepted and merged into `main`
>
> Unblocks: Phase 4 scheduler, Pending and request ownership

本文收敛 review finding VM-03（[`bytecode-vm-architecture-review.md`](../../../architecture/bytecode-vm-architecture-review.md)）。
若与 Phase 1/2 accepted receipts 冲突，以 receipt 为准并先修订本文。

## 1. 目标

1. `RequestException`（`runtime/model/src/service_error.rs`）是唯一 opaque exception envelope：cause 保存
   **actual concrete leaf identity**（运行时值自身的 `catch_identity()`），不是 compiler 静态 `payload_type`、
   指令 `TypeIndex` 或 slot 静态类型；
2. `throw` 从被抛值的运行时事实构造 envelope；`rethrow` 复用原 envelope 不变（identity 保持）；`resume_throw`
   消费同一 opaque envelope，不使用 `compact_type_tag`；
3. root/scheduler outcome 区分 `Return(values)` / `Throw(RequestException)` / `VmFailure` / `PlatformTerminal`；
   `VmError::UnhandledThrow` 删除，根级未捕获 throw 以 typed outcome 传出；
4. unwind 期间每一层退出的 frame 的 live slot 都经 Phase 2 lifecycle executor drop（cleanup owner 释放）；
5. `catch` 按 envelope 的 actual `CatchIdentity` 匹配（union `A | B` 的实际叶 A 匹配 `catch<A>`），不按静态类型；
6. 请求边界投影：Throw → canonical 用户 error response、terminal 恰一次；VmFailure → sanitized InternalError；
   PlatformTerminal → StopWithoutResponse（Phase 1 语义不变）。

## 2. 非目标

Phase 3 不：

- 接入 host effect、真实 Pending/child/stream（cross-Pending rethrow 的 live VCP 属 Phase 4；本 Phase 只保证
  envelope 设计可被 Phase 4 原样消费，并用受控 resume harness 证明 identity）；
- 实现 recoverable error 的恢复语义、service error 转换或 platform error envelope；
- 改变 Phase 1 的 11 事件观察序、budget、terminal、cleanup 或 Phase 2 的 lifecycle/COW 语义。

## 3. 精确协议

### 3.1 VM 侧

- `UnwindState` 改为携带完整 `RequestException` + unwind cursor/phase；`execute_throw` 用被抛
  `ValueSlot` 的运行时 identity 构造 `RequestException::local(...)`（构造失败 = VmFailure，不是回退静态类型）；
- `execute_rethrow` 复用 `UnwindState` 中的同一 envelope（不重包）；`resume_throw` 消费 `ResumeOutcome::Throw`
  携带的 envelope；
- `begin_unwind` 的 frame-exit 全走 Phase 2 lifecycle executor；`catch` region 的 handler 进入前把 envelope 存到
  catch slot；
- 根级未捕获 throw 返回 typed outcome（如 `DispatchOutcome::UnhandledThrow(Arc<RequestException>)`），
  `VmError::UnhandledThrow` 删除。

### 3.2 scheduler / request 边界

- `ResumeOutcome::Throw(RequestException)` 保持 opaque 传递；scheduler 不再把普通 throw 压成 terminal failure；
- 请求驱动把 `Throw` 投影为 canonical 用户 error（terminal `Failed` 一次、无二次 settle）；`VmFailure` 投影为
  sanitized `InternalError`；`PlatformTerminal` 与既有 cancel/session-stop 同路径。

### 3.3 compiler / admission

- emission 为 throw/catch/rethrow 产出 envelope 语义所需的事实（actual identity 在运行时取自值，compiler 不
  写死 `payload_type`）；union/nominal 异常的 catch 匹配事实保留；
- admission 放行同步 throw/catch/rethrow（payload 为 Phase 2 的 record/array/scalar 面）；仍拒绝 host effect/
  Pending/child/stream 内的 throw（fail closed，直到 Phase 4/5）。

## 4. 精确支持面

accepted：同步普通 `throw`/`catch`/`rethrow`，异常 payload 为 Phase 2 支持面（scalar/record/array），union 叶
按 actual identity 匹配，region cleanup 释放 owner，未捕获 throw 投影为 canonical user error。

仍 disabled 且 fail closed：host effect、Pending/child/stream 相关 throw、service error 恢复语义、platform error
envelope、异步 rethrow（live 面）。

## 5. VCP-3

真实 `.skiff` fixture 经 production compiler→linker→VM→request path：`throw union(A|B) 的 A 叶` →
`catch<A>` 匹配成功（若抛 B 叶则 `catch<A>` 不匹配，走到 `catch<B>` 或未捕获 error）；`rethrow` 后 envelope
identity 不变；cleanup owner（record/array，含 Phase 2 drop）在 unwind 中释放（heap spy 证明 release 序列）；
外部结果证明 catch 语义与 terminal 恰一次。受控 resume harness：同一 envelope 经 `ResumeOutcome::Throw` →
`resume_throw` 后 identity 不变。

## 6. 双线与 Gate

首日并行三 lane（写集见 MAP3）：central kernel K3（VM envelope/outcome + scheduler/request 投影 + linker
admission）、compiler lane C3（throw/catch 发射与 admission）、Proof+Gate lane P3G（VCP-3 harness + negative +
Phase 3 Gate，含 Phase 1/2 全量回归）。Gate 从首日包含全部 required scenario（含 expected-red），producer 由红
转绿时同一 join 收进矩阵。

## 7. Acceptance checklist

- [ ] envelope 单一权威；throw 用 actual runtime identity；rethrow/resume_throw 保持 identity；
- [ ] root outcome 四分类；`VmError::UnhandledThrow` 已删；terminal 恰一次；
- [ ] catch 按 actual `CatchIdentity` 匹配，union 叶 A/B 行为正确；
- [ ] unwind 经 lifecycle executor 释放 cleanup owner（heap spy 证据）；
- [ ] 请求边界三类投影正确（user error / sanitized InternalError / StopWithoutResponse）；
- [ ] admission 只放行同步 throw/catch/rethrow，host/Pending throw 仍 fail closed；
- [ ] VCP-3 与受控 resume harness 绿；
- [ ] Phase 1（11 事件/budget/terminal/cleanup）与 Phase 2（lifecycle/COW）回归全绿；
- [ ] canonical Phase 3 Gate 聚合全部 required evidence class 并拒绝 dirty/stale/missing/zero/skip/tampered；
- [ ] frozen candidate 由全新 Acceptance Agent 给出 PASS；
- [ ] Phase 3 result 合入 `main` 后才标为 `accepted`，Phase 4 才解禁。

## 8. Stop and escalation

- envelope 无法在现有 heap/request 结构上 opaque 化（需要第二权威或静态类型回退）时停止并上报；
- catch 匹配语义与 canonical `doc/reference` 冲突时停止并回到文档层，不在下游兼容推断；
- 任何共享设计选择经独立 review 后仍无法消解时停止并上报用户。
