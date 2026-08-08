# Bytecode VM and Runtime Memory Architecture

本文是 Skiff bytecode VM、请求内存模型与挂起/恢复机制的目标内部架构事实源。它面向 compiler、
artifact、linker、runtime、host adapter 与 profiler 维护者，规定 relocatable bytecode、linked
execution image、验证、值布局、collection 物理表示、request-local GC、VM fiber、effect suspension、
异常 unwind 和执行归因之间的长期边界。

Skiff 尚未发布。本架构落地时整体升级 artifact/runtime 格式并删除树遍历 async evaluator；production
不保留旧 artifact reader、双解释器或按版本 fallback。

本文不定义用户可见语法和语言语义。`var` / `let` / `const`、collection value semantics、mutating
receiver、static collection transformation，以及显式 caller-writable 参数的最终拼写必须写入
`../reference/`。本文只规定这些语义在 runtime 内的表示和执行后果。在对应 reference 文档完成迁移前，
这些内容是本架构的显式语言契约依赖，不得被当作已经生效的当前语法。

## 1. Outcome and invariants

完成态必须满足以下不变量：

- 所有 service/package program code 通过一个扁平、同步的 bytecode dispatch loop 执行；普通 local
  call 不创建 Rust future、async closure 或 native recursive evaluator frame。
- VM 只在一个具体 pending-capable effect 本次实际返回 `Pending` 时停放 fiber；函数的 transitive
  `mayPending` 不能触发预先挂起，也不能产生另一套 local call ABI。
- artifact 保存 relocatable ISA；assembly linker 解析 exact target/type/effect binding，runtime 只执行
  已完成 link 且通过 verifier 的 `LinkedBytecodeImage`。
- VM slots、operand stack 和 collection elements 使用紧凑、固定宽度的 value slot。nominal/catch/union
  identity 必须保留，但不在每个值上复制完整 identity object。
- VM frame/value stack 与语言 heap 分离。call return 回收 frame segment；eligible tail call 原地替换
  frame，空间不随 tail hop 增长。
- 每个 request 使用独立、GC-capable 的 managed heap。短且低分配请求可以零 collection 结束；是否运行
  collector 只由 allocation pressure 决定，不由请求持续时间或入口类型决定。
- 同一 request 可以有多个 runnable/waiting fiber 和真实重叠的外部 I/O，但任意时刻只有一个 fiber
  执行 bytecode 或修改 request heap。
- sync/ready 热路径、execution budget、profiling statement units、source attribution、exception identity、
  timeout/cancellation、Actor ownership 和 recoverable boundary 语义不因 VM 迁移而改变，除非对应
  reference contract 被显式修改。
- artifact ISA 不暴露 Rust pointer、allocator address、`Vec` layout、native future layout 或 runtime
  quickening 细节。

本架构不承诺首版 JIT，也不定义 global process heap、跨 request raw heap sharing、持久化 continuation，
或同一 request 内多线程同时执行 bytecode。它必须为后续 baseline/template JIT 保留显式类型、控制流、
stack effect 与 safepoint 信息。

## 2. Canonical layers and owners

完整执行链只有以下 canonical owners：

```text
source + dependency summaries
  -> source type/effect/provenance analysis
  -> lowering
  -> relocatable bytecode emitter
  -> immutable PackageArtifact/File IR bytecode
  -> RuntimeAssembly linker
  -> LinkedBytecodeImage
  -> admission verifier
  -> VM + request scheduler + effect adapters
```

职责边界如下：

1. **Source effect analysis** 是 callable transitive may-effect 的唯一 owner。lowering 不得再次遍历 AST
   推断另一份 `maySuspend`。
2. **Bytecode emitter** 负责控制流、stack effects、constant pool、relocation、frame layout、exception
   region、statement entry 与 source map，但不制造 assembly-local exact address。
3. **Assembly linker** 消费 exact package/deployment/image facts，把 relocation 解析为 image-local target、
   type tag、shape、effect adapter 和 capability table entry。
4. **Admission verifier** 独立验证 linked code；compiler/linker 产出的 max stack、target kind 和 effect
   声明都不能直接视为可信。
5. **VM core** 只拥有 program frame、value stack、control flow、unwind 和 effect invocation protocol；
   它不直接实现 DB、HTTP、service routing、Actor admission 或 native resource lifetime。
