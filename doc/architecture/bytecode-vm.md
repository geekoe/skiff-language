# Bytecode VM and Runtime Memory Architecture

本文是 Skiff bytecode VM、deployment execution image、运行时内存与挂起/恢复机制的目标内部架构事实源。
它面向 compiler、artifact、deployment loader/linker、runtime、host adapter 与 profiler 维护者，规定
relocatable ISA、两阶段验证、值布局、collection 物理表示、request-local GC、Actor shared heap、VM
fiber、跨 owner trampoline、异步 unwind 和执行归因之间的长期边界。

Skiff 尚未发布。本架构落地时整体升级 artifact/runtime 格式并删除树遍历 async evaluator；production
不保留旧 artifact reader、双解释器或按版本 fallback。

本文依赖的用户可见语义已在`../reference/`收敛：普通aggregate value semantics、
top-level frozen `const`、local `var` / immutable `let`，以及只允许Package Local ABI使用的
显式`InOut`。本文只规定它们的VM物理实现与验证边界。

## 1. Decisions and invariants

完成态必须满足以下不变量：

- Runtime 以 release pointer 解析出的 deployment `buildId` 为加载单位。loader 从该 deployment 及其
  package closure 构建、验证并缓存 immutable `DeploymentExecutionImage`。旧`RuntimeAssembly`
  record已删除，不再是任何现行数据模型的名称。
- 所有 service/package program code 通过一个扁平、同步的 bytecode dispatch loop 执行；普通 local call、
  dynamic local interface call 和同步完成的跨 owner child call都不递归进入 Rust evaluator，也不创建逐层
  Rust future。
- VM 只在一个具体 host operation 或 owner acquisition 本次实际返回 `Pending` 时停放 fiber。静态
  `maySuspend` 不能产生预先挂起、虚假 yield 或另一套物理 local call convention。
- service、Actor 和 callback 的同进程调用通过 scheduler trampoline 切换到独立 owner/fiber/heap；child
  同步完成时直接回到 parent，child 真正等待时才 park 调用链。
- Actor 保留 instance-owned shared arena。同步段写入立即可见，失败不回滚；只有 actual `Pending` 才释放
  Actor segment lease，恢复前重新 acquire 并校验 fence/arena epoch。
- artifact 保存 relocatable ISA。任何 linker 索引访问前先做 structural validation；link 后再做 CFG、类型、
  exact target、effect、exception、resume 与 `NoPending` semantic verification。
- VM slots、operand stack 和 collection elements 使用紧凑、固定宽度的 value slot。nominal/catch/union
  identity 必须保留，但不在每个值上复制完整 identity object。
- 普通 record、Array 和 Map 对用户采用 value semantics；实现以 move、root share transition、path COW 和
  transient builder 保持常见 mutation 热路径。`dup` 不是未经追踪的 bit copy。
- VM frame/value stack 与 managed heap 分离。call return 回收 frame segment；eligible tail call 原地替换
  frame，空间不随 tail hop 增长。
- 每个普通 request owner 使用独立、GC-capable managed heap。短且低分配请求可以零 collection 结束；是否
  运行 collector 只由 allocation pressure 决定，不由请求持续时间或入口类型决定。
- sync/ready 热路径、execution budget、profiling statement units、source attribution、exception identity、
  timeout/internal-stop 与 recoverable boundary 语义不得被 VM 实现细节暗中改变。
- artifact ISA 不暴露 Rust pointer、allocator address、`Vec` layout、native future layout、decoded micro-op
  layout 或 runtime quickening 细节。

本架构不承诺首版 JIT，也不定义 global process heap、跨 request raw heap sharing、持久化 continuation，
或同一 managed heap 被多个 OS 线程同时修改。它必须为后续 baseline/template JIT 保留显式类型、控制流、
stack effect 与 safepoint 信息。

## 2. Deployment image and canonical owners

### 2.1 Load path

请求执行的 canonical load path 是：

```text
(profile, serviceId, version)
  -> release pointer
  -> deployment buildId
  -> per-buildId deployment loader
       -> bounded artifact decode + pre-link structural validation
       -> exact deployment + PackageArtifact/File bytecode closure
       -> link relocations and deployment-owned capabilities
       -> initialize/freeze ConstantHeap
       -> post-link semantic verification
  -> immutable DeploymentExecutionImage
  -> request/Actor/callback execution owner
```

`DeploymentExecutionImage` 至少包含：

```text
DeploymentExecutionImage
  owner: DeploymentOwnerIdentity
  linkedCode: LinkedBytecodeImage
  constantHeap: ConstantHeap
  operationEntries
  gatewayEntries
  packageDirectBindings
  serviceDependencySlots
  type/shape/recoverable plans
  deployment-owned capability descriptors
```

`DeploymentOwnerIdentity` 以 exact deployment `buildId` 为不可替代事实，并保留诊断所需的 service id/version
与 deployment artifact identity。Request、stream、callback capability 和 Actor implementation pin 都引用
这个 owner，而不引用 ambient release state。

同一 `buildId` 的 image 内容必须唯一。并发 load 共享 per-buildId critical section；失败不发布半 image，所有
waiter 得到同一失败。已发布 image immutable；若实现增加 LRU，in-flight owner 仍以强引用 pin image，逐出只
影响下次 load，不改变语义。

### 2.2 ReleaseBundle 只是离线清单

旧`RuntimeAssembly` record删除。publish/verify/promotion若需要聚合一批immutable refs，使用
可选`ReleaseBundle`：它可以列出一组deployment refs、聚合验证结果并提供可复现的
bundle identity。`ReleaseBundle`不得：

