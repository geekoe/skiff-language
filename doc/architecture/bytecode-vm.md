# Bytecode VM and Runtime Memory Architecture

本文是 Skiff bytecode VM、deployment execution image、运行时内存与挂起/恢复机制的目标内部架构事实源。
它面向 compiler、artifact、deployment loader/linker、runtime、host adapter 与 profiler 维护者，规定
relocatable ISA、两阶段验证、值布局、collection 物理表示、request-local GC、Actor shared heap、VM
fiber、跨 owner trampoline、异步 unwind 和执行归因之间的长期边界。

Skiff 尚未发布。本架构落地时整体升级 artifact/runtime 格式并删除树遍历 async evaluator；production
不保留旧 artifact reader、双解释器或按版本 fallback。

本文依赖的用户可见语义已在`../reference/`收敛：普通aggregate value semantics、
top-level frozen `const`、local `var` / immutable `final`，以及只允许Package Local ABI使用的
显式`InOut`。Collection bracket 的 strict read、indexed assignment、atomic loan 与公开错误同样
由`../reference/syntax.md`、`../reference/static-semantics.md`、`../reference/runtime.md`和
`../reference/std-surface.md`定义。本文只规定它们的VM物理实现与验证边界。

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
- PackageArtifact 可以保存 generic bytecode template；deployment linker 对 exact package closure 做有限、确定性的
  monomorphization。`LinkedBytecodeImage` 中所有 function/frame/type/shape 都已 concrete，不存在 runtime
  `TypeParam` substitution 或按调用现场临时编译。
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
  concreteSpecializations
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
4. **Deployment linker** 消费 exact package/deployment facts，计算有界 generic specialization closure，并把
   relocation 解析为 image-local target、type、shape、effect adapter、capability 和 const entry。
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

M3 hard-cut后的持久化 envelope 是 bytecode schema `skiff-bytecode-v7`、ISA
`skiff-bytecode-isa-v4` 与 bytecode identity generation v5（schema marker
`skiff-bytecode-artifact-v5`，identity prefix `skiff-bytecode-image-v5:sha256`）。v7 header 除
magic/schema/ISA/declared identity 外，必须携带并精确钉住以下六个 semantic authority：

- opcode contract：`opcodeTableFingerprint`，覆盖 numeric/semantic opcode identity、operand role、stack
  effect、允许的 relocation kind，以及default/frame-entry/per-opcode statement charging rule；
- native lifecycle registry：`nativeValueLifecycleRegistry` 的 exact registry id、version 与 fingerprint；
- value lifecycle policy：`valueLifecyclePolicy` 的 exact version 与 fingerprint；
- host effect registry：`hostEffectRegistry` 的 exact registry id、version 与 fingerprint；
- intrinsic registry：`intrinsicRegistry` 的 exact registry id、version 与 fingerprint；
- platform error projection registry：`platformErrorProjectionRegistry` 的 exact
  `PlatformErrorProjectionRegistryRef`：

  ```text
  {
    registryId: "skiff-platform-error-projections",
    registryVersion: 1,
    fingerprint: "sha256:<64 lowercase hex>",
  }
  ```

这些字段是必填 pin，不是可选的 provenance note。Structural admission 必须把每个 pin 与当前 reader 的
compile-time authority 做 exact equality；缺失、未知或任何 identity/fingerprint mismatch 都在 link 前拒绝，
即使当前 image 恰好没有引用对应 registry entry 也不能忽略。Validated view、compiler handoff、hydration、
linked candidate 与 verifier 必须成组保留这些 pin，后续层不得从 ambient registry 重建、替换或只比较
display id/version。

`platformErrorProjectionRegistry`只能来自compiler-owned checked-in generator输出的singleton或其validated typed
handoff；public emitter不得接受调用方任选的descriptor。Structural admission必须将它与Runtime binary的
generated singleton exact-match。PackageArtifact root还要保存同一exact descriptor；PackageArtifact、bytecode与
Runtime三方任一不一致都在执行前拒绝。

Registry内每个generated projection key逐字等于canonical public symbol、没有版本后缀，且registry中出现的
每个key恰有一个active entry、不得重复。Bytecode catch/failure plan只能引用该generated key与payload
builder；schema/codec/policy变化保持key并更换entry/whole-registry fingerprint，通过本节既有
artifact/runtime hard cut传播，不能让VM按key或payload shape猜另一fingerprint的codec。

v7 在 v6 的五个required authority基础上增加该第六pin。它没有改变opcode number、operand layout或
operand-stack semantics，因此ISA保持v4；required header、identity preimage与完整image改变，所以bytecode
identity升级到generation v5。同一wordcode若带不同authority pin、source-event placement或statement charge
contract，不是同一个executable artifact，并必须产生不同bytecode identity与上层build identity。