6. **Request scheduler/effect adapters** 拥有 pending operation、wake、cancel、deadline、fiber queue 和 host
   I/O。adapter 不能保存可被 GC 移动的裸 heap pointer。

Canonical RuntimeAssembly projection 与所有 test fixture 必须进入同一个 emitter/linker/verifier/VM
shape。测试 fixture 可以用内存 artifact store，但不能维护 test-only opcode、第二套 evaluator 或 assembly
失败后的 legacy fallback。

## 3. Artifact ISA and runtime micro-ops

### 3.1 ISA definition

这里的 ISA（Instruction Set Architecture）是持久化 bytecode 的语义契约，包括：

- opcode 与 operand schema；
- 每条 instruction 的 operand-stack input/output；
- branch、call、tail call、return、throw 和 effect invoke 语义；
- constant、type、shape、target 与 relocation index 的含义；
- verifier 规则；
- ISA/schema version。

ISA 不包括 runtime 内存地址、decoded Rust enum 大小、dispatch optimization、superinstruction 或 JIT machine
code。

### 3.2 Encoding

Artifact bytecode 使用 wordcode：

- code 是 `u32` word sequence；
- `pc` 是函数 code 内的 word offset；
- 每个 opcode 在该 ISA version 中有固定 operand word 数；
- operand 只保存 immediate、relative branch、slot index、pool index 或 relocation index；
- multi-operand instruction 占多个 word，不把任意 `ExecutableAddr` 强塞进一个 `u32`；
- jump target 必须指向 instruction header，不能落入 operand word。

Rust-ish artifact 草图如下；字段名不是 public API：

```text
RelocatableBytecodeFunction
  functionKey
  words: [u32]
  constants: [ArtifactConstant]
  relocations: [BytecodeRelocation]
  frameLayout
  maxOperandDepth
  exceptionRegions
  statementEntries
  sourceMap
```

`frameLayout` 保存 parameter/local slot 数量和 compiler-known slot kind；debug binding name 进入单独 debug
table，不扩大每个 runtime frame。

### 3.3 Relocation and link

Package artifact 不能保存最终 `ExecutableAddr`，因为该地址依赖具体 RuntimeAssembly 的 package slot、file
slot 与 link overlay。Artifact instruction 引用 relocation，例如：

```text
LocalExecutableRef
PackageCallableRef
InterfaceMethodRef
TypeRef
ShapeRef
EffectRef
ConstantRef
```

Assembly linker 解析 relocation，生成 immutable image-local table：

```text
LinkedBytecodeImage
  functions
  exactTargets
  typeTags
  shapes
  constants
  effectAdapters
  capabilityBindings
```

共享 Package code 不因不同 assembly 被原地 patch。Decoded instruction 保存 image-local table index；若未来
实测证明 patch private decoded copy 更快，可以在不改变 artifact ISA 的前提下采用。

### 3.4 Decoded micro-ops

Runtime admission 后可以把 wordcode 解码为固定 Rust micro-op array，并执行 quickening：

```text
Artifact ISA             Runtime-only decoded form
call relocation          CallLocal(exact_function_index)
get field shape/field    GetDenseField(offset)
invoke effect ref        InvokeEffect(adapter_index, resume_descriptor)
```

Superinstruction 只允许存在于 decoded/JIT layer，例如 `LoadSlotCall`、`GetFieldBranch`。它们不得进入
artifact schema，不得改变 semantic instruction charging、statement profiling 或 source attribution。

### 3.5 Initial semantic instruction families

首版 ISA 至少包含以下语义族；具体 numeric opcode 由 artifact schema 单一声明生成，compiler 与 runtime
不得各自复制编号：

```text
Value/slot
  const
  copy_slot
  move_slot
  store_slot
  drop
  dup

Control
  jump
  jump_if_true
  jump_if_false
  switch_tag

Call
  call_local
  tail_call_local
  return

Record/value
  new_record
  get_dense_field
  set_writable_path
  representation_wrap
  interface_box

Collection
  new_array_builder
  array_builder_push
  freeze_array
  array_get
  array_push_owned
  new_map_builder
  map_builder_put
  freeze_map
  map_get
  map_put_owned

Exception/control region
  throw
  rethrow
  enter_region
  leave_region

Effect
  invoke_effect
```

`copy_slot` 与 `move_slot` 是物理 ownership/share facts：二者产生相同用户值，区别只在 source value 是否仍
live、unique backing 是否可以转移。Verifier 必须证明 `move_slot` 后源 slot 不再被读取。