- 产生 runtime-local executable address；
- 成为 runtime loaded-set key；
- 出现在 request/fiber/stream/callback 的执行 identity 中；
- 拥有 active/current、prepare/commit、generation、lease 或 switch 语义；
- 作为缺少 deployment image 时的 runtime fallback。

测试 fixture 必须走同一个 deployment loader/linker/verifier/VM，可以使用内存 artifact
store，但不能构造 test-only bundle admission 或第二套 evaluator。

### 2.3 Service dependency resolution

`DeploymentExecutionImage.serviceDependencySlots` 只保存 `(serviceId, exact version, expected protocol identity,
operation)` 等 contract facts，不把 provider executable address 固化为当前 profile 的全局 assembly binding。
执行一次 service boundary call 时，boundary scheduler 解析 release pointer 得到 exact provider `buildId`，
校验 protocol identity，加载或取得 provider image，并为该 invocation pin provider owner。

因此不存在跨 service 的 atomic generation snapshot。Pointer 更新不影响已开始的 invocation/stream/callback；
后续新的 service call 可以解析到新 build。若将来需要多 service 原子部署，那必须定义独立、显式的 deployment
transaction contract，不能重新把 ambient multi-service generation 塞进 VM。

### 2.4 Responsibility split

1. **Source analysis** 是 callable effect、writable loan、capability provenance、escape 和
   `maySuspend` 的唯一 owner。
2. **Bytecode emitter** 负责控制流、stack effects、constant graph、relocation、frame layout、exception
   region、statement entry、source map 与 synthetic callback body，不制造 runtime address。
3. **Structural validator** 在 linker 前验证不可信 artifact 的格式、边界、索引和资源上限。
4. **Deployment linker** 消费 exact package/deployment facts，把 relocation 解析为 image-local target、type、
   shape、effect adapter、capability 和 const entry。
5. **Semantic verifier** 不信任 compiler/linker summary，独立证明 linked code 可安全执行。
6. **VM core** 只拥有 program frame、value stack、control flow、unwind 和 invocation protocol；它不直接实现
   DB、HTTP、release pointer、Actor registry 或 native resource lifetime。
7. **Scheduler/adapters** 拥有 child transfer、pending operation、wake、cancel、deadline、fiber queue 和 host
   I/O。Adapter 不得保存可被 GC 移动的裸 pointer。

## 3. Artifact ISA and runtime micro-ops

### 3.1 ISA means

ISA（Instruction Set Architecture）是持久化 bytecode 的语义契约，包括：

- opcode 与 operand schema；
- 每条 instruction 的 operand-stack input/output；
- branch、call、tail call、return、throw、callback 和 effect invoke 语义；
- constant、type、shape、target 与 relocation index 的含义；
- verifier rules；
- ISA/schema version。

ISA 不包括 runtime 内存地址、decoded Rust enum 大小、dispatch optimization、superinstruction 或 JIT machine
code。

### 3.2 Wordcode

Artifact bytecode 使用 wordcode：

- code 是 `u32` word sequence；
- `pc` 是函数 code 内的 word offset；
- 每个 opcode 在该 ISA version 中有固定 operand word 数；
- operand 只保存 immediate、relative branch、slot/pool index 或 relocation index；
- multi-operand instruction 占多个 word，不把任意 runtime address 强塞进一个 `u32`；
- jump target 必须指向 instruction header，不能落入 operand word。

```text
RelocatableBytecodeFunction
  functionKey
  words: [u32]
  relocations: [BytecodeRelocation]
  frameLayout
  maxOperandDepth
  exceptionRegions
  statementEntries
  sourceMap
  effectSummaryRef
```

Package artifact 另保存 bounded frozen constant graphs、type/shape declarations、callback capture layouts 和
debug table。Debug binding name 不扩大 runtime frame。

### 3.3 Relocation and dynamic targets

Artifact relocation 至少区分：

```text
LocalExecutableRef
PackageCallableRef
ServiceOperationRef
ActorMethodRef
InterfaceRequirementRef
SyntheticCallbackRef
HostEffectRef
TypeRef
ShapeRef
FrozenConstantRef
```

`InterfaceRequirementRef` 只解析 interface identity、method slot 和 canonical signature；它不能被 linker
伪装成一个 exact executable target，因为 concrete receiver 只在运行时可知。合法动态行为必须由
`call_interface` 等显式 opcode 表达。

Link 后的 code image：

```text
LinkedBytecodeImage
  functions
  exactLocalTargets
  serviceOperations
  actorMethods
  interfaceTables
  syntheticCallbacks
  hostEffectAdapters
  typeTags
  shapes
  frozenConstants
  source/debug tables
```

Package code 不因不同 deployment 被原地 patch。Decoded instruction 保存 image-local table index；若未来实测
证明 patch private decoded copy 更快，可以在不改变 artifact ISA 的前提下采用。

### 3.4 Initial instruction families

首版 ISA 至少包含以下语义族；numeric opcode 由 artifact schema 单一声明生成，compiler 与 runtime 不得
各自复制编号：

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
  budget_checkpoint

Call
  call_local
  tail_call_local
  call_service
  call_actor
  call_interface
  return

Callback/interface
  interface_box_local
  interface_box_remote
  make_callback
  invoke_callback

Record/value
  new_record
  get_dense_field
  set_writable_path
  representation_wrap

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

Exception/region
  throw
  rethrow
  enter_region
  leave_region

Host effect
  invoke_host