承载bytecode ref的PackageArtifact hard cut到`skiff-package-artifact-v15`，其root
`platformErrorProjectionRegistry`为必填字段。Package build identity projection同时加入该exact descriptor，
preimage marker为`skiff-package-artifact-build-identity-v13`，identity prefix为
`skiff-package-build-v14:sha256`。必填的package-owned statement manifest identity与registry descriptor都进入
build preimage，但不进入Package Local ABI preimage；Package Local ABI与ServiceProtocol identity generation
保持不变。

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
  origin: BytecodeFunctionOrigin
  typeParameters
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

### 3.3 Deployment-link monomorphization

泛型采用用户选择的方案 A：artifact 保留 relocatable template，exact deployment 在 link 时单态化。算法契约：

```text
SpecializationKey
  templateFunctionKey
  canonicalConcreteTypeArguments
  concreteReceiver/SelfInstantiation

roots = operation/gateway/Actor/package-direct/frozen-const behavior entries
worklist = roots in canonical identity order
while worklist not empty:
  intern SpecializationKey -> concrete function index
  substitute frame slots, value-transfer plans, type/shape refs,
             call relocations, return/exception plans and callback captures
  enqueue newly reachable concrete specializations in (call-site pc, key) order
```

- specialization key 的所有 type argument 必须 fully concrete。Artifact template/call relocation可引用词法
  `TypeParam`，但对某个concrete key代换后若仍残留`TypeParam`、unresolved associated fact或caller-local
  inference variable，该build立即link失败。
- 同一个 key 的 direct/mutual recursion 可以指向正在构造的 concrete function index，因此普通递归不会让
  worklist无限展开。若 polymorphic recursion 持续生成新 key，或specialization数量、单函数/总code words、
  type depth超出受信上限，整个buildId以稳定link error失败；不得lazy specialize、截断后fallback或把
  template交给runtime解释。
- canonical root/worklist顺序与intern规则保证相同validated输入生成相同linked overlay；并发load共享同一个
  per-buildId结果。
- specialization会重算 concrete `CallableEffectSummary`、frame layout、max operand depth、exception/callback
  plan与tail-call eligibility；不能把template summary不经代换直接当作proof。
- post-link verifier拒绝 `LinkedBytecodeImage` 中任何残留 `TypeParam`。VM frame不携带generic environment，
  `call_local`/`tail_call_local`只引用exact concrete function index。

Service operation和external ingress第一版仍不能是generic declaration；它们可以使用fully instantiated的
generic platform/user types。Package public generic callable可作为template发布，只在某个exact consumer/
deployment closure形成finite concrete specialization时可执行。

### 3.4 Relocation and dynamic targets

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

### 3.5 Initial instruction families

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

Stream
  stream_next
  emit_stream

Exception/region
  throw
  rethrow
  enter_region
  leave_region

Host effect
  invoke_host