不存在 transitive-function `call_suspend`。不存在仅为 VM 引入、但没有语言语义 owner 的 `yield` 或
`make_closure`。如果语言 reference 未来增加 closure/yield，再独立扩展 ISA。

## 4. Admission verification

Runtime 只能执行 verified linked image。Verifier 必须至少检查：

- opcode/version 已知，instruction word 边界完整；
- pool、slot、shape、type tag、effect adapter 与 target index 全部在界内；
- 所有 relocation 已解析为允许的 exact target kind；
- jump/switch/handler/resume pc 都指向 instruction header；
- 每条 CFG path 无 stack underflow，merge point 的 stack height 与 slot state 一致；
- 重算的最大 operand depth 不超过 artifact 声明；
- opcode 的 typed stack input/output 与 linked type/shape plan 相容；
- exception region 正确嵌套，handler stack height、catch slot、matcher 与 cleanup depth 合法；
- pending-capable instruction 有唯一、合法的 resume descriptor；
- declared `NoPending` guarantee 的函数不可到达 pending-capable effect；
- `tail_call_local` 满足 exact local target、return plan equivalence 与 cleanup-region eligibility；
- `move_slot`、builder edit token 和 writable path 不产生 use-after-move 或同时可写 alias；
- statement/source tables 有序、无越界，并覆盖所有可抛错或 effect instruction；
- frame/stack/constant/object size 不溢出 runtime resource accounting。

Unknown effect/target/type fact 必须 fail closed 或保守降为允许 Pending/escape 的普通 verified instruction；不能
把 unknown 当作 sync、pure、unique 或 local。

## 5. Callable effects and `mayPending`

### 5.1 One effect analysis

Compiler 现有 `CallableEffectSummary` 方向保留并成为唯一 owner。至少继续表达：

```text
writesCallerReachable
returnsCallerAlias
throwsCallerAlias
escapesCallerValue
requiresSameHeapIdentity
invokesUnknownTarget
mayPending
```

当前字段 `maySuspend` 在迁移时删除，由 `mayPending` 取代：函数的某条 reachable path 能执行到一个
pending-capable effect。它是 sound transitive may-analysis，不表示本次调用会挂起。

分析对 local call graph SCC 做 fixed point；dependency summary 从 exact PackageArtifact 取得；interface、
service、callback 或未知动态 target 没有更强保证时保守包含 `Unknown` pending effect。

可以进一步保存 pending-effect categories：

```text
Db
Network
Service
Actor
Timer
StreamRead
StreamWrite
File
Callback
Unknown
```

`mayPending` 是该集合非空的派生结果，而不是与集合并列的另一份事实。

### 5.2 What effect analysis controls

`mayPending`/effect summary 用于：

- const/frozen initializer 与其它 pure/no-pending context 验证；
- sync-only callback/native boundary；
- caller-writable/exclusive access 的 callee 是否满足首版 `NoPending` guarantee；
- concurrent lane mutation/escape/effect 检查；
- package/interface implementation effect guarantee；
- compiler/JIT sync-only region 优化；
- verifier 对 bytecode 实际 effect 的交叉检查。

它不用于：

- 选择 local `call` 与 `call_suspend`；
- 在函数入口分配 continuation；
- 决定本次 effect 是 Ready 还是 Pending；
- 阻止 eligible exact-local tail call。

如果所有调用者都允许 fiber parking，`mayPending` 不属于 local call ABI。只有声明 `NoPending` 的 boundary 才
把它作为 effect guarantee；implementation 的 effect set 必须是 boundary allowance 的子集。

Lowering 的独立 `suspend_analysis` 在迁移完成后删除。Bytecode emitter 消费 canonical effect facts，并从
resolved leaf target 选择 sync intrinsic 或 pending-capable `invoke_effect`，不得重新推断整条 call graph。

## 6. Runtime value model

### 6.1 Compact value slot

VM hot storage 使用固定宽度 `ValueSlot`。首版性能目标是 16 bytes；这是 runtime layout，不是 artifact ABI：

```text
ValueSlot
  payload: u64
  metadata: u64
    kind
    flags
    compactTypeTag
```

Immediate value：

```text
null
bool
number
integer/date when represented immediately
```

Reference value：

```text
RequestHeapRef
ActorStateRef
ConstRef
ResourceRef
```

