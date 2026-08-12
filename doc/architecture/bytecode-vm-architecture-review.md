# Bytecode VM Architecture Convergence Review

> Status: implementation conformance review, non-normative
> Reviewed: 2026-08-12
> Canonical contract: [`bytecode-vm.md`](./bytecode-vm.md)

本文记录当前 bytecode VM 实现相对 canonical architecture 的结构性偏差，用于实现收口和
MVP 范围决策。它不是第二份架构事实源；当本文与 `bytecode-vm.md` 冲突时，始终以后者为准。

本次审查只阅读 compiler、artifact、linker、verifier、VM、heap、scheduler、request、host
和 router 代码，没有运行测试、构建或格式化工具。问题按下列三类区分：

- **当前错误语义**：现有可执行路径已经可能产生错误结果、错误 ownership 或无法取消的等待。
- **结构阻断**：相关能力今天可能仍然 fail-closed，但公开接口无法表达目标模型；直接接通会跑错。
- **完成项**：今天明确 fail-closed，且现有边界足以在以后补实现。完成项不作为本文主问题。

本文采用一个明确的 MVP 原则：verifier 应当更薄，而不是继续替 compiler、linker 或 runtime
补事实。source-owned type/effect/lifecycle fact、registry-owned native ABI、deployment-owned identity
和 scheduler-owned continuation 必须各有唯一 authority；事实缺失时停止 emission/link/admission，
不得在下游反向推断。

## 1. Executive summary

| ID | Severity | Classification | Problem |
| --- | --- | --- | --- |
| VM-01 | Critical | 当前错误语义 | Value transfer/drop plan 未进入 VM 执行，aggregate 实际成为共享可变对象 |
| VM-02 | Critical | 当前错误语义 | `SetWritablePath` 破坏 root slot，接口无法返回 COW 后的新 root |
| VM-03 | Critical | 当前错误语义 | Exception envelope 与 actual catch identity 在 compiler→VM 间丢失 |
| VM-04 | Critical | 当前错误语义 | HTTP 等待被伪装成 `Ready`，同步阻塞 scheduler/Tokio worker |
| VM-05 | Critical | 当前错误语义 | HTTP stream 使用 adapter singleton，`ResourceRef` 不是资源 authority |
| VM-06 | Critical | 当前错误语义 | HostEffect registry、artifact signature、linker bypass、字符串 dispatch 多重 authority |
| VM-07 | Critical | 当前错误语义 | compiler/linker 在 exact fact 缺失后使用宽松 fallback |
| VM-08 | High | 当前不可运行 | Task exact target 被丢弃，同时 request ingress contract 又拒绝所有 task |
| VM-09 | High | 结构阻断 | Scheduler child API 无法表达跨 owner heap 和 boundary materialization |
| VM-10 | High | 边界缺陷 | Raw `ValueSlot`/handle provenance 可伪造，8-bit heap domain 很快复用 |
| VM-11 | High | authority 分裂 | Immutable deployment image 之外仍重读 artifact，并混用 package/deployment build ID |
| VM-12 | High | GC 前结构阻断 | Pending root walk 遗漏 suspended invocation chain |
| VM-13 | High | 当前生命周期错误 | Request supervisor 不以 router session 为 owner，断线可留下孤儿请求 |
| VM-14 | High | 当前预算/性能偏差 | Fuel、单 opcode 工作量、frame reconciliation 和 collection 表示不满足 bounded hot path |

前七项不是 verifier completeness 问题。继续增加 verifier 规则只能掩盖事实 owner 分裂，不能修复
运行语义。

## 2. Current semantic failures

### VM-01 — Value lifecycle plan is not executable semantics

#### Current state

source 层已有独立的 value transfer/lifecycle facts：

- `compiler/source/src/value_transfer.rs`
- `compiler/source/src/value_transfer/classifier.rs`

production bytecode pipeline 却调用 emitter 自己的 plan derivation：

