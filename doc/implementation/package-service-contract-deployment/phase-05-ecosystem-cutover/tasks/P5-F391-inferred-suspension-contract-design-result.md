# P5-F391 Inferred suspension contract design result

状态：Design complete；无新增未决语义，可进入只读实现审计。

## 权威链与取代关系

- 直接父审计：
  `P5-F382-interface-suspension-projection-audit-result.md`
- 用户可见语言事实源：
  `doc/reference/interface.md` §3、§4、§11，
  `doc/reference/static-semantics.md` §12、§12.1，
  `doc/reference/any-interface.md` §2.3、§2.4、§8、§9
- 长期内部架构事实源：
  `doc/architecture/actor-model.md`“常驻实例与协程并发”，
  `doc/architecture/package-service-contract-deployment.md` §2、§3、§4、§5、§6.2、§9、§10，
  `doc/architecture/any-interface-value.md`“Boxing”“Method Table”“Dynamic Dispatch”“Remote
  Fail-Closed”，
  `doc/architecture/compiler-package-pipeline.md`“PackageSourceModel”“ServiceContract Projection”

F382的production population、逐跳字段位置和49/47统计仍是实现审计证据；其“现有文档要求interface
effect contract”和A/B/C/D待选择结论已被本结果及上述权威文档取代。若本文与权威文档冲突，以权威文档
为准。

## 冻结语义

- 不增加`async`、`suspending`、effect declaration或显式`yield`关键字。
- 每个有函数体的concrete executable从body、调用图、native/builtin summary和真实等待点固定点推断
  `maySuspend`。
- concrete public Package callable发布精确推断summary，package dependency直接消费它，不读取依赖源码
  重算。
- interface method requirement没有suspension位；conformance不比较implementation summary。同一
  requirement允许同时存在suspending与non-suspending implementations。
- 静态已知concrete、package direct callable或public-instance concrete binding使用该callable summary；
  `any I`或未知interface dispatch保守为`maySuspend=true`。
- service call不读取callee summary：`ServiceCallRef`这一调用种类本身就是caller的潜在挂起点。
  caller只在response尚未就绪、实际等待时释放actor executor。
- ServiceContract不包含callee internal `maySuspend`，也不包含由该位机械映射的
  `NotCancellable`/`Cooperative`类别。pending service call统一参与request deadline与ancestor
  cancellation；provider是否、何时停止是implementation/deployment执行机制，不是协议承诺。
- Host/runtime若需要callee summary选择执行lane、cancel signal投递或其它内部机制，只从
  PackageArtifact / deployment implementation metadata取得。
- 保守`maySuspend=true`不会插入调度点。runtime仍只在stream next、service response wait、timer等真实
  等待尚未完成时让出；同步完成的concrete执行不会因interface保守分析发生交错。

## Compatibility 与 identity 结果

本任务只定义未发布语言的新一代严格artifact；不提供旧字段dual-read、默认填充、fallback或hash兼容。

| 只改变的事实 | stable callable / operation id | Package Local ABI | Package build | interface requirement / conformance | callback-interface PackageSchemaTypeId | ServiceProtocolIdentity | implementation / deployment / assembly |
| --- | --- | --- | --- | --- | --- | --- | --- |
| concrete public callable从non-suspending变为suspending | `PackageCallableId`稳定；作为service root时`ContractOperationId`也稳定 | 改变 | 改变 | 不变 | 不变 | request/response/stream/callback/错误语义不变时不变 | provider build、deployment revision/identity及assembly选择可以改变 |
| private concrete implementor只改变internal summary | stable method/call target identity不因该summary重命名 | 若未进入public callable则不因它单独改变 | 改变 | 不变 | 不变 | 不变 | implementation链改变 |
| interface requirement调用形状改变 | effect无关；按既有stable-key规则处理 | 改变 | 改变 | 改变 | 若进入callback schema则改变 | 被operation引用时改变 | 依赖闭包重建 |
| service operation request/response/stream/callback或公开错误语义改变 | `ContractOperationId`仍由service + stable operation key决定 | 视Package API变化而定 | 改变 | 视来源而定 | 视schema变化而定 | 改变 | consumer fail closed并重建 |