`string`、`bytes`、Array、Map、record/object、Actor reference、interface、exception 等非小型 payload 不再
inline 到 Rust enum。一个包含多个 `String`/`Vec` 的 host struct 不能决定所有 operand slot 的大小。

### 6.2 Nominal and catch identity

删除旧 `RuntimeValueCarrier` 包装不能删除它承载的语义。`ValueSlot.compactTypeTag` 或 heap object header 中的
`TypeTagId` 必须精确区分：

- nominal record/representation；
- named-union enclosing context 与 concrete/synthetic branch；
- catch leaf identity；
- interface payload 所需 concrete identity。

`LinkedBytecodeImage` 保存 `TypeTagId -> full stable identity/type plan`。`0` 可以表示无 nominal identity。
Throw、catch、boundary encode 和 recoverable encode 通过 table 展开完整 identity，不从 runtime shape 猜测。

### 6.3 Handle discipline

Heap reference 使用 stable table index，并包含足够的 heap-space/generation validation；不能保存裸 object
pointer。Collector 移动物理对象时只更新 handle entry，不重写所有 `ValueSlot`。

Request handle 不能进入另一个 request、artifact、durable record 或 boundary payload。Actor/constant/resource
reference 是不同 kind，不能伪装为 request handle。

## 7. Separate memory domains

Runtime 内存分成以下 owner：

```text
LinkedBytecodeImage / ConstantHeap
  immutable, shared across requests

VmFiber
  frames, slots, operand stack, regions

RequestManagedHeap
  request-local language values

ActorStateHeap
  instance-local, versioned, cross-request state

ResourceTable
  host resources requiring explicit close/cancel/drop
```

### 7.1 Constant heap

Top-level frozen `const`、string/bytes literal、shape/type descriptors 与 immutable lookup table 随 execution image
共享。加载 constant 不做 request allocation；需要写入的 `var` 在第一次物理 mutation 时转成 request-owned
representation。

### 7.2 VM stack

Frame、slot 与 operand stack 不放进 `RequestManagedHeap`。它们使用 request-owned contiguous vector/page
segments，纳入统一 memory budget，但遵循 call/return truncate 和 tail-frame replacement。

### 7.3 Resource table

Stream endpoint、socket、native file staging、callback capability、timer registration 等有 destructor/cancel
语义的对象进入 `ResourceTable`。GC value 只保存 `ResourceRef`；resource 的 close/cancel 由 lexical region、
request terminal 或 explicit API 驱动，不能依赖 tracing collector 的不确定时机。

## 8. Collection and record representation

### 8.1 Semantic dependency

本架构依赖普通 record、Array、Map 采用 value semantics：复制、赋值和默认参数传递产生逻辑 snapshot；之后
通过一个 writable `var` 修改，不改变之前的 `let`/caller snapshot。实现可以共享 backing 或原地写唯一
backing，但不能暴露 reference identity。

Source 层的 caller-visible mutation 必须使用显式 writable mode（本文称 `InOut`；最终语法由 reference
定义）。普通参数是 value；compiler 可按 liveness/escape 选择 physical borrow、move 或 share。

### 8.2 Performance-first physical model

Value semantics 不等于 persistent tree，也不等于每次 mutation 全量复制。Canonical common path 是：

```text
Array
  SmallInline/SmallFlat
  UniqueFlat(ValueSlot buffer)
  SharedFlat
  optional PersistentChunks when benchmarks justify it

Map
  SmallLinear
  UniqueHash
  SharedHash
  optional Persistent/Chunked form when benchmarks justify it
```

新建 collection 拥有 unique edit token：

- `move_slot` 保留 token；
- source value 仍 live 的 `copy_slot`、escape、freeze 或跨 lane share 使 backing 变为 shared；
- unique Array `push` 是 amortized O(1) contiguous-buffer append；
- unique Map `put` 是 expected O(1) hash-table update；
- shared backing 第一次写入执行 COW 或切换 adaptive representation；新 backing 随后重新 unique；
- implementation 可以保守地让 shared backing 永不恢复 unique，不需要 hot-path retain/release。

Persistent vector/HAMT 不是默认要求。只有“大 collection 高频 snapshot 后继续更新”的真实 benchmark 证明
COW 复制成本占主导时，才引入 adaptive representation。

### 8.3 Builders and freeze

Literal、decode、DB result、`Array.map/filter/concat`、`Map.merge` 和 compiler 证明不逃逸的 loop accumulator
使用 transient builder：

```text
create unique builder
  -> repeated in-place append/put
  -> freeze/share
```