```

`copy_slot`、`dup`、container store 和普通 by-value argument preparation 都执行 semantic share transition；
它们不能只复制 16 bytes 后留下两个“唯一”edit token。`move_slot` 转移 value 并使 source slot dead；verifier
必须证明之后不再读取。

`tail_call_local` 是新 bytecode ISA 的显式操作。它只由 emitter 在 source eligibility 已确定且 relocation
为 exact-local kind 时产生，并由 post-link verifier 再证明 return plan 与 region eligibility。它有意取代
树遍历 evaluator 时代“禁止 artifact marker”的实现限制，不改变 reference 的 tail-position 语义。

不存在 transitive-function `call_suspend`，也不存在无法验证的 `invoke_unknown`。Restricted callback 有
`make_callback` 与 synthetic function；这不是开放 general first-class closure/yield。

### 3.5 Decoded micro-ops

Runtime 可以在 semantic verification 后把 wordcode 解码为固定 micro-op array并 quicken：

```text
Artifact ISA                         Runtime-only form
call_local relocation               CallLocal(functionIndex)
get field shape/field               GetDenseField(offset)
call_interface requirement/slot     CallInterface(tableIndex, methodSlot)
invoke_host effect                  InvokeHost(adapterIndex, resumeDescriptor)
```

Superinstruction 只存在于 decoded/JIT layer。它不得改变 semantic instruction charging、hard dispatch fuel、
statement profiling、safepoint 或 source attribution；内部含无界循环的 micro-op 必须自行 poll。

## 4. Two-stage verification

### 4.1 Pre-link structural validation

Bounded decoder 和 structural validator 在 linker 读取任何 artifact-controlled index 前执行。至少验证：

- magic/schema/ISA version 与 canonical opcode table 已知；
- artifact、function、word、table、string、constant graph、nesting depth 和单对象大小在配置上限内，所有
  count/offset arithmetic 无溢出；
- instruction word 边界完整，opcode operand 数正确；
- local pool/slot/relocation/table index 在界内，relocation declared kind 与使用 opcode 相容；
- jump/switch/handler/resume target 指向本函数 instruction header；
- exception/source/statement/capture table 结构有序、无重叠非法区间；
- frozen constant graph 是 bounded、无 cycle 的合法 graph encoding；
- artifact identity、内容 hash 与引用记录一致。

Structural validation 的输出是 linker 唯一可消费的 typed validated artifact view。失败不得进入“尽量 link”
路径。

### 4.2 Post-link semantic verification

Link 后 verifier 至少证明：

- exact target、target kind、arity、parameter/return plan 与 opcode 精确匹配；
- 所有 CFG path 无 stack underflow，merge point stack height、slot liveness 与 type state 一致；
- 重算的 max operand depth 不超过 validated declaration；
- typed stack input/output 与 linked type/shape/nominal plan 相容；
- dynamic interface method slot、Local/Remote/Callback 三条 carrier path 共享同一 canonical signature；
- exception region 正确嵌套，handler stack height、catch slot、matcher 与 cleanup depth 合法；
- 每个 pending-capable site 有唯一 resume descriptor，resume result/error shape 正确；
- declared `NoPending` callable 不可到达 pending-capable instruction；
- `tail_call_local` 满足 exact target、return plan equivalence 与 cleanup eligibility；
- move、share、builder edit token、writable path 和 `InOut` loan 不产生 use-after-move 或同时可写 alias；
- callback capture layout 与 synthetic body slot/signature/effect profile一致且不违反 escape policy；
- source/statement tables 覆盖所有 call、throw、effect 和 generated failure site；
- frame/stack/constant/object size 不溢出 runtime resource accounting；
- 每个 CFG cycle 经过 `budget_checkpoint` 或等价受信 checkpoint。

只有 effect summary 的未知性可以保守扩大成 `maySuspend=true` / `Unknown` effect category。未知 target、type、
shape、ABI、carrier tag 或 adapter identity无法证明 stack safety，必须 fail closed；不能用“保守 Pending”掩盖。

## 5. Callable effects and `maySuspend`

Compiler 继续计算 canonical `CallableEffectSummary`；没有理由因为 runtime 只在 actual Pending 挂起就删除它。
至少表达：

```text
effectCategories
inOutPathEffects
resourceAndCapabilityProvenance
callbackProfile
invokesUnknownTarget
pendingEffectCategories
maySuspend = pendingEffectCategories is not empty
```

普通 aggregate 参数、返回和throw payload都是logical snapshot，因此旧
`writesCallerReachable` / `returnsCallerAlias` / `throwsCallerAlias` 不再是aggregate effect fact。
只有显式`InOut` path可以写caller place；资源与capability继续使用它们自己的identity-bearing
provenance/lifetime descriptor，不伪装成aggregate alias。

它是 sound transitive may-analysis，不表示本次调用一定等待。Local call graph SCC 用 fixed point；dependency
summary 从 exact PackageArtifact 读取；`any I`、service、Actor、callback 或其它不能证明 exact synchronous
target 的调用保守为 `maySuspend=true`。

`maySuspend` 有两种同时成立的角色：

1. 它**不改变 VM 物理 calling convention**。所有 local call 使用同一 frame/dispatch loop，不能据此选择
   `call`/`call_suspend` 或在入口创建 continuation。
2. concrete public Package callable 的 summary **仍是 Package Local ABI fact**。变化改变 Local ABI/build，
   直接 package dependency 必须重编译；它也是 `NoPending`、`InOut`、sync callback/native context 和 verifier
   可达性证明的输入。

Interface requirement/conformance 与 ServiceContract 不承诺 implementation 的 `maySuspend`。Service call 本身
始终是 caller 的潜在等待点。未知 effect category 可以保守传播；未知 executable/ABI 不能因此合法化。

Effect summary 用于：

- compile-time constant evaluator 与 pure/no-pending context；
- `InOut` callee 的 `NoPending` 保证；
- callback profile、structured concurrency mutation/escape 检查；
- Package Local ABI 与 dependency recompilation；
- verifier 对 bytecode 可达 effect 的交叉检查；
- sync-only region/JIT optimization。

它不用于决定某次 adapter 是 `Ready` 还是 `Pending`，也不阻止 exact-local tail call。

## 6. Value layout and memory owners

### 6.1 Compact `ValueSlot`

VM hot storage 使用固定宽度 `ValueSlot`。首版性能目标为 16 bytes；这是 runtime layout，不是 artifact ABI：

```text
ValueSlot
  payload: u64
  metadata: u64
    kind
    flags
    compactTypeTag
