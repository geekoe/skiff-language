# P5-F180A：Actor Executor Gap Audit Result

状态：Completed

## 直接父任务

- `P5-F179-actor-registry-surface-and-control-result.md`

## 总结

当前完成的是Actor声明、注册操作、bootstrap保存、内部句柄及部分epoch/lease围栏；真正的Actor方法执行链
尚未建立。

最早的生产断点不在Router，而在compiler lowering：

```text
hub.submitOp(op)
  -> source typing识别Actor方法
  -> lowering错误投影为普通LocalExecutable/ExternalServiceSymbol
  -> linker解析成普通ExecutableAddr
  -> caller Runtime直接执行impl
```

因此现状会绕过logical identity、epoch、Router owner定位、bootstrap激活和单实例executor。不能在此
基础上只补Router method frame；必须先建立Actor method call的canonical compiler/artifact事实。

## 全链现状

### 1. Source typing：只有这一段具备Actor专用语义

已有production实现：

- `compiler/source/src/expression_type_model.rs::actor_receiver_call_type`
  从receiver的Actor类型解析方法签名，禁止显式方法类型参数，校验去掉`self`后的参数并返回精确类型。
- `compiler/source/src/type_resolution_model.rs::actor_method_signature`
  要求receiver对应`SourceTypeKind::Actor`，再从本地impl方法索引读取签名。

不足：

- 当前Actor source tests只覆盖四个registry intrinsic、错误id/bootstrap和禁止普通构造，没有
  `hub.submitOp(...)`方法调用正负例。
- `compiler/source/src/resolved_call_targets.rs`没有Actor method target；
  `resolved_call_targets/builder.rs`把Actor impl与普通impl统一记录成`LocalCallTarget::ImplMethod`。
  effect、suspend和call graph因此没有Actor调用边界事实。

分类：**部分完整的production实现**。

### 2. Lowering、File IR与linker：方法调用走错普通直接调用路径

production证据：

- `compiler/lowering/src/function_lowering.rs::lower_receiver_call_target`没有Actor分支。
- 同文件`lower_static_impl_receiver_call_target`把本地receiver变成
  `CallTargetIr::LocalExecutable`，把`ServiceSymbol` receiver变成`ExternalServiceSymbol`。
- `artifact-model/src/executable.rs::CallTargetIr`没有Actor method variant。
- `runtime/linker/src/linker/file_conversion.rs::linked_call_target`没有Actor method转换。
- `runtime/linker/src/assembly_execution/code_linker.rs::link_call_target`最终把普通local target解析为
  `ExecutableAddr`。

F179A已经让`ActorDeclarationIr`进入File IR和linked program，但
`compiler/lowering/src/source_file_lowering.rs::lower_actor_declarations`仍固定写入
`public_methods: Vec::new()`。Artifact model的`ActorPublicMethodIr`、linked declaration的方法carrier
和linker类型链接代码虽已存在，却没有真实compiler producer。现有Actor linker fixture只手工构造
registry call，未覆盖源码Actor方法。

分类：

- Actor declaration/method carrier：**只有DTO和fixture骨架**。
- Actor method canonical call及linking：**完全缺失**。

### 3. Registry、bootstrap与内部句柄：注册路径完整，实例激活缺失

已有完整路径：

- `runtime/native/src/dispatch/actor.rs`按linked declaration验证的type plan编码Actor id和bootstrap，
  以canonical JSON bytes发出四个control操作。
- `runtime/model/src/value.rs::ActorRef`保存service、Actor type、id type、canonical id bytes/hash和epoch。
- `router/src/actor/identity.ts::actorLogicalKey`统一使用
  `serviceId + actorTypeIdentity + actorIdTypeIdentity + actorIdHash`。
- `router/src/actor/inMemoryRegistryStore.ts::getOrCreate/replace/find/remove`共享同一entry与epoch。
- registry entry保存Actor ABI、implementation、bootstrap encoding/bytes以及owner lease字段。

完全缺失：

- 没有production代码把registry中的`encodedBootstrapBytes`发送到owner Runtime并按
  `LinkedActorDeclaration.fields`物化成实例字段。
- Runtime没有Actor instance field arena/store；bootstrap目前止于Router内存。
- 当前`actorImplementationIdentity`来自
  `RuntimeNativeActorCapabilityContext::actor_implementation_identity`的整个request build id，不是架构
  要求的Actor可达方法IR及依赖的独立implementation identity。

分类：

- 四个registry操作：**已有完整路径**。
- bootstrap到实例字段：**完全缺失**。