`Array.concat` 先计算总长度并一次分配；singleton literal 不要求真实构造中间 Array。若左侧 value 已死且
backing unique，runtime 可直接复用并扩容。`Map.merge` 同理。

### 8.4 Dense records and dynamic maps

Compiler-known record 使用 `ShapeId + dense ValueSlot fields`。字段 opcode 携带 verified field offset，不在热
路径查询字符串 `BTreeMap`。

动态 JSON object/Map 使用独立 representation；key 可以 intern/compact，lookup 使用适合 workload 的 hash
table。若语言要求 canonical iteration order，该要求必须写入 reference；否则只在 boundary canonical encode
时排序，不能迫使所有 runtime mutation 使用 tree map。

### 8.5 Strings and bytes

String/bytes 对用户保持 immutable value。实现可以组合：

- small inline representation；
- immutable shared flat buffer；
- multi-part concat/chunk/rope；
- flatten cache；
- compiler-proven unique builder。

链式 concat instruction 计算总长度并一次 materialize。Loop reducer 不得因每轮复制全部 prefix 保持
O(n²)；采用 chunk/rope 或已证明唯一的 builder，最终 boundary flatten 为 O(total bytes)。

## 9. Request-local managed heap and GC

### 9.1 No explicit short/long request classes

每个 request 使用同一种 GC-capable heap：

```text
RequestManagedHeap
  stable handle table
  bump nursery pages
  mature/retained pages as needed
  large-object storage
```

“启用 GC”表示 value/native/scheduler 遵守 safepoint 和 root protocol，不表示每个请求都运行 collection：

- 未跨过 allocation threshold 的请求在 terminal 时直接释放全部 pages，零 tracing cycle；
- 短但分配巨大的请求可以收集；
- 长但低分配、长期 Pending 的请求可以从不收集；
- duration、HTTP/stream 入口类型和 await 次数不直接触发 GC。

首版 collector 可以是 copying nursery、mark/compact 或其它 per-request precise collector；artifact ISA 和
ValueSlot 只依赖 stable handle 与 safepoint contract，不依赖具体算法。

### 9.2 Roots and safepoints

精确 roots 包括：

```text
runnable/waiting VmFiber slots and operand stacks
active exception/cleanup regions
PendingOperation registered roots
request context values
transaction/Actor overlays owned by this request
resource payload roots explicitly registered by adapters
```

Safepoint 可以位于 allocation slow path、effect enter/return、loop/backedge budget poll 和 scheduler handoff。
到达 safepoint不等于运行 collector；只有 heap pressure flag 已设置才收集。

同一 request 的 bytecode 执行是 cooperative single-owner。External I/O 可以并行，多个 fiber 可以同时
Waiting，但 collector 只在没有 fiber 正修改 heap、所有 pending owner 已登记 roots 时运行。

### 9.3 No raw borrow across allocation or await

Native/effect adapter 可以在一个无分配 instruction scope 内短暂 borrow heap object；在 allocation、GC、
Pending return 或 await 之前必须结束 borrow。Pending operation 只能保存：

- stable handle + registered root；或
- 已 boundary-materialize 的 owned bytes/host value。

违反该规则是 runtime boundary bug，不能用 pin all objects 规避。

### 9.4 Unified memory budget

Request memory limit 统计实际 reserved/owned capacity，而不只估算 heap node payload：

```text
heap pages and handle table
VM frame/value stack pages
pending operation buffers and roots
string/bytes/collection capacities
transaction journal/edit overlay
resource estimates
Actor invocation overlay
```

达到 soft limit 时先收集；收集后仍超过 hard limit 才产生结构化 memory resource error。Request terminal
直接释放全部 request pages、fiber、pending owner 和 resources。

## 10. Frame, call, tail call, and arguments

### 10.1 Fiber/frame layout

```text
VmFiber
  state: Runnable | Waiting
  currentFunction
  pc
  frames: [Frame]
  values: [ValueSlot]
  regions: [ActiveRegion]

Frame
  functionIndex
  returnFunction/returnPc
  slotBase/slotCount
  operandBase/operandDepth
  callSite
  instantiatedTypeContext
```

Compiler 计算每函数 max operand depth；verifier 重算。进入 frame 时一次扩展对应 contiguous segment，普通
instruction 不触发 stack reallocation。Return 截断 segment 并把结果移入 caller destination。