```

Immediate value 可以包含 null、bool、number，以及可安全 immediate 表达的 integer/date。Reference value 明确
区分：

```text
RequestHeapRef
ActorStateRef
ConstRef
ResourceRef
CallbackClosureRef
```

string、bytes、Array、Map、record/object、Actor reference、interface、exception 等非小型 payload 不以内含
多个 `String`/`Vec` 的 Rust enum 形式占据每个 slot。

### 6.2 Nominal and catch identity

删除旧 carrier wrapper 不能删除其语义。`compactTypeTag` 或 object header 的 `TypeTagId` 必须精确区分：

- nominal record/representation；
- named-union enclosing context 与 concrete/synthetic branch；
- catch leaf identity；
- interface payload所需 concrete/interface identity。

Image 保存 `TypeTagId -> full stable identity/type plan`。Throw、catch、boundary/recoverable encode 通过 table
展开完整 identity，不从 physical shape 猜测。

### 6.3 Handle discipline

Managed reference 使用 stable handle-table index，并带 heap-space 与 stale-handle validation；不能保存裸 object
pointer。Collector 移动物理对象时只更新 handle entry。

Stable handle 只解决地址移动，不自动使对象存活。任何 allocation safepoint 都必须能从 root set 找到该 handle。
Request handle 不能进入另一个 request、artifact、durable record 或 service payload；Actor/constant/resource
reference 是不同 kind，不能伪装。

### 6.4 Memory domains

```text
DeploymentExecutionImage / ConstantHeap
  immutable, shared across requests

VmFiber
  frames, slots, operand stack, regions, UnwindState

RequestManagedHeap
  one owner-local request value graph

ActorStateHeap
  one actor instance shared arena, stable field roots

ResourceTable
  host resources with explicit close/cancel/drop

TransientRootStack
  VM/adapter temporaries across allocation-capable code
```

Frame、slot 与 operand stack 不放进 managed heap；它们使用 request-owned contiguous vector/page segments，
纳入统一 budget。Stream endpoint、socket、file staging、timer registration 和 service callback capability 等
具有显式 lifetime 的对象进入 ResourceTable，不能依赖 tracing GC 的不确定 finalizer。

## 7. ConstantHeap initialization

Top-level `const` 在目标语义中是 deeply frozen、request-independent value，不是每次读取时执行一次普通
function body。Canonical protocol 是：

1. Source checker 只接受 deterministic、pure、`NoPending`、无 request/Actor/resource/callback capability 的
   initializer。Const dependency 按 import DAG 与模块源码顺序解析；forward reference/cycle fail closed。
2. Compiler/build tooling 在受限 `ConstEvaluator` 中执行 initializer。它使用独立 temporary heap、deterministic
   intrinsic set，以及明确的 instruction/allocation/depth/graph-size budget；不得调用 runtime host adapter。
3. Evaluation 失败、超预算、抛错或产生不可冻结 value 时，package build 失败，不生成可在 request 时重试的
   initializer。
4. Artifact 保存 canonical `FrozenConstantGraph` 与 symbolic type/shape/code relocations，不保存需要 runtime
   求值的 initializer body。
5. Pre-link validator 验证 graph encoding、size、acyclicity和索引；linker解析 symbolic identity；semantic
   verifier证明 graph 不引用 request/Actor/resource/closure/pending state或另一个 image 的 raw address。
6. Loader 在 image publication 前构造 immutable ConstantHeap；任一步失败则整个 buildId load 失败。半初始化
   heap 不可见。

Constant load 返回 `ConstRef`，不产生 request allocation。把 const value 存入verified writable root后发生
第一次mutation时，runtime沿修改路径 thaw/COW 到该root所属 RequestManagedHeap 或 ActorStateHeap；原 constant graph 永不
改变。Immutable string/bytes leaf 可以安全共享 backing。Constant graph dedup 是可选物理优化，不改变 identity。

Public-instance top-level const 可以包含 frozen local behavior，并在 link 后取得 method table；它仍不能保存
request callback、live native handle 或 mutable singleton state。需要跨 request 可变状态应使用 Actor/DB，而不是
ConstantHeap。

## 8. Aggregate value representation

### 8.1 Semantic dependency

普通 record、Array 和 Map 采用 value semantics：赋值、普通参数传递、返回和 container store 产生逻辑
snapshot。之后通过verified writable root修改，不改变之前的 binding、caller value 或 container element。
实现可以共享 physical backing，但不能暴露 raw reference identity。

Actor field 是共享 state root；直接写 `self.field` 路径仍修改 actor state。把字段读成普通 local value时遵守
value semantics，得到 O(1) shared snapshot，而不是获得隐藏的 live mutable alias。

### 8.2 Move, share and nested path COW

每个 aggregate node 独立维护 owner/edit state；不只给最外层 collection 打一个 shared bit。Canonical path：

```text
fresh node -> Unique(edit owner)
move       -> transfer same owner, source dead
copy/dup   -> root becomes Shared
write path -> ensure root unique, then descend
              clone each shared node on that path
              share-transition every child handle copied by a clone
              mutate unique leaf/backing