- `compiler/driver/pipeline/bytecode_lane.rs::emit_bytecode_lane`
- `compiler/emission/src/bytecode/plans.rs::derive_bytecode_value_transfer_plans`

emitter 对无法精确分类的 native、generic、package nominal 和 generated temporary 使用启发式或
`SnapshotRelease` fallback。linked frame 虽然携带 concrete plan，VM handlers 仍主要按位复制或清除
`ValueSlot`：

- `runtime/vm/src/fiber.rs::execute_copy_slot`
- `runtime/vm/src/fiber.rs::execute_load_slot`
- `runtime/vm/src/fiber.rs::execute_dup`
- `runtime/vm/src/fiber.rs::execute_drop`
- `runtime/vm/src/fiber.rs::execute_return`
- `runtime/vm/src/fiber.rs::begin_unwind`

生产 VM 没有统一调用 `VmHeap::snapshot_share`、`transfer_owner`、`release_snapshot` 和递归
resource drop。`reconcile_frame_slots_at` 只特判一部分直接 `ResourceRef`，不是 linked lifecycle
plan 的执行器。

Request heap 的 mutator 同时不读取 `snapshot_owners` 来决定 COW：

- `runtime/request/src/vm_heap.rs::snapshot_share`
- `runtime/request/src/vm_heap.rs::write_array_element`
- `runtime/request/src/vm_heap.rs::write_record_field`
- `runtime/request/src/vm_heap.rs::set_writable_path`

#### Failure modes

- `b = a`、参数传递、container element load 或 return 后修改其中一个 aggregate，会改变其他别名。
- overwrite、return、tail-call、throw 和 unwind 丢失 snapshot/resource drop。
- aggregate 内嵌 `ResourceRef` 时，释放外层 aggregate 不会递归 cancel provider。
- 即使只补 `snapshot_share` 调用，mutator 仍然原地写，因此 value semantics 仍不成立。

#### Required convergence

1. source/lowering 为每个 slot、result、capture 和 generated temporary 交付 exact plan。
2. 删除 emitter 的类型猜测和 fallback；缺 plan 直接拒绝 bytecode emission。
3. VM 增加唯一的 linked-plan lifecycle executor，所有 move/share/drop/overwrite/frame-exit 都经过它。
4. heap mutator 消费 owned root，并按 owner count 执行真实 COW。
5. 删除与 lifecycle executor 竞争的事后 frame reconciliation。

### VM-02 — `SetWritablePath` cannot represent path COW

#### Current state

canonical opcode 同时消费 selectors 和 RHS，但实际 path resolution 直到 RHS 已求值后才发生：

- `artifact-model/src/bytecode/opcodes/table.rs` 的 `SetWritablePath`
- `compiler/emission/src/bytecode/functions.rs` 的 writable path emission

`VmHeap::set_writable_path` 只返回 `Result<()>`，无法返回 COW 后的新 root。Request heap 沿旧 handle
原地修改；VM 随后清除 root slot，并将 RHS leaf 写回 root slot：

- `runtime/model/src/vm_heap.rs::VmHeap::set_writable_path`
- `runtime/request/src/vm_heap.rs::set_writable_path`
- `runtime/vm/src/fiber.rs::execute_set_writable_path`

#### Failure modes

- `r.field = 2` 后，local `r` 可能变成整数 `2`，不再是 record。
- shared snapshot 没有 path COW，写入会改变所有 aliases。
- intermediate selector 失败时，RHS 的 host effect 或其他 observable side effect 已发生。

#### Required convergence

将 mutation 改为两阶段协议：

```text
prepared = prepareWritablePath(ownedRoot, selectors)
rhs = evaluateRhs()
replacementRoot = commitWritablePath(prepared, rhs)
slot = replacementRoot
```

`prepared` 必须 pin intermediate path 所需事实，commit 必须原子地返回 resulting root。该 opcode
在完整协议落地前应 fail-closed。