Non-tail recursion 不再增长 native stack，但仍受统一 frame/memory/instruction budget 约束；耗尽必须返回
结构化 resource error，不能依赖 host stack overflow。

### 10.2 Local call

`call_local` 按源码顺序求值参数，随后：

1. 根据 compiler liveness 以 borrow/move/share 方式准备 `ValueSlot`；
2. 创建 callee frame 和 exact slots；
3. 设置 caller return pc；
4. 在同一个 dispatch loop 切换到 callee；
5. 不检查 callee transitive `mayPending`，不创建 future。

Callee 深处若实际 effect Pending，整个 fiber（包括 caller frames）一起停放。

### 10.3 Tail call

Eligible exact-local tail call 发射 `tail_call_local`：

- 参数在 caller frame 仍 live 时按顺序求值一次；
- return plan、nominal tag、generic/self 与 cleanup eligibility 遵守
  `tail-call-execution.md` 的用户语义；
- 替换当前 frame/value segment，不 push 新 frame；
- 每个 hop 保留 call/function-entry budget charging；
- eliminated tail edge 不增长 diagnostic stack；当前 edge 失败使用当前 call site；
- callee 可以在后续 effect point Pending，不影响 tail eligibility。

VM 上线后，`tail-call-execution.md` 中为树遍历 evaluator 设置的 Rust async trampoline/native-stack 实现细节
由本节替代；其用户可见 eligibility/error contract 在对应 reference 更新前继续有效。

### 10.4 Explicit caller-writable arguments

若 reference 引入 `InOut`：

- argument 必须是 writable `var` access path；
- compiler/verifier 保证一次调用期间的 exclusive writable access；
- backing unique 时 callee 可原地写；backing 已被 snapshot 共享时仍必须 COW，不能改变旧 snapshot；
- call boundary 本身不复制 value；
- 首版 caller-writable callee 必须具有 verified `NoPending` guarantee，exclusive loan 不跨 effect suspension；
- ordinary throw 时 caller binding 的 writeback/rollback 语义由 reference 明确定义，VM 以显式 writable
  region 实现，不能依赖 Rust borrow 析构时机。

未来若允许 caller-writable loan 跨 Pending，必须先为 fiber-owned loan、ordinary throw writeback、
concurrent sibling exclusion、cancel cleanup 和 GC roots 定义新的 reference/region contract；不能只删除
`NoPending` verifier check。

## 11. Actual-Pending suspension

### 11.1 Pending-capable effect instruction

只有 leaf effect instruction 可以停放 fiber：

```text
invoke_service
invoke_actor
invoke_http
invoke_db
stream_next
stream_emit/backpressure
timer_wait
file_io
invoke_callback
invoke_unknown
```

这些在 artifact 中可以统一为 typed `invoke_effect` + linked adapter descriptor。Pure native/collection/record/
arithmetic/local-call instruction 不能返回 Pending。

Adapter 启动结果：

```text
EffectStart
  Ready(ValueSlot)
  Pending(PendingOperation)
```

Buffered stream read、cache hit 或立即完成的 adapter 可以返回 Ready；pending-capable 不意味着必然 Pending。

### 11.2 Park

执行 `invoke_effect` 前，verifier 已知 stack input、result type、source site 和 resume descriptor。若返回
Pending：

1. effect arguments 按 adapter contract move/materialize/register roots；
2. fiber 记录 `{pendingId, resumePc, expectedStackHeight, resultPlan, sourceSite}`；
3. `VmFiber` 从 runnable queue 移到 request waiting table；
4. request scheduler 拥有 `PendingOperation`、deadline/cancel registration 与 wake；
5. dispatch loop 返回 scheduler，不复制/序列化 frame chain。

挂起状态是原有 fiber 的所有权移动，不是每层函数的 heap continuation，也不是 persisted durable state。

### 11.3 Resume

Operation 完成后产生：

```text
Resume
  Value(value)
  Throw(error)
  Cancelled(terminal)
```

成功时 result 按 plan 导入 caller request heap/stack，pc 设为 resume pc，fiber 回到 runnable queue。

失败时 scheduler 不沿 Rust future stack 返回 error；它在原 effect source site 向 VM 注入 error，随后走与
同步 throw 相同的 exception/cleanup unwind。Cancellation/internal stop 走 terminal unwind，不伪装成普通
catchable value。

Budget、profiling、type substitutions、Actor frame、transaction region 和 local diagnostic prefix 都属于
fiber/request state，挂起前后不重置。