```

`array_get`是source-visible`Array[index]`的strict read；越界时构造
`std.collection.ArrayIndexOutOfBoundsError { index, length }`。`map_get`只表示source-visible`Map[key]`
strict read；missing时构造`std.collection.MapKeyNotFoundError {}`。它不得实现`JsonObject[key]`，也不得
实现optional`Map.get(key) -> V?`；后者必须经过独立intrinsic/receiver-call semantic path。
`JsonObject[key]`将来必须由receiver-kind明确的`JsonObjectGet`/typed-segment producer构造
`std.collection.JsonObjectPropertyNotFoundError {}`。当前initial instruction family尚无该producer；补齐它是
M4 collection migration prerequisite，不属于M3 registry hard cut，本次不新增opcode或升级ISA。
`map_put_owned` 是 internal upsert mnemonic，可承接 source `Map.set` 或 terminal indexed assignment；
它不向 source surface 公开 `Map.put`。

OpcodeContract 必须保留以下失败分类，不能因为都在 VM 内部发生而共用一个 catchable
error path：

- source-visible strict collection bracket 使用上述 ordinary catchable request exception；
- `Trap(Assertion)` 的 false 结果、divide-by-zero 和产生非有限值的 arithmetic 是不可捕获
  terminal；当前不存在公开 `ArithmeticError`，也不得借 collection error 替代；
- `MapEntryAt` 是遍历等 runtime-internal canonical snapshot access，其 ordinal 越界是
  VM/generated terminal，不是 source bracket missing，不生成任何public collection error。

`copy_slot`、`dup`、container store 和普通 by-value argument preparation 按 linked `ValueTransferPlan`执行；
ordinary aggregate 才做 semantic share transition。它们不能只复制16 bytes后留下两个“唯一”edit token，
也不能复制move-only/affine resource。`move_slot`转移value并使source slot dead；verifier必须证明之后不再读取。
`stream_next`（`StreamNext`）消费一次性stream endpoint的下一项。`for item in stream` 的 lowering 有
item/end 两个 successor：item 路径 resume 后压 1 个 `T`，end 路径走独立 end resume PC、零结果并使用
显式 `ResumeOutcome::StreamEnd`。`emit_stream`只能出现在verified server-stream producer，并在buffer满/
consumer未就绪时形成真实backpressure Pending。

`tail_call_local` 是新 bytecode ISA 的显式操作。它只由 emitter 在 source eligibility 已确定且 relocation
为 exact-local kind 时产生，并由 post-link verifier 再证明 return plan 与 region eligibility。它有意取代
树遍历 evaluator 时代“禁止 artifact marker”的实现限制，不改变 reference 的 tail-position 语义。

不存在 transitive-function `call_suspend`，也不存在无法验证的 `invoke_unknown`。Restricted callback 有
`make_callback` 与 synthetic function；这不是开放 general first-class closure/yield。

### 3.6 Decoded micro-ops

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

- magic/schema/ISA version 已知，且 opcode contract、native lifecycle registry、value lifecycle policy、host
  effect registry、intrinsic registry与platform error projection registry六个required header pin都与reader的
  compile-time authority精确一致；
- artifact、function、word、table、string、constant graph、nesting depth 和单对象大小在配置上限内，所有
  count/offset arithmetic 无溢出；
- instruction word 边界完整，opcode operand 数正确；
- local pool/slot/relocation/table index 在界内，relocation declared kind 与使用 opcode 相容；
- jump/switch/handler/resume target 指向本函数 instruction header；
- exception/source/statement/capture table 结构有序、无重叠非法区间；statement rows只落在instruction header，
  same-PC `sequenceOrdinal`从0稠密，typed attribution occurrence无洞，且每个opcode-required event恰有一条；
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
- linked function、slot、type/shape、exception/callback/value-transfer plan中没有残留`TypeParam`，每个concrete
  specialization key唯一且所有call edge指向对应exact specialization；
- dynamic interface method slot、Local/Remote/Callback 三条 carrier path 共享同一 canonical signature；
- 每个 `HostEffectRef` 与 `IntrinsicRef` 都只能在 header 精确钉住的 registry 下解析，并精确匹配 target、
  binding/metadata 与 instantiated signature；symbol/name 相同或 registry id/version 相同不能替代 fingerprint
  equality；
- exception region 正确嵌套，handler stack height、catch slot、matcher 与 cleanup depth 合法；
- 每个 pending-capable site 有唯一 resume descriptor，resume result/error shape 正确；
- declared `NoPending` callable 不可到达 pending-capable instruction；
- `tail_call_local` 满足 exact target、return plan equivalence 与 cleanup eligibility；
- move、share、builder edit token、writable path 和 `InOut` loan 不产生 use-after-move 或同时可写 alias；
- 每个 bracket/index plan 的 concrete receiver kind、selector/result type、strict/intermediate/terminal
  policy 与 opcode 精确一致；`Array` 只接受 `integer` 且 terminal write 为 replace-only，
  `Map<K,V>` 只接受精确 `K`，`JsonObject` 只接受 `string`，terminal Map/JsonObject write
  为 upsert，intermediate 与 `InOut` terminal 则必须 exist；
- bracket read result 存在 exact linked snapshot lifecycle，indexed store 只有一个 atomic commit，`InOut`
  在所有 argument/selector/path check 后才原子取得整组 loan；任一 fallible segment 都有独立
  source site；
- 每个slot/parameter/result/container field的`ValueTransferPlan`完整；move-only/affine resource不会经过copy、dup、
  普通snapshot store或多consumer stream路径，所有overwrite/frame-pop/unwind edge都有exact drop；
- callback capture layout 与 synthetic body slot/signature/effect profile一致且不违反 escape policy；
- source/statement tables 覆盖所有 call、throw、effect 和 generated failure site；verifier 从 typed rows 与
  fingerprinted charge contract 重建 immutable statement schedule，不能把 linked/raw rows 当成已验证 schedule；
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

### 6.5 Value transfer and affine resources

每个linked type/slot/parameter/result/container position都有显式`ValueTransferPlan`，至少区分：

```text
SnapshotShare       # ordinary scalar/aggregate value；copy产生logical snapshot/COW share
MoveOnly            # identity-bearing value；只能move
AffineResource      # 最多一个live owner，离开作用域必须release
ExplicitCloneLease  # 只有声明了clone adapter的resource才能显式复制lease
```

`ResourceRef`不是ordinary aggregate，不能自动走share transition。`Stream<T>` endpoint是
`AffineResource`/one-shot consumer：赋值、参数准备和return默认move，`copy_slot`、`dup`、普通container store与
第二次迭代都由source checker/verifier拒绝。Socket/file/timer等是否move-only或可显式clone由各自native
contract决定，不能从slot kind猜测。

`move_slot`原子转移resource token并清空source。`drop`、slot overwrite、frame truncation、tail replacement、
normal return、throw/unwind与request stop都执行linked drop plan；ResourceTable以
`(resourceId, generation, owner)`做exact、幂等release，显式close后后续drop为no-op。含resource的特权native
record必须有递归drop/transfer plan；普通用户record/schema仍不能藏resource。GC只追踪resource payload显式
登记的managed roots，不承担close时机。

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
- unique Map upsert 是 expected O(1) hash update；
- shared node first write 做 path COW；
- literal、decode、DB result、map/filter、`Array.concat`、`Map.merge` 和 compiler-proven loop accumulator 使用
  transient builder；freeze 后才作为普通 value 流动；
- `Array.concat` 先计算总长度并一次/分段分配；左 operand 已死且 backing unique 时可复用；
- persistent vector/HAMT 只在真实 benchmark 证明频繁大 snapshot 后写入由全量 COW 主导时引入 adaptive form。

Source API 不用 `appending`/`setting` 等细微词形区分 purity。Receiver
`Array.push` / `Array.set` / `Array.pop` 与 `Map.set` / `Map.delete` 只对verified writable path做
mutation；该path可以root于local `var`、有效`InOut` loan，或Actor method中当前规则允许写入的
`self` field。Pure transform 使用一眼可见的 static `Array.concat` 和 `Map.merge`。

#### Bracket/index plans and atomic mutation

Source analysis/lowering 必须为每个 index segment 产生 symbolic typed fact；deployment link 把它解析成
concrete image-local plan，至少包含：

```text
ResolvedIndexSegment
  receiverKind: Array | Map | JsonObject
  selectorType
  resultType
  policy: StrictRead | IntermediateMustExist | TerminalReplace | TerminalUpsert | LoanMustExist
  resultTransferPlan
  sourceSite