```

复制 root 不需要立即遍历整棵 graph。第一次写入先复制 shared root；复制其 direct child handles 时，那些 child
各自执行 share transition。继续沿目标 path 做同样操作，因此嵌套 record/collection snapshot 不会因最外层
唯一性判断而被错误原地修改。

Single-owner RequestHeap/Actor segment 内的 share state 不要求 atomic refcount。实现可以使用小 refcount、
share epoch 或 sticky-shared bit；若不能证明重新 unique，就保守保持 shared。GC liveness 与 edit uniqueness 是
两套事实，不能互相替代。

### 8.3 Arrays, maps and builders

Canonical common path：

```text
Array: SmallFlat -> UniqueFlat -> SharedFlat
Map:   SmallLinear -> UniqueHash -> SharedHash
```

- unique Array `push` 是 amortized O(1) contiguous append；
- unique Map `put` 是 expected O(1) hash update；
- shared node first write 做 path COW；
- literal、decode、DB result、map/filter、`Array.concat`、`Map.merge` 和 compiler-proven loop accumulator 使用
  transient builder；freeze 后才作为普通 value 流动；
- `Array.concat` 先计算总长度并一次/分段分配；左 operand 已死且 backing unique 时可复用；
- persistent vector/HAMT 只在真实 benchmark 证明频繁大 snapshot 后写入由全量 COW 主导时引入 adaptive form。

Source API 不用 `appending`/`setting` 等细微词形区分 purity。Receiver
`Array.push` / `Array.set` / `Array.pop` 与 `Map.put` / `Map.delete` 只对verified writable path做
mutation；该path可以root于local `var`、有效`InOut` loan，或Actor method中当前规则允许写入的
`self` field。Pure transform 使用一眼可见的 static `Array.concat` 和 `Map.merge`。

### 8.4 Dense records and canonical Map order

Compiler-known record 使用 `ShapeId + dense ValueSlot fields`；field opcode 带 verified offset，不在热路径查询
字符串 map。Dynamic JSON object/Map 使用独立 hash representation。

Map 的用户语义已经要求 canonical key order：`keys()` 与 `for` iteration 在操作开始时生成 snapshot，并按
canonical string payload 的 UTF-8 bytes 升序排列。Mutation 可以继续使用 hash table；只在 snapshot/encode
边界排序，不能把 ordered tree 强加给所有 `put`。

### 8.5 Strings and bytes

String/bytes 是 immutable value。实现可以组合 small inline、shared flat buffer、chunk/rope、flatten cache 与
compiler-proven builder。链式 concat 必须避免每轮复制全部 prefix；最终 boundary flatten 为 O(total bytes)。

## 9. Request-local GC and root protocol

### 9.1 No short/long request class

每个普通 request owner 使用同一种 GC-capable heap：

```text
RequestManagedHeap
  stable handle table
  bump nursery pages
  mature/retained pages as needed
  large-object storage
```

“启用 GC”表示所有 value/native/scheduler 遵守 safepoint/root protocol，不表示每个请求都运行 collection：

- 未跨 allocation threshold 的请求 terminal 时释放全部 pages，零 tracing cycle；
- 短但分配巨大的请求可以收集；
- 长但低分配、长期 Pending 的请求可以从不收集；
- duration、HTTP/stream 入口类型和 await 次数不直接触发 GC。

### 9.2 Complete root set

精确 roots 包括：

```text
runnable/blocked/waiting VmFiber slots and operand stacks
active exception regions and UnwindState payload
parent/child invocation boundary state
PendingOperation registered roots
request/deployment context values
transaction cleanup state and driver-owned pending payload roots
ResourceTable payload roots explicitly registered by adapters
CallbackClosure captures
TransientRootStack entries
```

ActorStateHeap 有自己的 field roots、active/suspended fiber roots与quiescence compaction contract，不混入普通
request collector。

Safepoint 可以位于 allocation slow path、host effect enter/return、loop/backedge checkpoint 和 scheduler
handoff。到达 safepoint不等于运行 collector；只有 pressure flag 设置才收集。同一 heap 的 bytecode mutation
是 cooperative single-owner；collector只在没有 fiber 正修改 heap且所有外部 owner已登记 roots时运行。

### 9.3 Transient roots

Allocation-capable instruction优先在 allocation 成功前把 input operands 留在 VM slots/operand stack。若 VM 或
adapter 必须把 handle 暂存进 Rust local，它必须在从 rooted slot 移除前 push 到 `TransientRootStack`，并在
最后一个可能 allocation/safepoint 后 pop。

Cross-heap materialization 的 source slot、destination builder、partial error payload 都适用同一规则。
Pending owner 接管 handle 时，roots 必须从 transient stack 原子转移到 `PendingOperation`；不能出现两边都
没有 root 的窗口。Debug build 应在 safepoint 断言 adapter borrow为空且 transient protocol平衡。

### 9.4 No raw borrow across allocation/Pending

Adapter 可以在无 allocation/safepoint 的 instruction scope短暂 borrow object；在 allocation、GC、返回
`Pending` 或 scheduler handoff 前必须结束。Pending state只保存 stable rooted handle或已 materialize 的
owned host bytes/value。Pin all objects 不能代替该契约。

### 9.5 Unified memory budget

Request budget统计实际 owned/reserved capacity：heap pages/handle table、VM stack pages、all owner-local child
heaps、pending buffers/roots、collection/string capacities、transaction journals、resources、callback captures
与 boundary materialization buffers。Actor arena有 per-instance 独立上限，同时本次 invocation 的增量工作
计入 request execution budget。

Soft limit 先触发 collection；收集后仍超 hard limit才产生结构化 memory resource error。Request terminal
释放所有 fiber、owner-local heap、pending owner和resource。

## 10. Frames, local calls, tail calls and `InOut`

### 10.1 Fiber/frame layout

```text
VmFiber
  owner
  state: Runnable | BlockedOnChild | WaitingHost | Unwinding | Terminal
  currentFunction/pc
  frames: [Frame]
  values: [ValueSlot]
  regions: [ActiveRegion]
  unwind: Option<UnwindState>
  actorContinuation: Option<ActorContinuation>