### 11.4 Cross-owner calls

Exact local program call 始终是 `call_local`。Service/Actor/callback 等跨 owner 调用始终是 effect boundary：

```text
caller fiber/request heap
  -> boundary encode/freeze
  -> PendingOperation
  -> provider activation with its own fiber/heap
  -> boundary result/error
  -> caller decode + resume
```

Provider 和 caller 不共享 raw request handle。Same-process optimization 也必须保持 owner、heap、error channel 与
activation generation boundary。

### 11.5 Structured concurrency

`concurrent` 为每条 lane 建立独立 `VmFiber`，共享 request scheduler 和允许共享的 immutable value roots。
多个 pending I/O 可真实重叠；bytecode heap mutation仍由 single-owner scheduler 串行执行。

Join、winner、cancel 和 deterministic error selection 属于 structured-concurrency owner。Lane fiber terminal
不能自行结束整个 request，也不能跳过 sibling cancellation/cleanup。

## 12. Exceptions, regions, and cleanup

每函数 exception table 至少包含：

```text
ExceptionRegion
  startPc/endPc
  handlerPc
  handlerStackHeight
  catchMatcher
  catchSlot
  cleanupDepth
```

`catchMatcher` 是 linked catch-leaf/type-tag matcher，不是单个静态 `CatchIdentity`。Handler 进入前 VM 截断
operand stack、写入 exception/catch result slot，并保持原 correlation/source envelope。

Fiber 另有 active region stack，用于不能仅靠 pc jump 表达的 owner：

```text
timeout scope
DB transaction/lease
structured concurrent join
stream consumer/producer supervision
with/resource lifetime
Actor invocation/commit overlay
```

Region 定义 normal-exit、throw、cancel/internal-stop 的 cleanup/commit/rollback action。Pending 只停放 fiber，
不退出 region；resume error 从相同 region depth 开始 unwind。

Transaction snapshot 不再深 clone/rebase整个 heap graph。Value-semantic root checkpoint 配合 edit epoch：

- checkpoint 前可回滚 root 在 transaction 内不得无 journal 原地破坏；
- first write 可以 COW，或记录精确 mutation journal；
- rollback 恢复 root/slot/region checkpoint，之后 GC 回收不可达新节点；
- commit 发布新 roots并丢弃 journal；
- nested transaction 使用嵌套 epoch/journal boundary。

## 13. Actor, durable, and boundary memory

Actor state 不使用普通 request heap lifetime。`ActorStateHeap` 是 instance-local、versioned、可 compact 的独立
owner；Actor invocation 使用 immutable snapshot/root + transactional edit overlay：

- method entry 冻结 exact actor generation/snapshot；
- invocation 内 unique state path 可使用 edit token；
- commit 原子发布新 root/generation；
- rollback 丢弃 overlay，旧 root 不变；
- request-local resource/stream/callback 不能被写入 Actor state；
- Actor compaction 更新 Actor handle/epoch，不扩大所有普通 request handle。

Service boundary、durable task、recoverable value 和 DB persistence 保存 stable schema/code/type identity 与编码
payload，不保存 `ValueSlot`、request handle、GC generation、pc 或 pending operation id。

## 14. Execution budget, profiling, and source attribution

### 14.1 Charging versus polling

Semantic instruction charging 与 deadline/cancel poll 是不同机制：

- emitter/ISA 为 statement、expression node、function entry、local call、loop condition/backedge、generated
  chunk 等当前语义点生成稳定 charge metadata/operation；
- VM 可以批量把累计 units 提交给 execution control；
- poll 仍由现有 interval、hard limit、function/loop/effect boundary 触发；
- quickening、superinstruction、decoded op 数量或 JIT machine instruction 数不能改变 semantic units。

VM 迁移首版必须保持当前可观察 instruction-count/budget fixtures。若要重新定义单位，先修改 reference 与
budget contract，不能以“bytecode 每 N 条计一次”暗中改变。

### 14.2 Statement profiling

Artifact 分开保存：

```text
StatementEntry
  pc
  statementId/function attribution

SourceMapEntry
  pc range
  InstructionSourceSite
```

每次控制流进入 statement entry 恰好记录一个 profiling unit。Source map 覆盖 call、throw、effect、DB、
timeout 和 compiler-generated instruction，不能用 statement offset替代精确 call/effect site。