```

VM 只消费 semantic verifier 证明过的 concrete plan，不根据 raw runtime tag 尝试 string/record/
unsnarrowed `Json` fallback。Strict read 先求值 receiver，再求值 selector，各一次；返回值按
`resultTransferPlan`形成logical snapshot，不暴露writable handle。ArrayGet及array writable segment构造
`std.collection.ArrayIndexOutOfBoundsError`，MapGet及map writable segment构造
`std.collection.MapKeyNotFoundError`，future JsonObjectGet及typed JsonObject segment构造
`std.collection.JsonObjectPropertyNotFoundError`，都使用当前segment source site。Map key与JsonObject
property不得进入payload、message、trace或telemetry；错误类型也不携带`operation`、`container`或同义字段。

Indexed assignment 先只求值并缓存 writable root 与从外到内的 selector，同时解析全部
intermediate path；随后只求值一次 RHS，再按 linked COW/transfer plan 执行一次 atomic
store。Array terminal 是在界内 replace；Map/JsonObject terminal 是 upsert；所有 intermediate 都是
must-exist。Store preparation 失败不得留下部分 path mutation。不能把该语义降成一串可观测的
per-segment load + store。

当前 initial instruction list 的 `set_writable_path` 只表达 dense field store，
`array_push_owned`/`map_put_owned` 也没有表达多 segment transactional path。因此本节是
OpcodeContract 的强制输入，不声称现有 opcode 集已完成该编码；contract landed，
implementation pending。

### 8.4 Dense records and canonical Map order

Compiler-known record 使用 `ShapeId + dense ValueSlot fields`；field opcode 带 verified offset，不在热路径查询
字符串 map。Dynamic JSON object/Map 使用独立 hash representation。

Map 的用户语义已经要求 canonical key order：`keys()` 与 `for` iteration 在操作开始时生成 snapshot，并按
canonical string payload 的 UTF-8 bytes 升序排列。Mutation 可以继续使用 hash table；只在 snapshot/encode
边界排序，不能把 ordered tree 强加给所有 `set`/upsert。

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
```

进入 frame时按 verified max depth一次扩展 contiguous segment；普通 instruction不触发 stack realloc。Return
截断 segment并移入 caller destination。Non-tail recursion不增长 native stack，但仍受 frame/memory/fuel限制。

### 10.2 Local and tail call

`call_local` 按源码顺序求值参数，以 move/share准备 slots，push callee frame，然后在同一 dispatch loop继续。
不检查 callee transitive `maySuspend`，不创建 future；深处 actual Pending时整个 fiber一起停放。

`tail_call_local` 在 caller frame live 时求值参数一次，验证共同 return plan/concrete Self specialization/
region eligibility后
替换当前 frame segment。每 hop 保留 call/function-entry charge与hard fuel；eliminated edge不增长诊断栈。

### 10.3 `InOut` is Package Local only

目标态 `InOut` 是显式 caller-writable、write-through loan，不是 service wire mode：

- 只允许 exact local/package-direct callable signature；它进入 Package Local ABI identity；
- actual argument必须是 writable `var` access path；compiler/verifier证明调用期间 exclusive；
- 所有 argument 按源码顺序各求值一次；每个 `InOut` root/index selector 也只求值一次，
  selector 按 path 从外到内缓存，期间不提前取得部分 loan；
- 全部 `InOut` path 的 intermediate 与 terminal segment 都必须 exist。只有全部 argument/
  selector/path check 成功后才原子取得整组 loan；任一失败都无部分 loan、不进入 callee；
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
  EnterAdapter(AdapterInvocation)
  EmitStream(StreamItem)
  Park(PendingOperation)
```

Host adapter启动结果为：

```text
EffectStart
  Ready(Result<ValueSlot, VmError>)
  EnterAdapter(AdapterInvocation)
  Pending(PendingOperation)