Frame
  functionIndex
  returnFunction/returnPc
  slotBase/slotCount
  operandBase/operandDepth
  callSite
  instantiatedTypeContext
```

进入 frame时按 verified max depth一次扩展 contiguous segment；普通 instruction不触发 stack realloc。Return
截断 segment并移入 caller destination。Non-tail recursion不增长 native stack，但仍受 frame/memory/fuel限制。

### 10.2 Local and tail call

`call_local` 按源码顺序求值参数，以 move/share准备 slots，push callee frame，然后在同一 dispatch loop继续。
不检查 callee transitive `maySuspend`，不创建 future；深处 actual Pending时整个 fiber一起停放。

`tail_call_local` 在 caller frame live 时求值参数一次，验证共同 return plan/self/generic/region eligibility后
替换当前 frame segment。每 hop 保留 call/function-entry charge与hard fuel；eliminated edge不增长诊断栈。

### 10.3 `InOut` is Package Local only

目标态 `InOut` 是显式 caller-writable、write-through loan，不是 service wire mode：

- 只允许 exact local/package-direct callable signature；它进入 Package Local ABI identity；
- actual argument必须是 writable `var` access path；compiler/verifier证明调用期间 exclusive；
- callee必须有 verified `NoPending`（即 sound `maySuspend=false`），loan不跨 child/host Pending；
- callee mutation直接更新 caller path。Backing unique时原地写，shared时path COW；
- ordinary throw不回滚已经执行的写入；caller若捕获错误会观察到它们；
- service operation、gateway entry、Actor external method参数、interface requirement、service callback、durable/
  recoverable payload与host effect ABI一律禁止 `InOut`；这些边界总是 value materialization；
- `InOut` 不允许被 concurrent sibling共享或逃逸到 callback/resource。

这样 package helper可以获得显式高性能 mutation，而 ServiceContract 不承诺跨 heap alias、writeback、失败原子性
或远程引用。

## 11. Scheduler trampoline and actual Pending

### 11.1 VM control results

单个 native dispatch driver迭代处理：

```text
VmControl
  Continue
  Complete(Result<ValueSlot, VmError>)
  EnterChild(ChildInvocation)
  Park(PendingOperation)
```

Host adapter启动结果为：

```text
EffectStart
  Ready(Result<ValueSlot, VmError>)
  Pending(PendingOperation)
```

`Ready` 明确包含同步失败；error在当前 source site注入VM，不需先制造future/Pending。只有可信 host adapter
可以返回 Pending。

Service/Actor/callback dispatch adapter 可以进一步返回：

```text
BoundaryStart
  Ready(Result<BoundaryValue, VmError>)
  EnterChild(ChildInvocation)
  Pending(PendingOperation)      # remote transport, actor acquisition, etc.
```

### 11.2 Enter child without native recursion

`EnterChild` 使 parent fiber进入 `BlockedOnChild`，scheduler把 child fiber/owner/heap压入 invocation stack并在
同一个 loop立即执行。它不把 parent放进 host waiting table，不释放 Actor lease，也不计作 suspension。

Child 同步 `Complete` 时，scheduler按 boundary plan materialize result/error到parent heap，弹出 child并继续
parent。整个过程可以跨任意层 owner，但 native stack保持扁平。

Child 遇到真实 host Pending时，scheduler保存 parent/child chain并 park leaf。调用链上持有的 Actor segment
lease在此时按各自规则释放；普通 request heap仍由 rooted fibers拥有。Wake后从leaf恢复，逐层同步完成并返回。

### 11.3 Park and resume

Park前 site 已有 verified stack/result/resume descriptor。Fiber至少保存：

```text
pendingId
resumePc
expectedStackHeight
resultPlan
sourceSite
invocationChain
unwindPhase if cleanup is pending
```

Arguments在 adapter contract下完成 move/materialize/root transfer；fiber进入waiting table；scheduler拥有
PendingOperation、deadline/internal-stop registration与wake。Frame chain不复制、不序列化。

Wake result为 `Value`、`Throw` 或 internal terminal。成功按plan导入并回到runnable；失败在原effect site注入，
走同一个unwind state machine。Budget、profiling、type context、diagnostic prefix与transaction region不重置。

## 12. Dynamic interface and callback execution

### 12.1 `call_interface`

`call_interface` operand包含 verified interface table、method slot、arity和canonical signature；receiver与args
在 operand stack。Runtime读取显式 carrier tag：

```text
Local
  concreteType + methodTable + payload
  -> push exact local frame in same owner/heap

RemoteService
  dependencySlot + publicInstance + operationTable
  -> boundary resolution, then EnterChild or transport Pending

CallbackCapability
  capability owner + operationTable + request lifetime
  -> EnterChild back to capability owner or transport Pending