### VM-03 — Exception envelope and ordinary throw are lost

#### Current state

语言语义要求 exception envelope 保存 actual concrete leaf identity，rethrow 保留原 envelope。当前链路
却在多个位置改用静态类型：

- compiler `Throw` 写入静态 `payload_type`；
- VM local throw 使用指令的静态 `TypeIndex`；
- rethrow 使用 exception slot 自身的静态类型；
- async `resume_throw` 改用 `compact_type_tag`。

相关实现位于：

- `compiler/emission/src/bytecode/functions.rs`
- `runtime/vm/src/fiber.rs::{execute_throw, execute_rethrow, resume_throw}`

`UnwindState` 当前也只携带 payload，不携带完整 `RequestException`、cleanup cursor、outcome kind 或
phase。

跨 child/stream boundary 时，普通用户 throw 又会被压缩成 `VmError::UnhandledThrow`，scheduler
随后将所有 `Err` 转为 `ResumeOutcome::Failure`，parent 将其视为 terminal failure：

- `runtime/vm/src/error.rs`
- `runtime/scheduler/src/bytecode.rs`
- `runtime/scheduler/src/stream_driver.rs`

#### Failure modes

- `throw e: A | B` 且 actual value 为 `A` 时，`catch<A>` 可能无法匹配。
- rethrow 可能把包装 record 的静态类型当成新的异常 identity。
- 同一异常是否跨过 Pending 会改变 catch 行为。
- `catch<E> { serviceCall() }` 或 stream producer 的 ordinary throw 绕过 catch、abort 和 cleanup，直接
  终止请求。

#### Required convergence

root/scheduler outcome 必须区分：

```text
Return(values)
Throw(RequestException)
VmFailure(invariant/error)
PlatformTerminal(reason)
```

所有 local、child、adapter 和 stream resume 路径传递同一种 opaque exception envelope。MVP 若暂不
实现 envelope，至少应拒绝 union throw、rethrow 和跨 boundary ordinary throw，不能生成错误语义。

### VM-04 — HTTP wait bypasses Pending

#### Current state

HTTP executor 的公开接口是同步 `Result`。unary path 启动 OS thread 后立即 `join()`；stream open
同步等待 response head；`StreamNext` 调用阻塞式 `Receiver::recv()`：

- `runtime/request/src/http_executor.rs`
- `runtime/host/src/host/bytecode_http_executor.rs`
- `runtime/request/src/bytecode_ingress.rs::poll_stream_next`

request adapter 等待结束后仍返回 `BytecodeAdapterHandoff::Ready`。同步 VM driver 又位于 Tokio task
中，因此该等待会直接阻塞 runtime worker。

#### Failure modes

- 慢连接、慢首包或慢 chunk 期间，scheduler 没有 Pending owner，无法统一处理 deadline、cancel、lease
  和 terminal arbitration。
- raw fuel 只在 VM dispatch boundary 检查，阻塞期间不再 poll。
- 每个请求可以创建并阻塞额外 OS thread。
- stream 使用无界宿主 channel，背压和内存不属于 request budget。

#### Required convergence

host effect start 只能返回：

```text
Ready(result) | Pending(operation)
```

真正等待必须由 Pending registry 持有，completion 只通过 terminal cell 唤醒 scheduler。首版若不实现
该协议，应拒绝相应 HTTP effect；不得同步等待后伪报 `Ready`。

### VM-05 — Stream `ResourceRef` is not the resource authority

#### Current state

`RequestAdapterExecutor` 只有一个 `Mutex<Option<HttpClientStreamState>>`。每次 stream open 虽然会分配
新的 `ResourceRef`，却覆盖该 singleton。`StreamNext` 忽略传入 arguments/endpoint，直接读取 singleton：

- `runtime/request/src/bytecode_ingress.rs::RequestAdapterExecutor`
- `runtime/request/src/bytecode_ingress.rs::poll_stream_next`