Decoded fusion/JIT 必须保留 statement hook 与 semantic charge；挂起/恢复不能重复计算已经进入的 statement，
也不能漏记 resume 后的新 statement。

### 14.3 Diagnostic stack

Frame 保存 exact call site；throw/effect error 使用当前 source map site。Non-tail local frame按 unwind 生成 stack
trace；eligible tail replacement 遵守 bounded tail diagnostic contract。Cross-service/provider frame通过既有
canonical error channel逐跳投影，不能拼接进程内 pointer/debug frame。

## 15. Performance contract

性能是本架构的一等完成目标，不是实现后的可选优化。至少保持以下结构性保证：

- sync local call/return 在预留 frame/value capacity 内零 host heap allocation、零 Rust future；
- slot load/store 复制/移动固定宽度 `ValueSlot`，不 clone inline `String`、ActorRef 或完整 catch identity；
- constant load 不产生 request allocation；
- dense record field access 是 verified offset access；
- unique Array push 为 amortized O(1)，unique Map put 为 expected O(1)；
- literal/decode/map/filter/concat 使用一次或分段预估后的 builder，不按元素复制完整 collection；
- low-allocation request 可以零 GC cycle；
- Ready effect 不停放 fiber、不分配 program continuation；
- Pending effect 只分配一个 pending owner并移动已有 fiber；开销不随 local call depth复制；
- eligible tail recursion active frame/value/diagnostic space 为 O(1)；
- long-lived allocation-heavy request 能回收 unreachable intermediate value，不随总历史分配量单调增长；
- runtime micro-op quickening不扩大 artifact ISA。

实现计划必须建立 release benchmark suite并固定 baseline，至少覆盖：

```text
pure expression/control loop
deep sync local calls and tail calls
dense record projection
unique Array/Map build and mutation
shared snapshot followed by COW mutation
JSON/DB materialization
string reducer/concat
Ready effect
Pending park/resume
long request allocation/collection
real Agine LLM SSE reducer/chat smoke
```

目标是 sync interpreter-only 热路径相对当前树遍历 async evaluator 获得数量级级别改善（原始预期
5-20x），但任何具体阈值必须在 implementation/benchmark plan 中绑定 workload、release profile、机器、
统计口径和 baseline commit。不能用 I/O latency 掩盖 interpreter regression，也不能只以 microbenchmark
替代真实 chat path。

## 16. Cross-document and completion contract

本架构只有在以下条件全部满足后才完成：

- `../reference/syntax.md` 明确 `var`、local immutable `let` 与 top-level frozen `const`；
- `../reference/static-semantics.md` 和 `../reference/runtime.md` 从 mutable collection reference semantics
  收敛到 value semantics、writable lvalue、默认 value argument与显式 caller-writable mode；
- `../reference/std-surface.md` 明确 mutating receiver API 与 static transformation API，例如 receiver
  `push/put`、static `Array.concat`/`Map.merge`，不依赖英语词形区分 pure/mutating；
- compiler callable effect analysis 成为唯一 owner，独立 lowering suspend analysis 删除；
- artifact opcode/operand/stack-effect schema 单一声明并生成 compiler/runtime constants；
- PackageArtifact 保存 relocatable bytecode，RuntimeAssembly admission 产出 verified linked image；
- canonical assembly 与 test fixture 共享同一 emitter/linker/verifier/VM；
- request heap、Actor heap、constant heap、resource table 与 VM stack owner 分离；
- sync、actual-Pending、resume error、timeout/cancel、catch/rethrow、transaction rollback、stream cleanup、
  Actor commit、tail call、GC root 和 memory limit 都有 focused test；
- source-to-artifact-to-link-to-runtime 真实路径、request-local GC pressure test 和 Agine chat smoke 通过；
- production tree evaluator、old artifact reader、`call_suspend`、test-only evaluator 和 compatibility fallback
  全部删除。

新增/修改 artifact instruction 必须同时更新 canonical schema、emitter、linker、verifier、decoded runtime、
source/statement mapping 和 focused tests。Physical ValueSlot/collector/micro-op optimization只要保持本文件的
semantic/runtime invariants，可以不升级 artifact ISA；opcode/operand/stack semantics 变化必须升级 ISA/schema
version。

临时迁移顺序、feature branch、dual-run differential harness 和逐组件删除计划属于 `../implementation/`，
不得写回本长期 architecture contract。开发期间可以在测试中 differential-run 新旧 evaluator，但
production artifact/runtime 不允许双轨。