```

`Ready` 明确包含同步失败；error在当前 source site注入VM，不需先制造future/Pending。只有可信 host adapter
可以返回 Pending。

Service/Actor/callback dispatch adapter 可以进一步返回：

```text
BoundaryStart
  Ready(Result<BoundaryValue, VmError>)
  EnterChild(ChildInvocation)
  OpenStreamChild(StreamInvocation)
  Pending(PendingOperation)      # remote transport, actor acquisition, etc.
```

`VmFiber`与resumable `NativeAdapterFrame`都是scheduler unit；两者只能通过上述control result交还执行权，
不能从adapter递归调用VM，也不能为方便把同步callback/stream步骤伪造成Pending。

### 11.2 Enter child without native recursion

`EnterChild` 使 parent fiber进入 `BlockedOnChild`，scheduler把 child fiber/owner/heap压入 invocation stack并在
同一个 loop立即执行。它不把 parent放进 host waiting table，不释放 Actor lease，也不计作 suspension。

Child 同步 `Complete` 时，scheduler按 boundary plan materialize result/error到parent heap，弹出 child并继续
parent。整个过程可以跨任意层 owner，但 native stack保持扁平。

Child 遇到真实 host Pending时，scheduler保存 parent/child chain并 park leaf。调用链上持有的 Actor segment
lease在此时按各自规则释放；普通 request heap仍由 rooted fibers拥有。Wake后从leaf恢复，逐层同步完成并返回。

### 11.3 Resumable native adapters

接收restricted callback的native API（例如`Array.map/filter`类adapter）不能在Rust栈上直接poll callback
bytecode。`EnterAdapter`安装：

```text
NativeAdapterFrame
  adapterIndex
  stateId + bounded adapterState
  rootedInputs/partialOutput
  callbackClosureRefs
  callerDestination + resultPlan
  sourceSite

AdapterControl
  Continue
  EnterChild(CallbackInvocation)
  Complete(Result<ValueSlot, VmError>)
  Park(PendingOperation)
```

Adapter每次需要调用callback时返回`EnterChild`；child结果由scheduler写回adapter resume slot，adapter随后可继续
迭代、再次调用或完成。Adapter state只保存stable handles/owned host bytes并计入request budget；callback
throw在原callback/call site进入同一unwind。只有adapter自己的host operation实际等待才`Park`。这使大collection
callback、callback内local/service call及真实Pending都保持flat native stack。

### 11.4 Stream producer and consumer

`OpenStreamChild`为verified server-stream invocation建立一个`StreamSupervisor`、consumer endpoint与provider
producer fiber。Service call在provider admission和boundary plans验证完成后，先把affine consumer handle
materialize给caller，再poll producer body；不能先同步跑producer直到buffer满才让caller取得handle。Gateway
root则把同一supervisor接到已有external response sink。Producer始终pin exact provider build、request
frame/deadline/trace/call stack与provider heap；它不是detached coroutine，也不允许普通local function任意spawn
一个可逃逸stream。

Producer执行`emit_stream`时返回`EmitStream`。Supervisor只在waiting consumer或bounded buffer有容量时同步接收；
否则把producer停在真实backpressure `PendingOperation`。`stream_next`（`StreamNext`）的 item/end 是两个独立
resume contract：item 路径 resume 后压 1 个 `T` 并按 boundary plan materialize，不能共享provider heap
handle；自然 end 走独立 end resume PC、零结果，使用显式 `ResumeOutcome::StreamEnd` 并跳到 continuation；
error映射为ordinary throw。Producer body 的 natural end `return` 栈 arity 为 0：它只生成一次end，不把
`Stream<T>` 作为返回值 materialize，`Stream<T>` 仅作为调用入口/边界的显式 producer authority。
Producer normal exit生成一次end，throw/timeout生成一次error；consumer break/drop/ancestor stop关闭affine
endpoint并向producer发送best-effort stop，晚到item只做内部清理。同一endpoint不得被两个fiber/lane消费。

Native external source stream也登记在同一ResourceTable/Supervisor contract，但其producer由host adapter拥有。
Package-direct stream wrapper复用当前request已有的stream registry；普通Skiff package/local body仍不能创建新的
可逃逸`Stream<T>`。这些限制由source checker、effect metadata与post-link verifier共同证明。

### 11.5 Park and resume

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

Start、completion、deadline、internal stop与cancel可能发生在不同线程；`Park`不能依赖“先登记waiter、后端才
可能完成”的时序假设。每个pending site使用一个受信completion cell：

```text
PendingCellState
  Open(rootEscrow)
  Waiting(waiter, pendingOwner)
  Settled(outcome, rootEscrow)   # completion早于waiter publication
  Claimed                       # outcome已恰好一次交给scheduler