#### Failure mode

同一 request 依次打开流 A、B 后，再对 handle A 调用 `next`，读取的是 B；ResourceTable 中的 cancel
entry、实际 receiver 和 provider thread 相互脱离。

#### Required convergence

每个 ResourceTable entry 必须拥有完整 provider state、cancel handle 和 single-consumer lane。所有
`next/cancel/drop` 必须先验证 `ResourceRef`，再通过 entry 访问状态。删除 adapter singleton 和其他
parallel registry。

### VM-06 — HostEffect has four competing authorities

#### Current state

canonical `HostEffectRegistry` 从 native signature 生成 ABI/effect 和 fingerprint。随后链路又分别拥有：

1. emitter 将 registry 的 `NativeCall` effect 改写成 `HostEffect`；
2. artifact 自报完整 signature；
3. linker 的 `match_reference` 对 `skiff.run/std` 或 `std.*` binding 吞掉所有 mismatch；
4. request adapter 用字符串 switch 实现另一套 binding surface。

相关实现：

- `artifact-model/src/host_effect_registry/registry.rs`
- `compiler/emission/src/bytecode/functions.rs`
- `runtime/linker/src/bytecode/link/dispatch.rs`
- `runtime/request/src/bytecode_ingress.rs`

#### Failure modes

- 有正确 top-level registry fingerprint 的 artifact 仍可为既有 std binding 自报错误 arity、type、plan
  或 effect。
- linked image 接受 artifact signature，而不是 pinned registry entry。
- registry 接受但 request 字符串 switch 未实现的 binding 只能在执行期失败。
- verifier 被迫重复维护 config/db/host-effect 特判，仍无法恢复唯一 ABI authority。

#### Required convergence

- artifact 只携带 canonical binding ID 和必要实例化 operands。
- linker 从 pinned registry 构造 typed linked entry，不复制 artifact 自报 signature。
- 删除 emitter effect rewrite、linker std bypass、linker/verifier binding switch 和 request 字符串 dispatch。
- host 只实现一个 `TypedHostEffectId -> executor` bridge。
- verifier 只证明 linked typed ID、调用栈事实和 pending contract。

### VM-07 — Exact-fact failure falls back to a second type system

#### Current state

compiler/linker 存在多条“exact fact 缺失后继续猜”的路径：

- source expression type 使用 `Option` 同时表达 non-value/divergence/fact missing；lowering 缺 fact 时回退
  File IR、`void`、`unknown` 或 `Json`；
- throw payload 重新从 syntax/callable text 推断；
- package nominal record shape 查询失败时，从 initializer 字段制造 nominal shape；
- linker exact normalization 失败后回退 `equivalent_type_ref`；
- fallback 对称接受 integer/number、nullable/inner，并忽略部分 package ABI expectation；
- lifecycle merge 将 expected `SnapshotRelease` 与 actual `Trivial` 视为兼容。

主要实现：

- `compiler/source/src/expression_type_model.rs`
- `compiler/lowering/src/function_lowering.rs`
- `compiler/lowering/src/type_inference.rs`
- `compiler/emission/src/bytecode/functions.rs`
- `runtime/linker/src/bytecode/stack_map/transfer.rs`

#### Failure modes

- 有值 call 可被编码成零 result，改变 operand stack 和 ABI。
- nullable value 可进入 non-null slot。
- ABI mismatch 正是 normalization failure，却被 fallback 再次接受。
- 需要 release 的值可沿 trivial-drop path 合并。
- artifact 内部可以形成自洽但不属于 package descriptor 的 nominal physical shape。

#### Required convergence

- source→lowering handoff 使用封闭结果，例如 `Exact(TypeRef) | NonValue | Diverges`；事实缺失是编译错误。
- nominal shape 只能来自 package-owned exact descriptor。
- linker merge 只接受 exact normalized type 和 exact lifecycle plan。
- 删除 `equivalent_type_ref`、宽松 `plans_match` 和 bytecode path 的 syntax re-inference。
- 语言级 coercion 必须由 source/lowering 发出显式 opcode。