从新模型落地开始，identity golden必须证明“implementation summary mutation不改变protocol”，不能继续沿用
F382中`maySuspend/cancellation`改变ServiceProtocol的旧golden。一次性删除wire字段时必须同步提升相应
strict schema与identity generation；generation切换本身不是兼容承诺，也不能用旧hash与新hash做语义
稳定性断言。

## 后继实现审计清单

后继先做只读审计，再按共享schema/identity checkpoint、compiler、runtime/tooling consumer拆分非重叠
实现任务。以下是必须逐项分类的production owner；fixture和golden只随对应owner迁移。

### A. Interface requirement 与 callback schema：删除旧位

- 删除`artifact-model/src/package_unit.rs::InterfaceMethodSignature.may_suspend`及wire
  `maySuspend`。`syntax::InterfaceOperation`、`SourceInterfaceRequirementSignature`和
  `InterfaceOperationIr`当前没有该字段，保持没有，不能补默认值。
- 删除`compiler/core/src/package_interface_methods.rs`与
  `compiler/projection/src/package_artifact/callables/mod.rs`为interface method硬编码
  `may_suspend: false`的路径；同步清理visible-type、type substitution、normalization和artifact
  reconstruction中的复制。
- `compiler/source/.../interfaces/conformance.rs`保持只比较receiver、parameters、return和其它
  requirement-owned shape；增加正反测试防止后续把suspension重新加入。
- 把`artifact-identity/.../validation/public_instances.rs`的三方suspension equality改为：
  interface不参与；concrete public signature仍必须与其implementation link/executable的summary精确一致。
  不能为了让Relay通过而完全删除concrete一致性验证。
- 删除`artifact-model/src/contract_types.rs::BoundaryCallbackOperation.may_suspend`。同步清理
  `compiler/projection/src/package_artifact/schema.rs`的interface-to-callback-schema投影、
  `compiler/source/src/type_resolution_model.rs`的重建，以及
  `runtime/model/src/callback_projection.rs::CallbackContractOperationProjection.may_suspend`。
- 删除callback adapter对interface requirement summary与concrete executable的equality，包括
  `runtime/eval/src/assembly_execution/callback_native.rs`等validator。callback/interface动态调用在caller
  分析中保守为可能挂起；runtime method table只需调用形状与concrete target。

### B. Concrete executable / Package callable：必须保留

以下`may_suspend`不是旧interface/protocol字段，不得全局删除：

- `SourceExecutableSignature`、`ExecutableSignatureIr`、`ExecutableIr`及linked executable；
- `CallableMayEffects`、callable semantic facts与
  `BoundaryImplementationRequirements.complete_may_effects`；
- canonical `PackageCallableSignature`，以及仍存在期间只表示concrete Package callable的legacy
  `CanonicalPublicCallableSignature`；
- concrete gateway handler/entry signature与actor public method的implementation/ABI summary。

审计必须确认：

- public callable summary仍进入Package Local ABI与build preimage；
- `PackageCallableId`仍只由stable public path决定；
- public-instance method的public callable summary来自source exact executable fact，不从interface、
  FileIR requirement或service descriptor补值；
- router/runtime读取legacy publication summary时只把它当concrete implementation/package fact，不能再
  反向生成ServiceContract。

### C. 调用图传播：按target种类迁移

- `compiler/lowering/src/suspend_analysis.rs`当前把
  `DependencyPackageFunction | ContractOperation | Unknown`统一返回`true`。迁移后：
  dependency Package callable读取精确已发布summary；`ContractOperation`与`Unknown`仍为`true`；
  interface/`any I` target必须有显式保守分支。
- `compiler/source/src/callable_effects/transfer/call.rs`的dependency Package path继续消费
  `CallableSemanticFacts`；`detached_contract_callee`不得再读取
  `operation.contract.may_suspend`，而应因target是service call直接设caller-side
  `may_suspend=true`。