```

Adapter在把completion handle交给host前先建立root escrow。Host completion从`Open -> Settled`，或从
`Waiting -> Claimed`并enqueue一次；scheduler发布waiting owner时从`Open -> Waiting`，或观察`Settled`后
`-> Claimed`并立即enqueue。Deadline/stop/cancel通过同一个terminal arbiter竞争settlement；只有一个winner，
duplicate/late completion只命中bounded tombstone并释放自身host payload，不能二次wake、二次drop或覆盖terminal。

Roots、pending budget reservation、callback/resource lease和boundary buffers在Ready、Pending publication与
pre-completed outcome三条路径上必须恰好转移一次。Actor segment lease只能在pending owner及其roots已经成功
发布后释放；若publication失败，仍由当前fiber同步unwind。该原子握手必须有completion-before-register、
register-before-completion、deadline/cancel四向竞态和duplicate completion测试。

Wake result为 `Value`、`Throw` 或 internal terminal。成功按plan导入并回到runnable；失败在原effect site注入，
走同一个unwind state machine。Budget、profiling、concrete specialization、diagnostic prefix与transaction region不重置。

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
  -> same-runtime EnterChild back to capability owner
     or RemoteBoundary transport Pending when that transport exists
```

Verifier证明method slot和三条分支的参数/返回shape都等于interface requirement；image construction验证每个
local method table exact target和remote operation table。Unknown carrier/tag/slot fail closed。Local branch
即使静态 `maySuspend` 保守为true，也不会因此yield；remote/callback分支只有actual wait才park。

当前 Runtime 的 service callback capability只覆盖同runtime owner lookup/context switch；目标bytecode VM
必须把这条路径收敛为`EnterChild`。Router wire没有反向callback frame。
因此跨runtime callback必须在deployment admission fail closed，不能把上图的`transport Pending`读成现状能力。
未来RemoteBoundary需要由service/runtime transport文档先定义owner route、request/cancel/response、认证、
lifetime与backpressure，VM只消费其已验证adapter。Agine的package-local `any I` adapter内部调用AIHub是Local
branch + 普通正向service call，不覆盖这条transport门禁。

### 12.2 Restricted callback bodies

目标reference允许IIFE、白名单callback argument和具名callback reference，因此bytecode ISA不能把closure
全部推到未来。当前source parser/AST/lowering尚未形成完整`FnExpr`链路；production完成必须补齐真实
source-to-bytecode路径，不能只用手工artifact fixture证明：

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
  candidateOutcome: Option<ReturnValue | ThrowValue | ChildFailure>
  selectedOutcome: Option<ReturnValue | ThrowValue | VmFailure | PlatformTerminal>
  responsePublication: NotEligible | Unpublished | Published
  cleanupFailure: Option<VmError>
  regionCursor
  cleanupPhase: Enter | Starting(actionId) | Waiting(actionId, pendingId)
                | Completed(actionId) | Done