## 3. Current unusable or structurally blocked lanes

### VM-08 — Task target is discarded and task ingress is self-contradictory

#### Current state

task wire 携带 `target_kind=function` 和 exact `target`，host 仅验证非空，随后改用没有 target ID 的
`BytecodeRouteSelector::Operation`。route 对它选择 deployment 的第一个 operation binding：

- `runtime/transport/src/protocol/bytecode.rs`
- `runtime/host/src/host/request_entry/assembly_wire.rs::task_request_from_wire`
- `runtime/host/src/loader/bytecode_admission.rs::BytecodeRouteSelector`

生成的 task envelope 没有 gateway ingress selector；request validation 却要求所有 bytecode request 都有
`ingress_selector`。task recoverable payload 也没有进入 `gateway_entry_arguments`。

#### Consequences

- 当前所有 task 在 request ingress 处 fail-closed，无法执行。
- 若只删除 ingress check，task 会静默调用第一个 operation，而不是 wire 指定 function。
- 即使选中目标，payload 仍不会被 materialize 为 callable arguments。

#### Required convergence

task 使用独立的 exact typed package-callable entry：

```text
TaskWireTarget
  -> verified package-direct binding
  -> recoverable argument decode plan
  -> exact callable entry
```

删除无 ID 的 `Operation` selector，不复用 gateway ingress contract。

### VM-09 — Child scheduler cannot express a fresh owner heap

#### Current state

`BytecodeChildStart` 只有 `unit + resume`，没有 owner、heap、resource context 或 boundary materializer。
`BytecodeScheduler::run` 对 parent、child 和 adapter 始终传入同一个 `&mut dyn VmHeap`，trampoline stack
也只保存 unit：

- `runtime/scheduler/src/bytecode.rs`
- `runtime/scheduler/src/trampoline.rs`

child completion 将 provider `VmOwnedValues` 原样交给 parent；parent resume 又要求 image identity 与 caller
一致。生产 request executor 当前返回 `UnsupportedChild`，公开 `_with_ports` API 还忽略 caller 提供的
ports。

#### Consequences

直接接通当前接口后，service B 会运行在 caller A 的 heap 中；即使 executor 私下创建 B heap，scheduler
也没有保存/切换它的位置，返回值也没有 materialize 回 A heap 的通道。

#### Required convergence

scheduler invocation stack entry 至少拥有：

```text
InvocationUnit {
  image,
  owner,
  heap,
  resourceContext,
  inboundPlan,
  outboundPlan,
  resumeSite,
}
```

删除 ignored ports 和假的 child executor。接口改变前，service/Actor/remote interface/callback 继续
fail-closed；不要通过放宽 verifier 来启用。

## 4. Ownership, identity, and publication risks

### VM-10 — Handle provenance is forgeable and quickly reused

#### Current state

Request heap domain 只有 8 bit，由全局 counter 截断；256 个 heap 后复用。每个 heap 的 serial 又从 1
开始。validation 只检查 domain、当前 serial 和 type/flags：

- `runtime/request/src/vm_heap.rs`
- `runtime/model/src/request_heap.rs`

`CompactTypeTag`、`VmHandle`、reference-valued `ValueSlot` constructors 和 `VmOwnedValues::from_values`
又对外公开。entry/resume boundary 因而可接收 raw slot，而不是与 verified resume site 绑定的 checked
bundle。

#### Failure modes

- 旧 Pending/adapter slot 在 domain wrap 后可能命中新 request 中相同 serial 的对象。
- raw handle 可带伪造 compact tag，在 exception/nominal dispatch 前不一定被 heap 验证。
- ResourceRef 每 request 从小整数重新开始，同样缺 owner/generation identity。

#### Required convergence