```

Verifier证明method slot和三条分支的参数/返回shape都等于interface requirement；image construction验证每个
local method table exact target和remote operation table。Unknown carrier/tag/slot fail closed。Local branch
即使静态 `maySuspend` 保守为true，也不会因此yield；remote/callback分支只有actual wait才park。

### 12.2 Restricted callback bodies

当前静态语义已经允许 IIFE、白名单 callback argument和具名 callback reference，因此 bytecode ISA不能把
closure全部推到未来：

- emitter把每个 callback body outline为 `SyntheticCallbackFunction`，保存exact signature、effect summary、
  capture layout、source map和普通bytecode body；
- immutable/value capture通过copy/move进入closure env；writable capture使用显式 fiber-owned cell descriptor，
  只在现有 non-escaping callback profile内合法；
- `make_callback` 创建 `CallbackClosureRef`。Capture slots属于GC roots；adapter若跨actual Pending持有callback，
  必须把closure/root所有权登记到PendingOperation；
- IIFE可以直接调用synthetic function而不物化general closure；
- callback不能进入普通record/container、返回、durable/recoverable payload或越过其静态lifetime；verifier交叉
  检查escape sites；
- 不提供任意first-class function、unbounded escaping closure或隐式callback conversion。

Service API顶层 `any I` callback capability不是 `FnExpr` closure。它由boundary projection产生，拥有显式
deployment owner、interface operation table和request/stream lifetime；它不能进入recoverable/durable payload。

## 13. Exceptions, regions and asynchronous unwind

### 13.1 Exception tables

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

`catchMatcher` 是 linked catch-leaf/type-tag matcher。进入handler前VM截断operand stack、写exception/catch
slot并保留原correlation/source envelope。

### 13.2 Region and `UnwindState`

Active region可以拥有 timeout、DB transaction/lease、structured join、stream supervision、resource lifetime
等 cleanup。Actor invocation没有commit overlay；它只登记segment lease/fence cleanup。

Commit、abort、lease release和stream/resource close都可能Pending，因此fiber必须显式保存：

```text
UnwindState
  reason: Return | Throw | Timeout | InternalStop | ChildFailure
  primaryTerminal
  regionCursor
  cleanupPhase: Enter | Starting(actionId) | Waiting(actionId, pendingId)
                | Completed(actionId) | Done
```

规则：

- `primaryTerminal` 一旦选择就不会因resume/re-poll丢失；
- 每个cleanup action有稳定action id和单调phase，Pending resume只继续该phase，不能重复commit/abort；
- normal transaction exit先commit；commit失败把commit error设为primary failure，再按policy尽力abort，abort
  failure不覆盖原commit error；
- ordinary throw在commit选择前等待abort，并保留原throw为primary；
- timeout/internal stop的可见terminal按reference立即固定，best-effort cleanup可以在bounded owner中继续，但
  不延迟已固定response，也不能把late result写回结束heap；
- cleanup自身调用child/host时继续使用同一 trampoline/actual-Pending协议。

### 13.3 Transaction state

Nested transaction当前不支持，source checker与verifier都拒绝。`db transaction`的atomicity只属于
DB driver transaction：commit发布DB write，abort只回滚未提交DB write。VM不为local slot、request heap或
`InOut` place建立root checkpoint/journal；在throw、commit failure或abort之前已执行的普通内存写入
继续可见。

Actor transaction遵守同一DB-only规则。Transaction body禁止直接或经callee写actor field，但该禁令是
compiler/verifier effect rule，不是内存rollback机制。不得为Actor重新引入snapshot overlay。

## 14. Actor shared heap

每个live Actor instance拥有一个 `ActorStateHeap` shared arena和stable field root slots。Actor method不是把
state clone到request heap：

1. method entry acquire `ActorSegmentLease`，校验actor identity、implementation build/identity、incarnation
   fence和arena epoch；
2. 同步段直接读写shared arena。已执行field/node mutation立即对后续获得lease的方法可见；
3. ordinary return只归还lease，不“commit”副本；throw/internal failure也不回滚已经执行的actor write；
4. 只有leaf真正返回Pending时才drop arena guard/lease；fiber保存
   `ActorContinuation { identity, implementationBuildId, fence, arenaEpoch, leaseState }`；
5. resume前重新acquire并重新校验。Actor已升级/逐出、fence或epoch不匹配时按Actor terminal contract失败，
   不能在stale snapshot上继续；
6. request-scoped stream/resource/callback capability在actor field写路径立即fail closed；
7. compaction只在无active/suspended continuation的quiescence执行，并bump arena epoch。

Aggregate value semantics不改变共享state可见性：直接actor field path write仍立即修改shared arena；只是把field
读取到普通local或传给普通参数时得到logical snapshot，COW防止local mutation暗中改actor。显式写回
`self.field = value`或direct writable field path才修改state。

跨owner service/callback child同步运行期间，caller Actor lease保持不释放；child实际Pending时才释放。这正是
trampoline不能把所有child call一律排队并伪造Pending的原因。

## 15. Boundary and recoverable values

### 15.1 Service/owner boundaries

Service/Actor/callback boundary总是按typed value plan materialize：

```text
caller owner + heap
  -> canonical boundary value/freeze
  -> provider owner + fresh heap or Actor arena
  -> execute child
  -> canonical result/error
  -> caller heap