- `compiler/source/src/expression_type_model.rs`等为service test/effect target制造
  `PackageCallableSignature` view的路径，不得把不存在的provider contract bit伪装成concrete summary；
  call-site suspension由service target种类给出。
- 已知local/concrete/public-instance package-direct target继续参加固定点传播；缺summary、未知动态target
  一律fail closed为可能挂起。

### D. ServiceContract 与 runtime：删除/迁移协议字段

- 删除`artifact-model/src/boundary/operation.rs::BoundaryOperationContract.may_suspend`。
- 从ServiceContract operation删除当前由它派生的`cancellation` /
  `BoundaryCancellationContract::{NotCancellable, Cooperative, Unsupported}`。若gateway ingress仍需要
  独立external cancellation policy，把它迁到gateway entry/deployment owner；不得留在共享
  ServiceContract body，也不得从callee summary推导。
- 清理`compiler/projection/.../boundary/types.rs`的
  `may_suspend -> cancellation`映射；ServiceContract definition/compile/projection只接收code-free
  boundary shape。
- source contract-call typing、dependency ingest与effect transfer不得要求或读取上述字段。所有
  `ServiceCallRef`在caller分析中固有为可能挂起。
- runtime assembly admission与dispatch不得比较provider executable summary和ServiceContract。当前
  `assembly_execution/mod.rs`按contract bit选择ordinary/async lane、
  `ordinary.rs`拒绝suspending operation、`websocket_contract_plan.rs`比较executable/contract、
  callback projection复制contract summary等路径都必须审计并分域迁移。
- caller-side service wait与取消不因provider summary而分叉。Host需要优化同步provider或决定内部cancel
  signal机制时，从deployment绑定的PackageArtifact读取summary；无论选择哪条内部lane，都必须保持同一
  ActivationContext、boundary materialization、错误、stream和caller cancellation语义。
- HTTP/WebSocket gateway的linked executable summary可以保留为implementation校验；但它属于gateway
  entry/deployment，不得为ServiceContract补回`maySuspend`或cancellation字段。

### E. Strict schema、identity 与 golden 类别

字段删除会同时触及以下generation owner，必须在同一共享checkpoint审计并一次性更新，禁止兼容读取：

- PackageArtifact / legacy PackageUnit中interface method DTO；
- PackageSchema callback-interface descriptor；
- ServiceContractDefinition与ServiceContract operation body；
- 若implementation execution metadata迁移改变wire，ServiceDeployment及RuntimeAssembly消费面；
- Package Local ABI、Package build、PackageSchemaType、ServiceProtocol、deployment与assembly identity
  marker/prefix及其canonical normalization。

新的mutation/golden矩阵至少覆盖：

1. 同一interface requirement分别由`maySuspend=false/true` implementation满足，均conform；
2. public-instance public signature与concrete executable summary不一致仍fail closed；
3. concrete public callable summary mutation：Local ABI/build改变，`PackageCallableId`稳定；
4. dependency package call精确传播`false/true`summary；
5. `any I`/未知interface调用始终保守为`true`，但同步concrete执行不产生额外runtime yield；
6. provider summary mutation：ServiceContract bytes/protocol identity与`ContractOperationId`不变，
   deployment/build变化；
7. service call对两种provider summary都在caller侧为`true`并使用同一boundary/cancellation语义；
8. callback-interface `PackageSchemaTypeId`不因implementor summary变化；
9. 新wire拒绝legacy interface/callback/service-contract `maySuspend`与旧derived cancellation字段；
10. operation request/response/stream/callback shape变化仍改变ServiceProtocol；operation stable key不变时
    `ContractOperationId`稳定。

## 重建与验收边界

实现后按F382记录的真实生态DAG fresh重建interface owners、implementors、package dependents、
ServiceContracts、deployments与consumers。Relay最小独立链仍是
`std -> llm-api -> llm-providers -> Relay`，随后Aihub/Agine消费新artifact generation。不得复用F382的
validator-waiver probe、旧protocol receipt或旧锁定。

本设计节点没有修改production、test或artifact schema，也没有运行live instance。