### 4. Router execution与owner围栏：有store骨架，没有生产调用者

已有骨架：

- `router/src/actor/registryStore.ts`定义owner lease、execution ledger、entry epoch和
  `accepted/dispatching/running/finishing`状态。
- `inMemoryRegistryStore.ts::acceptActorExecution`检查entry present、expected epoch和owner lease；
  finish再次检查logical identity、epoch与lease。
- `remove`推进epoch，并在active execution清零后完成删除。

但全仓production调用表明：

- `acquireOwnerLease`、`acceptExecution`、`finishExecution`、`activeExecutionsForRuntime`、
  `evictIdle`只存在于store/manager本身，没有method dispatcher调用。
- `router/src/router/actorSpawnRuntimeControl.ts`只处理四个registry操作和spawn，没有Actor method frame。
- owner lease acquisition不检查现有未过期lease，可直接覆盖owner；在method dispatcher落地前必须修正。
- execution状态实际只写`accepted`和`finishing`，没有dispatching/running状态推进。

分类：**只有DTO/store fixture骨架**。

### 5. Runtime executor、字段隔离与suspension

以下目标态能力均无production owner：

- 每logical identity + epoch的Actor instance store；
- 单实例单线程执行权与mailbox；
- 不同实例并行；
- Actor `self`字段读取/写入专用路径；
- suspension point释放实例执行权；
- continuation恢复前epoch、implementation和incarnation检查；
- 显式yield、连续执行预算和Actor watchdog；
- active coroutine计数和安全点退出。

当前普通eval字段访问只操作request heap/object。Compiler虽禁止外部字段访问，但Runtime没有owner
executor field frame，因而尚不能证明字段只由owner Actor方法访问。

分类：**完全缺失**。

### 6. Upgrade、idle与crash

已有事实：

- entry保存epoch、ABI、implementation和owner lease expiry。
- `replace/remove`会推进epoch。
- Router registry默认in-memory，Router重启丢entry，符合目标文档。

缺口：

- entry状态只有`present/removing/removed`，没有`activating/live/upgrading`与目标implementation。
- `getOrCreate`命中entry时按其定义保留首次bootstrap，但也不比较incoming implementation；未来method
  admission必须单独校验。
- 不同implementation调用不会关闭admission、drain旧方法或选择目标Runtime。
- 没有`ActorUpgradingError`、`ActorVersionRejectedError`、
  `ActorIncarnationReplacedError`的wire/runtime映射。
- `evictIdleActor`只是手工primitive：只检查active count，既不读取TTL也没有sweeper。
- owner lease expiry没有续租或回收逻辑。
- Runtime断连只从runtime registry移除路由，没有通知ActorManager、失败active calls或释放Actor owner。

分类：**除epoch字段与手工primitive外完全缺失**。

## 需要的共享检查点

### CP1：Actor method ABI、implementation identity与canonical call

Owner：

- `artifact-model` / `artifact-identity`
- `compiler/source`
- `compiler/lowering`
- `runtime/linked-program`
- `runtime/linker`

必须冻结：

- Actor method稳定identity及参数、返回、`maySuspend`的canonical表示；
- Actor ABI identity覆盖id、字段、公开方法和runtime ABI；
- Actor implementation identity覆盖规范化方法IR及可达依赖，不能继续使用整个request build id；
- `ResolvedCallTarget::ActorMethod`、`CallTargetIr::ActorMethod`及linked dispatch plan；
- call只引用Actor declaration owner、Actor ABI/implementation和method identity，不复制声明或方法表；
- Actor仍不得伪造普通`TypeAddr`、record descriptor或type-table entry。

### CP2：Actor method invocation wire与错误合同

Owner：

- `runtime/capability-context`
- `runtime/transport`
- `runtime/host`
- `router/src/protocol`

Frame至少需要精确Actor logical key/ref、expected epoch、Actor ABI、requested implementation、method
identity、参数payload、deadline/cancellation correlation，以及typed return/error。

错误合同必须落实目标文档中的upgrading、version rejected和incarnation replaced；不得用普通service
request fallback或在request.start中恢复已退休的`actorCall`metadata。

### CP3：Router原子admission与owner状态机

Owner：

- `router/src/actor/registryStore.ts`
- `router/src/actor/inMemoryRegistryStore.ts`
- `router/src/actor/manager.ts`
- `router/src/router/actorSpawnRuntimeControl.ts`或拆出的Actor method dispatcher

必须先修复未过期owner lease可被覆盖的问题，再实现：