```

Provider不接收caller frame、mutable root、`InOut` loan、request handle或Actor execution frame。同进程
`EnterChild`只省transport/native future，不省boundary语义。Source/destination temporary values遵守transient
root protocol。

### 15.2 Recoverable contract

Durable/recoverable codec只观察logical typed value graph和stable code/type/carrier identity，不保存：

- `ValueSlot` bytes、request/Actor/Const raw handle；
- unique/share/edit token、COW backing identity或GC generation；
- fiber/frame/pc/resume/pending/unwind state；
- `InOut` loan、transient root或live callback/resource capability。

Value semantics意味着recoverable codec可以把共享physical backing编码成等价acyclic value tree；physical alias
不要求恢复。Cycle仍按recoverable contract fail closed。`ConstRef`在encode时编码其logical value/code identity，
decode到当前owner的fresh value；不能把image-local const index当durable identity。

## 16. Execution safety, budget and attribution

### 16.1 Unbypassable hard fuel

Artifact semantic charge metadata不能单独承担安全性。VM dispatch每执行一条semantic instruction都更新受信
hard fuel counter；counter耗尽必须检查instruction limit、deadline和internal stop后再继续。Artifact不能关闭、
跳过或重置该counter。

Post-link verifier另证明每个CFG cycle经过`budget_checkpoint`，以保持当前loop/backedge semantic charging和
及时stop行为。两层同时存在：hard fuel防损坏artifact/validator bug无限pure jump；显式checkpoint保持语言
可观察budget单位。Decoded fusion/JIT必须按所含semantic op数量扣fuel，并在内部unbounded loop poll。

### 16.2 Semantic charging and profiling

Emitter为statement/expression/function entry/local call/tail hop/loop/generated chunk等当前语义点生成稳定charge
metadata。VM可批量提交unit，但quickening、micro-op数或machine instruction数不能改变语义unit。

Artifact分开保存：

```text
StatementEntry { pc, statementId/function attribution }
SourceMapEntry { pc range, InstructionSourceSite }
```

每次控制流进入statement entry恰好记录一次。Source map覆盖call、throw、effect、DB、timeout和generated
instruction。挂起/恢复不重复已进入statement，也不漏记resume后的新statement。

Frame保存exact call site；throw/effect error使用当前source site。Non-tail local frame按unwind生成stack trace；
tail replacement遵守bounded diagnostic contract。Cross-service/provider frame通过canonical error channel投影，
不能拼接Rust pointer/debug frame。

## 17. Performance contract

性能是本架构的一等完成目标。至少保持：

- sync local call/return在预留frame/value capacity内零host heap allocation、零Rust future；
- 同步完成的跨owner bytecode child不park、不递归native stack；
- slot load/store操作固定宽度slot；semantic copy的share transition为O(1) root操作；
- constant load不产生request allocation；
- dense field access是verified offset；
- unique Array push为amortized O(1)，unique Map put为expected O(1)；
- nested snapshot只沿首次write path COW，不在每次copy时deep traverse；
- builder不按元素复制完整collection；Map mutation不为canonical iteration维持tree；
- low-allocation request可以零GC cycle；
- Ready success/error不分配program continuation；
- Pending只创建一个pending owner并移动现有fiber/invocation chain；开销不按local call depth复制；
- eligible tail recursion active frame/value/diagnostic space为O(1)；
- allocation-heavy long request能回收unreachable intermediate value；
- Actor同步段零state clone/encode/commit overlay；actual Pending只做lease release/reacquire；
- quickening不扩大artifact ISA。

Release benchmark suite至少覆盖pure loop、deep sync/tail calls、dense record、unique Array/Map、nested COW、
JSON/DB materialization、string concat、local/remote/callback interface dispatch、synchronous cross-owner child、
Ready error、Pending park/resume、pending cleanup、Actor segment和long-request GC，并包含真实Agine chat smoke。

原始目标是相对树遍历async evaluator让sync interpreter hot path获得数量级改善（预期5–20x）；具体门槛必须在
implementation benchmark plan绑定workload、release profile、机器、统计口径和baseline commit。

## 18. Cross-document and completion contract

本架构只有在以下条件全部满足后才完成：

- `runtime-lazy-load-deployment.md`、`package-service-contract-deployment.md` 与runtime实现都以buildId
  `DeploymentExecutionImage`为执行单位，旧`RuntimeAssembly`已删除，可选`ReleaseBundle`只用于离线聚合；
- `../reference/syntax.md` 明确 local `var`、immutable `let`与top-level frozen `const`；
- `../reference/static-semantics.md`、`../reference/runtime.md`和Actor文档从mutable aggregate reference
  semantics收敛到value semantics、writable path、ordinary value argument与Package-only `InOut`；
- `../reference/std-surface.md`明确receiver mutation与static transformation API；
- Package/Service契约拒绝service/gateway/interface/callback `InOut`，Package Local ABI保留该mode和
  `maySuspend` summary；
- artifact schema由单一opcode/operand/stack-effect声明生成；
- pre-link structural validator不能被linker绕过，post-link semantic verifier对未知target/type fail closed；
- compiler-time const evaluator与ConstantHeap load/freeze protocol落地；
- service/Actor/interface/callback的EnterChild/Ready/Pending矩阵共享一个scheduler trampoline；
- restricted callback synthetic body/capture和三分支`call_interface`有真实source-to-runtime test；
- request heap、Actor heap、constant heap、resource table、VM stack和transient roots owner分离；
- sync、actual-Pending、resume error、timeout/stop、catch/rethrow、transaction commit-fail-abort、stream cleanup、
  Actor partial write、tail call、GC transient root和memory limit都有focused test；
- hard fuel与CFG-cycle checkpoint共同阻止无charge pure jump loop；
- source-to-artifact-to-deployment-load-to-runtime真实路径、GC pressure与Agine chat smoke通过；
- production tree evaluator、old artifact reader、assembly admission/generation、`call_suspend`、test-only
  evaluator和compatibility fallback全部删除。

Collection/value/const/`InOut`语义改变后，旧evaluator不能作为这些case的“语义等价”differential oracle。
迁移期只可对未改变语义的fixture做differential test；新语义必须由reference-derived golden test验证。

新增/修改artifact instruction必须同时更新canonical schema、emitter、structural validator、linker、semantic
verifier、decoded runtime、source/statement mapping和focused tests。Physical slot/collector/micro-op优化只要保持
本文不变量，可以不升级artifact ISA；opcode/operand/stack semantics变化必须升级ISA/schema version。

临时迁移顺序、feature branch、benchmark gate和逐组件删除计划属于`../implementation/`，不得写回本长期
architecture contract。