- 使用不可快速复用的 owner nonce + generation，或全局单调 64-bit handle identity。
- ResourceRef 采用同等级 owner/generation fencing。
- raw reference constructors 限制在 heap/boundary internals。
- boundary materializer 针对 `VerifiedEntry`/`VerifiedResumeSite` mint opaque checked values。

### VM-11 — Deployment image is not the sole execution authority

#### Current state

loader/cache 正确按 deployment artifact identity 构建 image，但 cache 前后和 request adapter construction
仍多次重读 artifact store。`BytecodeRoute` 保存 `artifact_root`，并把 implementation package build ID
存为 request `buildId`：

- `runtime/deployment-image/src/owner.rs`
- `runtime/host/src/loader/bytecode_admission.rs`
- `runtime/host/src/host/request_entry/assembly.rs`

wire admission 校验的是 deployment artifact identity，因此同一 request 内存在两个不同含义的
`buildId`。

#### Failure modes

- image 已 pin 后，artifact 被回收、移动或临时不可读，request 仍可失败。
- 同一 package build 的多个 deployment 在 RequestEnvelope/telemetry 中被错误合并。
- request 语义依赖 ambient filesystem，而不是 immutable image。

#### Required convergence

- `DeploymentExecutionImage` 自含 verified operation/gateway/task entries 和 adapter plans。
- cache publication 后执行路径不得重新打开 artifact store。
- 删除 route 的 `artifact_root` 和重复 `build_id`；所有身份从 `image.owner()` 派生。
- verified program 应输出 distilled runtime facts，而不是让 scheduler/request 穿透 raw candidate。

### VM-12 — Pending root graph is incomplete

#### Current state

`PendingOwner` 同时拥有 escrow roots 和 suspended trampoline，但 `VmRootSource` 实现只访问 escrow。
`SuspendedTrampoline` 自身已经能枚举 active/blocked fiber roots；sleep 和 stream driver 却普遍使用
`EmptyRoots`：

- `runtime/scheduler/src/pending.rs`
- `runtime/scheduler/src/trampoline.rs`
- `runtime/scheduler/src/stream_driver.rs`
- `runtime/request/src/bytecode_ingress.rs`

#### Consequence

当前 RequestHeap 没有 collector，因此尚未形成现行 UAF；一旦接入 safepoint GC/compaction，parked frame
里的 live handle、wake value 或 buffered item 可能被错误回收或移动。

#### Required convergence

`PendingOwner<S>` 应要求 `S: VmRootSource`，root walk 必须组合：

```text
suspended invocation chain
+ transferred escrow
+ completion/wake values
+ bounded stream buffers
+ resource/provider roots
```

root graph 闭合前不得启用 request GC。

### VM-13 — Request lifetime is not owned by router session

#### Current state

request supervisor 只以 `request_id` 为 key；普通 begin 会覆盖已有 entry。router session disconnect guard
清理 connection/outbound state，但不取消 supervisor 中的 request：

- `runtime/host/src/host/request_supervisor.rs`
- `runtime/host/src/host/router_session.rs`

`router_session_id` 在部分 request entry API 中被忽略。

#### Failure mode

session A 断线后旧 request R 仍可继续外部副作用；session B 重连并启动同 ID 的 R 后覆盖 map entry。
之后 cancel 只作用于新请求，旧请求成为孤儿。

#### Required convergence

- active request key 使用 typed `(RouterSessionEpoch, RequestId)`；旧 session 的 cancel 不得命中新 session。
- reservation activation 必须返回精确的
  `Activated | RevokedByCancel | RevokedBySessionStop | Invalid`。cancel 或 session stop 在 Reserved 状态留下
  revoke tombstone；activation 先赢时只由已创建的 request budget 决定 terminal winner。
- 两种 revoked outcome 都只映射一次 `StopWithoutResponse`，不得创建 budget/inventory，不得重结算、发 terminal
  或重复 cleanup；`Invalid` 是 token/key/row identity 不匹配并映射既有稳定 admission error，cancel 与
  disconnect 竞争时保留第一个 revoke 原因。