- exact epoch/ABI/implementation admission；
- inactive/activating/live/upgrading状态；
- 同implementation复用owner；
- 不同implementation关闭新admission并启动drain；
- execution ledger真实状态推进。

CP2冻结后，CP3可与Runtime实例存储并行。

## 实施DAG

```text
F180-B  CP1 shared Actor method/identity checkpoint
  ├─ F180-C  compiler真实method producer与源码全链fixture
  └─ F180-D  CP2 method invocation wire/error contract
                ├─ F180-E  Router atomic admission/owner/upgrade state machine
                └─ F180-F  Runtime ActorInstanceStore与bootstrap materialization
                              └─ F180-G  单实例scheduler + eval self字段访问
                                            └─ F180-H  suspension/yield/resume fence

F180-E ─┬─ F180-I  upgrade drain、epoch transition、旧implementation拒绝
        ├─ F180-J  Runtime disconnect/crash cleanup
        └─ F180-K  owner lease renewal/expiry与idle TTL sweeper

F180-H + F180-I + F180-J + F180-K
  └─ F180-L  真实compiler→Router→owner Runtime并发/恢复验收
```

### F180-B：共享Actor method与identity模型

- 生成非空Actor public method ABI；
- 定义独立method identity和Actor implementation identity；
- identity变化矩阵覆盖字段、id、签名、`maySuspend`、方法IR与可达依赖；
- 无关Actor或不可达代码不得改变目标Actor identity。

### F180-C：Compiler method call硬切

- Actor receiver必须生成专用target，不能再成为`LocalExecutable/ExternalServiceSymbol`；
- 真实源码覆盖本地及跨文件Actor declaration/impl/caller；
- 未知方法、错误参数、显式method type args、Actor public boundary/DB使用继续失败关闭；
- linker验证唯一declaration、method ABI、owner和implementation，并保持非Executable linked target。

### F180-D：Method wire/control

- Rust/TypeScript strict parity corpus；
- request/response/error/cancel/deadline全覆盖；
- 缺epoch、错误ABI/implementation/method或多余字段全部拒绝。

### F180-E 与 F180-F：可并行

- Router：原子owner CAS、activation、method admission、execution ledger、implementation选择。
- Runtime：从bootstrap按声明精确解码，建立`(logical key, epoch)`实例字段frame；不写回registry。

### F180-G 与 F180-H：Executor和协程

- Actor execution token是字段访问唯一权限；
- 同实例同步片段不交错，不同实例可并行；
- async service call、stream next、timer、send和显式yield释放实例执行权；
- continuation不持有裸字段引用，恢复前重新检查epoch/incarnation/implementation。

### F180-I/J/K：生命周期扇出

- Upgrade：关闭admission、active清零、旧方法安全点退出、epoch推进、目标bootstrap激活、旧版本拒绝。
- Crash：Runtime断连失败排队/执行调用、释放owner；下一调用从bootstrap重激活。
- Idle/lease：真实TTL和sweeper、owner renewal/expiry；idle逐出保留registry entry。

### F180-L：最终验收

必须用真实源码和两个Runtime目标覆盖：

- 同实例同步代码不交错；
- suspension后另一方法可修改字段，原方法恢复时看到新状态；
- 不同实例并行；
- 外部/后台task不能访问字段；
- stale epoch resume失败；
- same implementation跨service version复用；
- different implementation触发upgrade并拒绝后续旧实现；
- replace/remove、Runtime crash、idle TTL都丢弃live字段，并从原bootstrap重新激活；
- Router restart丢entry，业务可用`getOrCreate`重建；
- 调用没有exactly-once保证，失败、重试和副作用窗口可观测。

## 决策阻断

目标架构已经确定同步调用、单实例协程、suspension交错、epoch升级、错误类别及不持久化状态，足以开始
CP1、CP2和实例存储设计；当前没有需要用户选择的公共语义分叉。

但CP1与CP2是强制shared checkpoint，不能由consumer任务自行猜测以下内部协议：

- method identity和Actor implementation identity的canonical preimage/SCC算法；
- Actor method参数、返回、typed error与cancellation的exact wire framing；
- owner activation握手及method call correlation字段。

这些必须先作为共享artifact/wire设计落地，再允许Router和Runtime并行消费。

## 审计验证

- 只读检查compiler、artifact、linker、eval、host、transport及Router production路径；
- 反向搜索Actor method frame、executor、upgrading/error、idle TTL和disconnect hook；
- 独立compiler、Runtime executor、Router lifecycle三个审计结论一致；
- 未修改production代码。