```

规则：

- normal return/ordinary throw在必须完成的commit/abort/close前只进入`candidateOutcome`；cleanup
  改变最终语义时不能先向caller发布success。`selectedOutcome`一旦选择不被resume/re-poll/
  late cleanup覆盖；
  只有逃出request/gateway root的outcome才进入response publication，且只能`Unpublished -> Published`一次；
- 每个cleanup action有稳定action id和单调phase，Pending resume只继续该phase，不能重复commit/abort；
- normal transaction exit先commit；commit成功后才选择return。commit失败以commit error选择failure，
  再按policy尽力abort；abort failure只进`cleanupFailure`/telemetry，不覆盖commit error；
- ordinary throw在发布前等待语义要求的abort，并保留原throw；abort失败不把同一request变成第二个terminal；
- 词法timeout立即选择当前scope的`std.error.TimeoutError { timeoutMs }`，仍可由scope之外的Skiff
  `catch`处理；request/root/inherited deadline只选择response/request terminal，不向dying frame注入error。
  HTTP或Actor primitive timeout只有在对应caller continuation仍active且其deadline先到时，才选择各自typed
  error；internal stop直接选择不可捕获的platform terminal。这些winner的
  best-effort cleanup可以在bounded owner中继续，但不延迟已固定语义结果，也不能把late result写回
  已结束的scope/request heap；
- cleanup自身调用child/host时继续使用同一 trampoline/actual-Pending协议。

Timeout/internal stop选择outcome后若cleanup仍可能Pending，scheduler只可把**driver-owned** transaction/resource/session
句柄和owned host bytes转移给`CleanupOwner`。该owner有独立有限budget/deadline与pending
handshake；它不能保留Skiff frame、request/Actor heap handle、callback closure、用户bytecode继续执行权，或
发起新的用户语义effect/response写回。已经开始的DB commit或外部effect仍可能在后台完成或成为
unknown outcome；cleanup owner只做收尾/观测，不得把结果重新注入Skiff状态。Request heap只有在所有roots已
释放或原子转移后才能销毁。局部timeout被caught时外层fiber继续拥有原request heap，只有已
转移的driver handle在cleanup owner中收尾；不得把这误实现为销毁整个request heap。普通return/throw
路径仍由原request owner完成语义必要cleanup；不能为了提早响应把commit/abort随意detach。

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

1. 每个identity的live incarnation钉住创建它的exact deployment `buildId`、`DeploymentExecutionImage`与
   ActorStateHeap；不同identity可以同时运行不同build。method entry acquire `ActorSegmentLease`并校验请求build
   与owner build精确相等、actor identity/implementation identity、incarnation fence和arena epoch；
2. 同步段直接读写shared arena。已执行field/node mutation立即对后续获得lease的方法可见；
3. ordinary return只归还lease，不“commit”副本；throw/internal failure也不回滚已经执行的actor write；
4. 只有leaf真正返回Pending时才drop arena guard/lease；fiber保存
   `ActorContinuation { identity, implementationBuildId, fence, arenaEpoch, leaseState }`；
5. resume前重新acquire并重新校验。Actor已逐出/owner失效、fence或epoch不匹配时按Actor terminal contract失败，
   不能在stale snapshot上继续；
6. request-scoped stream/resource/callback capability在actor field写路径立即fail closed；
7. compaction只在无active/suspended continuation的quiescence执行，并bump arena epoch。

不同build的get/method/task在live incarnation存在时直接`ActorVersionRejectedError`：VM/Router不在旧heap上跑
新image代码，不触发upgrade/retirement，也不刷新旧owner的idle时钟。普通idle TTL、disconnect或shutdown
销毁实例后，下一个成功claimant以自己的exact build创建，允许回退；不存在Actor release pointer或
newest/superseded集合。第一版即使`ActorAbiIdentity`相同也不跨build共享heap。未来若以ABI兼容优化，必须先
定义所有image-local type/shape/const/behavior引用的重绑定proof，不能只比较一个hash便混用两个image。

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

Artifact semantic charge metadata不能单独承担安全性。VM dispatch对每条semantic instruction从受信raw-op
quantum扣1；quantum为0时无条件poll deadline/internal stop，并从execution policy拥有的finite instruction
limit扣除已执行数量。总limit耗尽立即固定
`std.error.InstructionLimitExceededError { instructionCount, limit }`并终止当前frame；该frame没有剩余budget
执行catch。Failure到达service request root后可以固定为typed carrier，供仍active remote caller按admission
捕获；本地dying frame不可恢复执行。继续执行时只能由VM按固定、非零、受信上限补充下一个quantum。
Artifact不能设置、关闭、跳过或重置quantum/总limit，也不能用metadata扩大预算。

Post-link verifier另证明每个CFG cycle经过`budget_checkpoint`，以保持语言级loop/backedge semantic charging与
source attribution；checkpoint本身有固定非零raw-op成本，但不是唯一interrupt poll。两层同时存在：raw hard
fuel即使面对损坏artifact/validator bug也界定两次stop poll之间的最大工作量，semantic checkpoint保持既有
可观察budget单位。Decoded fusion/JIT必须按所含semantic op数量扣raw fuel；任何runtime micro-op内部循环也按
固定quantum poll，不能一次调用吞掉无界工作。

### 16.2 Semantic charging and profiling

Emitter为statement/expression/local call/tail hop/loop/generated chunk等当前语义点生成稳定source-event
placement；function entry由canonical frame-entry contract拥有。Persisted row只声明typed source-event identity、
placement与site，不能自报`chargeKind`；默认charge、function-entry rule与opcode reclassification全部由
fingerprinted canonical opcode contract拥有。数量与合法pc由
schema规则决定，post-link verifier从CFG/opcode和admitted typed rows重算并拒绝缺失、重复或伪造entry。VM可
批量提交unit，但quickening、micro-op数或machine instruction数不能改变语义unit，也不能影响raw hard fuel。

Artifact分开保存：

```text
StatementEntry {
  pc
  sequenceOrdinal
  attributionId:
    Statement { statementIndex, occurrenceOrdinal }
    | Expression { expressionIndex, occurrenceOrdinal }
    | Generated { ordinal }
  site: InstructionSourceSite
}
SourceMapEntry { pc range, InstructionSourceSite }
```

`StatementEntry`按`pc`非降序排列；同一pc可以有多个source event，`sequenceOrdinal`必须从0开始连续、无洞，
并精确给出该pc的执行次序。function-local attribution id必须唯一；同一statement/expression index的
`occurrenceOrdinal`以及generated ordinal分别稠密。Generated id只能配synthetic site；statement/expression
可以保留source或synthetic site。legacy `statementId`与row-owned `chargeKind`都不在v6 wire中。

默认映射是Statement→`Statement`、Expression→`Expression`、Generated→`GeneratedChunk`。某opcode若声明
`RequiredEvent { attributionClass, chargeKind }`，该pc必须恰有一个对应class的row；verifier把该row重分类为
opcode charge（例如local call、tail hop或loop check），不得额外合成第二个row或双重计费。Function entry则由
独立的frame-entry contract在每次frame invocation精确生成一次`FunctionEntry`，从不占用statement row。
默认映射、frame-entry rule以及每个opcode的statement rule全部参与`opcodeTableFingerprint`，不能由VM的
ambient switch或raw row字段替代。

PackageArtifact只持久化一个`bytecodeStatementManifestIdentity` pin；BytecodeArtifact、deployment与VM不复制
第二个可漂移pin。manifest preimage覆盖schema marker、exact package id，以及按`BytecodeFunctionOrigin`严格
排序的全部函数；零event函数也必须保留origin，每个函数的完整entry placement（含pc、sequence ordinal、
attribution id与site）全部参与identity。无bytecode的package必须声明该package id下的canonical empty
manifest。compiler publication必须把bytecode ref与manifest pin成对附加并重算Package build identity；loader
必须从已admit的完整bytecode function set重算manifest并与Package pin exact-match，不能信任compiler receipt、
function key子集或“只有非空函数”的投影。

Post-link verifier在mint seal前构造immutable verified schedule：row pc解析成linked instruction index，按
same-PC sequence保序，应用上述default/reclassification与rowless function-entry contract，并重新证明opcode所需
event的exact coverage。VM只消费这个schedule；`LinkedStatementEntry`或artifact raw rows即使已被exact-copy也仍是
untrusted metadata，不能由VM直接扫描计费。在schedule proof未实现或不完整时，verification必须返回
`ProofUnavailable`并使VM entry不可达，不能把raw rows透传成临时执行路径。

每次控制流进入verified statement schedule entry恰好记录一次。Source map覆盖call、throw、effect、DB、timeout、
每个可失败bracket/index path segment 和 generated instruction。一个 strict collection failure 使用它自己的
segment source site，`Array.set` 越界使用 receiver call site；`rethrow` 保留原 exception envelope 与原source，
不把rethrow instruction改成新throw source。挂起/恢复不重复已进入statement，也不漏记resume后的新statement。

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
- `../reference/syntax.md` 明确 local `var`、immutable `final`与top-level frozen `const`；
- `../reference/static-semantics.md`、`../reference/runtime.md`和Actor文档从mutable aggregate reference
  semantics收敛到value semantics、writable path、ordinary value argument与Package-only `InOut`；
- `../reference/std-surface.md`明确receiver mutation与static transformation API；
- collection bracket 的 typed plan、strict catchable error、linked snapshot lifecycle、indexed atomic store 与
  all-path-existing atomic `InOut` loan 已从 source facts 贯通到 OpcodeContract/emitter/verifier/VM；
- Package/Service契约拒绝service/gateway/interface/callback `InOut`，Package Local ABI保留该mode和
  `maySuspend` summary；
- artifact schema由单一opcode/operand/stack-effect声明生成；
- typed statement rows、package-owned manifest pin、loader recomputation与verifier-produced immutable schedule形成
  exact chain；same-PC sequence稠密，零event函数不从manifest消失，FunctionEntry无row，VM不读取raw rows计费；
- generic Package template在deployment link形成finite deterministic concrete specialization closure，linked image
  无`TypeParam`或runtime generic environment；polymorphic recursion/上限超出稳定fail closed；
- pre-link structural validator不能被linker绕过，post-link semantic verifier对未知target/type fail closed；
- compiler-time const evaluator与ConstantHeap load/freeze protocol落地；
- service/Actor/interface/callback的EnterChild/Ready/Pending矩阵、resumable native adapter和stream producer/
  consumer共享一个scheduler trampoline；adapter不得递归poll VM，`emit_stream`具有真实backpressure；
- restricted callback synthetic body/capture和三分支`call_interface`有真实source-to-runtime test；同runtime
  callback capability执行与cross-runtime placement fail-closed分别验收，Agine package-local callback +
  forward AIHub call不算remote callback transport证据；
- request heap、Actor heap、constant heap、resource table、VM stack和transient roots owner分离；
- move-only/affine ResourceRef（特别是one-shot Stream）在copy/dup/store、frame pop、tail replacement与unwind上
  通过linked transfer/drop plan验证，release exact且幂等；
- sync、actual-Pending、resume error、timeout/stop、catch/rethrow、transaction commit-fail-abort、stream cleanup、
  Actor partial write、tail call、GC transient root和memory limit都有focused test；
- Pending completion-before-register、register-before-completion、deadline/cancel/stop与duplicate completion竞态证明
  单winner、单wake、单root transfer；selected outcome/response publication唯一，bounded CleanupOwner
  不保留结束heap或产生late write；
- Actor owner fence包含exact build；不同identity可异版本并存、同identity mismatch只拒绝且不刷新idle，idle销毁
  后任意build可重新claim；当前registry的implementation pin必须从no-live-owner路径移除；当前owner
  lease/idle TTL同为30s且sweep先expire lease的默认路径必须修正，不能在未向Runtime发送/确认
  discard时只清Router fence并把残留instance当成已销毁；idle/disconnect后的新owner还必须
  推进incarnation epoch，不能让旧ref/continuation命中新instance；
- 受信raw hard fuel quantum/finite instruction limit与CFG-cycle checkpoint共同阻止无charge pure jump loop；
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