- session disconnect 必须终止其全部 request-owned Pending、resources 和 fibers。
- duplicate begin 始终拒绝，不能静默覆盖。

## 5. Budget and hot-path architecture

### VM-14 — Fuel and physical representation do not bound execution work

#### Current state

raw dispatch counter 本身不能被 artifact 重置，但生产 budget 在执行前一次预扣完整 fuel quantum（当前
1024），semantic events 又复用同一个 `instruction_count`：

- `runtime/request/src/execution_budget.rs`
- `runtime/request/src/bytecode_ingress.rs::BytecodeVmBudget`

单条 opcode 内还存在输入尺度工作，却没有 budget/cancel poll：string/bytes clone、string compare、concat、
map ordinal lookup 等。

VM 每条指令先调用 `reconcile_frame_slots_at`，扫描完整 stack map、分配 `Vec`，并 clone 当前 linked
instruction。record/map carrier 和 sidecar 使用 `BTreeMap`；`MapEntryAt` 调用 `.iter().nth(ordinal)`，
compiler 的 map for-in 每轮递增 ordinal，因此完整遍历为 O(n²)：

- `runtime/vm/src/fiber.rs`
- `runtime/request/src/vm_heap.rs`
- `compiler/emission/src/bytecode/functions.rs`

配置的 request memory hard limit 主要覆盖 legacy `RequestHeap` node estimate，不覆盖 sidecars、resource
table、VM frames、Pending、HTTP queue 和重复 carrier graph。

#### Consequences

- 很短的 request 也可能先计入约 1024 条“instruction”，小于 quantum 的 limit 可在首条指令前失败。
- hard fuel 只能限制 dispatch 数，不能限制大输入单 opcode 的 wall work 或 cancel latency。
- fixed-slot dispatch 在宽 frame 热循环中退化为 O(frame slots)+allocation。
- map iteration 为 O(n²)，dense record access 也不是 direct offset。
- 实际 process memory 可明显超过配置的 request heap limit。

#### Required convergence

- request-owned budget 是唯一 accounting/winner authority。VM 私有 dispatch wrapper 在每次 dispatch 紧邻前调用
  `before_dispatch`，一次原子授权并计入一个 raw unit，成功后恰好执行一次；instruction semantic error 仍已执行、
  已计费且不得 retry。不得保留 quantum grant/precharge/refund/remainder、可转移 token/receipt 或 VM raw counter。
- raw 与 semantic attribution 分离。limit=N 时前 N 次 dispatch 成功，第 N+1 次在 dispatch 前失败；limit=`u64::MAX`
  时 `MAX-1 -> MAX` 成功，下一次因 fuel limit 失败，因此 raw overflow 不可达。semantic/poll counter overflow 仍须
  fail closed。
- poll cadence 从同一 budget raw counter 推导；deadline/cancel/internal stop 与 fuel 共用一个 frozen winner，settle
  后 supervisor 只能消费 winner，不得重判。
- 为输入尺度 opcode 分块并设置 poll points，或在 admission/budget 中限制单对象尺寸。
- 删除 per-instruction full-frame reconciliation 和 instruction deep clone。
- dense record 使用 direct offset；map iteration 使用稳定 cursor/iterator，不按 ordinal 重扫。
- 建立统一 owner-local memory ledger，覆盖 heap、sidecar、frames、Pending、resources 和 host buffers。

## 6. Verifier simplification boundary

verifier 的 MVP 职责应收敛为：

1. 消费已经 bounded decode、structurally admitted 的 artifact。
2. 验证 CFG、stack effect、slot liveness、exception edges 和 resume-site shape。
3. 验证 relocation 已解析成 typed target ID，且 call-site 与 linked signature exact join。
4. 验证 `NoPending`、loan 和 ownership plan 的结构使用条件；不重新推导这些事实。
5. 输出 opaque/distilled `VerifiedProgram`，执行层不能继续读取 raw candidate tables。

verifier 不应继续承担：

- 从 artifact 字符串重建 native ABI；
- 从 opcode 反推 interface/callback target kind；
- 从 syntax 或 initializer 重建 type/shape；
- 修补 linker normalization failure；
- 判断 host 当前是否实现某个字符串 binding；
- 构造 request-owned heap objects；
- 为尚未实现的 generic/InOut/callback/Actor lane制造“看似完整”的证明。

对于首版不支持的能力，最简单且正确的策略是在一个明确的 capability gate 上 fail-closed，而不是让
多个 verifier phase 分别返回不同的 `ProofUnavailable`。

## 7. MVP convergence order

### Phase 0 — Deliberately narrow executable surface

首版只开放：

- exact non-generic local calls；
- unary gateway entry；
- 明确支持的 scalar/aggregate value shapes；
- 已闭合 envelope 的 local throw/catch；
- 真正 Ready 或真正 Pending 的少量 typed host effects。

继续 fail-closed：

- generic specialization；
- `InOut`；
- service/Actor/remote interface/callback child；
- stream；
- task package-direct entry；
- recoverable/durable boundary；
- request GC。

### Phase 1 — Correct local execution semantics

1. 建立 single lifecycle executor。
2. 实现 aggregate COW 和 two-phase writable path commit。
3. 统一 `RequestException` envelope 与 root outcome。
4. 删除 compiler/linker fallback authorities。

### Phase 2 — Correct host waiting and resource ownership

1. HostEffect typed registry bridge。
2. Ready/Pending handoff 与 terminal cell。
3. ResourceTable-owned stream/provider state。
4. Session-owned request cancellation。

### Phase 3 — Cross-owner execution

1. Scheduler unit 拥有 owner/heap/boundary plan。
2. 参数和结果在 owner boundary 显式 materialize。
3. ordinary throw 跨 boundary 保留 envelope。
4. 再逐项开启 service、Actor、interface 和 callback。

### Phase 4 — GC and performance

1. 闭合 Pending/request-wide root graph。
2. 建立统一 memory ledger。
3. 实现 safepoint GC/compaction。
4. 删除 hot-loop allocation、frame scan 和 ordinal map iteration。

## 8. Items intentionally treated as completion gaps

以下项目当前大多明确 fail-closed，不单独视为已有错误语义：

- ConstantHeap aggregate materialization；
- deployment linker generic monomorphization；
- `InOut` loan execution；
- callback capture/escape proof；
- bytecode Actor arena/lease integration；
- recoverable schema/value codec；
- request-local tracing GC；
- detailed source attribution/profiling sink。

这些能力可以延期，但启用前必须满足本文对应的 owner、heap、boundary、exception 和 root graph 条件。

## 9. Exit criteria for architecture convergence

实现可以被认为从“功能接近完成”进入“架构闭合”状态，至少需要满足：

- 任意 slot transition 都只能通过 exact lifecycle plan 执行，没有 raw aggregate bit-copy/drop 旁路。
- writable path 在 RHS 前完成 prepare，并原子返回 replacement root。
- local、Pending、child 和 stream 使用同一 exception envelope/outcome model。
- 所有真实等待都表现为 Pending；没有在 VM/Tokio worker 上的 `join` 或 blocking `recv`。
- `ResourceRef` 是 provider state 的唯一索引和 owner。
- HostEffect ABI 只来自 pinned registry，linker/verifier/request 不再维护副本。
- type/shape/plan normalization failure 直接失败，不存在宽松 fallback。
- task target、deployment build ID 和 request/session identity 都是 exact typed facts。
- child scheduler frame 自带 owner heap 和双向 boundary plan。
- Pending root graph 在启用 GC 前完整可枚举。
- fuel、memory 和 hot-path复杂度能够对应文档声明的 bounded execution model。
