# Package、Service Contract 与 Deployment 架构

本文定义Skiff长期目标中的代码编译、service协议、deployment装配与runtime执行边界。它是
compiler、artifact、runtime、router和registry共同遵守的canonical架构契约，不是实现计划，也不
冻结最终YAML字段名或CLI拼写。

Skiff尚未发布。实现应直接收敛到本文模型，不为旧service源码、旧publication artifact或旧
compiler pipeline保留兼容层。

## 1. 核心结论

目标模型只有四个一等对象：

```text
PackageArtifact     唯一用户代码编译/发布owner的聚合产物
ServiceContract     无代码、可独立发布的service协议
ServiceDeployment   无源码的contract implementation与运行配置
RuntimeAssembly     一组deployment及其完整依赖闭包的可执行装配
```

`Publication`不是领域对象、共同父类型、artifact kind或compiler pipeline。`publish`只允许作为
registry/release写入不可变artifact与更新pointer的动作名称。

四个对象分别回答不同问题：

- PackageArtifact：有哪些代码，如何在同一linked program内调用。
- ServiceContract：service调用者可以调用什么，跨boundary的语言语义是什么。
- ServiceDeployment：哪个package callable实现哪个contract operation，以及用什么配置、状态和
  ingress运行。
- RuntimeAssembly：哪些deployment放在一起运行，它们的package/service依赖如何闭合。

它们没有共同aggregate。可以共享canonical type、signature、operation、value-plan和identity
framing等叶子类型，但不能用共享DTO重新制造隐式父模型。

## 2. 不变量

以下约束是实现和演进的硬边界：

1. Package是唯一用户源码与独立编译单元；service没有用户源码集合。
2. Package compile不读取宿主deployment配置，也不依赖provider implementation package。
3. ServiceContract先于具体implementation存在；consumer编译只依赖contract。
4. ServiceDeployment不解析AST、不重新做type/effect分析，只消费typed PackageArtifact。
5. package call与service call是不同语义；物理同进程不允许把service call退化成普通package call。
6. 第一版service binding全部是`InProcessBoundary`；缺少本地provider时assembly失败，不经router
   fallback。
7. 普通即时service call只要求linkable；跨request或持久边界才要求recoverable。
8. runtime replica加载完整同一assembly；replica之间heap、CPU调度和lifecycle独立，外部数据层按
   deployment配置共享。
9. code identity、service protocol identity、deployment revision与assembly identity必须分开。
10. 当前ActivationContext必须随async continuation、stream和callback显式传播；任何service call都以它
    解析caller binding slot并切换到provider owner。

## 3. Package 与 PackageArtifact

Package source由`.skiff`源码、`package.yml`、`api.yml`和静态资源组成。compiler只接受package
source root，不存在package/service kind分支。

PackageArtifact至少包含：

```text
PackageArtifact
  packageId / packageVersion / packageBuildId
  FileIrUnit refs
  PackageLocalAbi
  implementation links
  package dependency requirements
  contract compile requirements
  service runtime requirements
  config/resource/runtime capability requirements
  callable semantic facts
  boundary callable projections
  unresolved ServiceCallRefs
```

`PackageLocalAbi`描述同一linked program内的public symbol、canonical signature、nominal type、
public instance、const与executable link信息。它允许同一heap引用、alias、原地mutation和其它只在
local code composition中成立的值。

ContractRequirement引入的`ContractTypeId`可以出现在PackageLocalAbi和package callable signature中；
它在package内部仍是普通local value type。只有call site进入ServiceBinding时才执行boundary
materialization。

每个进入Package API、因而可能被deployment选择的callable还携带一个显式boundary状态：

```text
BoundaryCallableProjection
  = Available {
      operationContract: BoundaryOperationContract
      implementationRequirements: BoundaryImplementationRequirements
    }
  | Unavailable([BoundaryUnavailableReason...])
```

Package callable在compile时尚未绑定任何ServiceContract operation，因此PackageArtifact中的Available
projection只保存contract-agnostic的`BoundaryOperationContract` body，不能携带或伪造
`ContractOperationId`、contract stable key或完整`BoundaryOperationDescriptor`。同一个package callable可以
被多个ServiceDeployment显式映射到不同contract operations；deployment只在映射后比较双方的operation
contract body。

缺字段不表示不可用或尚未分析。PackageArtifact必须保存完成boundary判断所需的typed effect、
provenance和link facts，使deployment无需读取源码。

`BoundaryOperationContract`只承载boundary可观察的signature、error/stream/cancel/callback、value plan与
公开effect保证。`BoundaryOperationDescriptor`由ServiceContract在该body外增加真实
`ContractOperationId`与stable key。具体config/state/native capability requirement和完整may-effect属于
`BoundaryImplementationRequirements`，不能泄漏进ServiceProtocolIdentity。

同一个PackageArtifact可以同时：

- 被其它package直接链接；
- 实现一个或多个ServiceContract；
- 被多个ServiceDeployment revision复用；
- 在同一assembly内只链接一份代码，由多个activation context调用。

## 4. ServiceContract

ServiceContract是独立、无代码、版本化的typed协议artifact。它不是deployment的派生缓存，也不
引用provider package、build、route、config或runtime replica。

```text
ServiceContract
  serviceId
  contractVersion
  serviceProtocolIdentity
  operations: name/id -> BoundaryOperationDescriptor
  boundary schema closure
```

每个operation descriptor包含canonical参数、返回、throw/error、stream、cancel、callback与value
plan契约。Contract schema必须闭合，consumer不读取provider源码补充类型事实。

Contract declaration是code-free typed输入。具体文件拼写与authoring UX不属于本文；无论最终使用
YAML、IDL或从显式interface declaration生成，发布后的ServiceContract都是独立source of truth。
工具可以从package callable生成contract草稿，但不能让已发布contract随implementation自动漂移。

Contract先于implementation发布，因此循环service调用按两阶段处理：

```text
compile/publish all required ServiceContracts
  -> compile Packages against those contracts
  -> validate and publish ServiceDeployments
```

每份Contract schema closure自包含，因此普通`A -> B -> A`调用循环不形成contract compile循环；它只在
两个packages的ServiceRequirement graph中出现，等所有contracts发布后再编译。第一版不允许Contract
schema通过跨contract引用重新制造循环closure。

## 5. ServiceDeployment

ServiceDeployment是无源码的配置与typed binding artifact：

```text
ServiceDeployment
  serviceId / contractVersion / expectedProtocolIdentity
  deploymentRevision
  implementation PackageArtifact ref
  operationBindings: contractOperationId -> packageCallableId
  dependencyBindings
  ingress: externalSelector -> contractOperationId
  config/secrets bindings
  state/DB/actor/queue ownership
  timeout/resource/activation policy
```

operation mapping必须显式。人类配置可以写package public path，deployment projection必须把它解析
成稳定callable id后写入artifact。禁止按同名函数隐式绑定、自动暴露整个package API或在runtime按
display name猜target。

Ingress只绑定ContractOperationId，不直接绑定package path/callable。这样换implementation package时，
外部entry仍先经过同一个contract，再由operationBindings选择provider executable。

ServiceContract的nominal boundary types使用独立`ContractTypeId`。Provider package在compile时声明
`ContractRequirement`并直接在boundary callable signature中引用这些contract types；这只引入typed
compile dependency，不产生runtime service edge。Package自己的nominal type即使结构相同也不能充当
contract type；需要转换时，开发者在package中编写显式wrapper。

`dependencyBindings`只表达当前deployment对implementation package requirements的provider selector/约束，
不拥有全局解析结果。RuntimeAssembly projection负责在root set及闭包中解析唯一provider、验证闭包并生成
每个ActivationContext的binding vector。

deployment validation必须保证：

- 每个contract operation恰好映射一次；
- 不存在未声明的额外operation；
- target callable的boundary projection是`Available`；
- operation descriptor中的ContractTypeId、schema closure与contract逐项精确匹配；
- implementation may-effect满足contract公开effect保证，且所有implementation requirements得到binding；
- 第一版不生成用户语义adapter、字段兼容或fallback；package-local type转换必须写在显式wrapper中；
- implementation package及其依赖闭包可解析；
- config、state与runtime capability requirements全部得到唯一binding。

ServiceDeployment可以换package build、config、route或resource policy而保持同一contract version；
前提是protocol identity完全不变。变化由deployment revision表达。

## 6. 两类调用与三层契约

### 6.1 Package direct call

package dependency调用使用`PackageLocalAbi`和implementation links：

- 同一linked program内直接调用；
- 可以共享当前request heap与引用identity；
- 可以原地修改caller传入对象；
- 不经过service dispatcher，不切换activation owner；
- 不要求linkable或recoverable。

File IR 对 package direct call 使用唯一 canonical target，不携带 legacy publication operation ABI：

```text
CallTargetIr::PackageCallable {
  packageRef: PackageRefIr
  packageCallableId: PackageCallableId
}
```

同一引用同时进入 owner-local `ExternalRefTable.packageCallables`，元素为
`PackageCallableRef { packageRef, packageCallableId }`。`expectedLocalAbi` 只由对应
`PackageRequirement` 拥有，不在每个 call site 或 external-ref table 重复。链接按以下链路 fail closed：

```text
PackageRefIr::Dependency(alias)
  -> PackageRequirement(alias, expectedLocalAbi)
  -> dependency PackageArtifact.packageLocalAbi.localAbiIdentity
  -> dependency PackageArtifact.callableLinks[PackageCallableId]
  -> OperationTargetRef
```

compiler source resolution必须先从已验证的dependency PackageArtifact取得`PackageCallableId`；lowering只
保持该typed identity，不从symbol path重建target。File IR materialization校验package coordinate被
`PackageRequirement`覆盖；assembly/linker再次校验local ABI并解析`callableLinks`。不得把
`PackageCallableId`编码进`OperationAbiRef`，也不得恢复`PackageOperationIndex`、publication ABI builder或
used-symbol closure作为bridge。

### 6.2 Service boundary call

service dependency调用只解析到`ServiceContract` operation。assembly把它绑定到某个
ServiceDeployment，再选择物理binding：

```text
ServiceContract operation
  -> ServiceBinding
       -> InProcessBoundary       # 第一版唯一production实现
       -> RemoteBoundary          # 未来扩展
```

进程内实现必须保留boundary语义：切换到provider ActivationContext，按value plan materialize参数，
使用同一error/stream/cancel/callback contract，再materialize返回值。它不能因为地址可见就直接传递
本地引用或method table。

Consumer lowering不会链接provider executable，也不生成伪PackageArtifact。它保存结构化调用引用：

```text
ServiceCallRef
  serviceRequirementSlot
  contractOperationId
  expectedProtocolIdentity
```

linked instruction通过当前caller ActivationContext的service binding vector解析该slot。这样同一个
PackageArtifact可以只链接一份代码，同时被多个deployments复用；每个deployment仍能把同一requirement
绑定到不同provider。全局把call site patch到某个provider executable是错误的，因为它会混淆activation
owner和dependency binding。

Binding vector的逻辑key是`(callerPackageBuildId, serviceRequirementSlot)`，不是裸slot index；不同packages
都可以拥有slot 0。Package direct call进入dependency package后仍沿用当前ActivationContext，因而该package
发起的service call会读取同一activation下属于自己的slot。

挂起/恢复、stream producer/consumer和callback dispatch都必须携带显式ActivationContext owner；不能依赖
thread-local“当前service”。Callback调用切回capability owner后，返回时再恢复receiver context。

这里区分三层：

- ServiceContract：位置无关的语言语义。
- Binding ABI：`InProcessBoundary`或未来remote transport的物理适配接口。
- PackageLocalAbi：provider内部最终调用executable的本地代码ABI。

本地和远程binding不需要相同的机器ABI；它们必须实现同一个ServiceContract。

## 7. Linkable、Recoverable 与 Callback Capability

即时service call使用lane-scoped linkable plan：

```text
LinkableValuePlan<ServiceCallLane>
  = 当前调用期间可materialize的carrier、encoding、owner和lifetime计划

RecoverableValuePlan<Lane>
  = LinkableValuePlan<Lane> + FutureValidityPlan
```

DB、spawn、queue、persistent work item或其它跨request lane才要求recoverable。普通service参数、返回
和error payload不因为是boundary call就必须在未来request中恢复。

ordinary data按contract生成detached value graph。caller可观察的alias、共享heap identity或原地
mutation不能穿过service boundary。

本地`any I`或native handle若要跨service，只能投影成request-scope callback capability：

```text
CallbackCapability
  ownerRuntime / ownerActivation
  requestGeneration
  interfaceOrAdapterContract
  opaqueCapabilityId
```

约束：

- capability由创建该值的activation拥有；
- 生命周期到顶层request结束，stream存在时延长到stream关闭；cancel或owner退出会提前失效；
- 对端只能通过contract声明的operation回调owner，不能得到method table、native object或本地地址；
- capability不能进入DB、spawn、queue、persistent payload或其它recoverable lane；
- 失效返回稳定`CapabilityExpired`/`CapabilityUnavailable`错误，不重建、不fallback；
- `any I`只有所有被投影method都boundary-capable时才可生成callback；native value必须有显式callback
  adapter，否则对应operation不可用。

InProcessBoundary用runtime capability table实现；未来RemoteBoundary使用opaque route回到owner。
两者对语言层值保持同一lifetime与失效语义。

## 8. Effect 与 Boundary Eligibility

所有package callable都可以拥有Local ABI；只有boundary projection为`Available`的callable能实现
ServiceContract operation。

compiler执行sound may-analysis，至少追踪：

- caller-reachable参数图的write；
- 返回或throw payload是否alias caller graph；
- caller value是否escape到capture、callback、stream、spawn、DB或native/external target；
- 是否依赖same-heap identity；
- callback/native adapter requirement；
- unknown call/effect。

分析允许保守拒绝，不允许漏掉boundary-visible行为。mutable helper、返回参数alias的函数和依赖本地
identity的算法仍是合法package API，但deployment选择它们时以结构化原因失败。

第一版不要求新增`local`/`remote`源码修饰符。未来annotation只能作为compiler assertion，不能成为
绕过分析的第二套规则。

## 9. Compiler 与 Projection 流水线

package compilation与service deployment projection是两条不同pipeline：

```text
PackageCompileInput
  -> PackageSourceModel
  -> LoweredPackage
  -> CompiledPackage
  -> PackageArtifactProjection
  -> PackageArtifact + FileIrUnit[]

ServiceContractDefinition
  -> ServiceContractArtifact

ServiceDeploymentInput
  + ServiceContractArtifact
  + PackageArtifact closure
  -> ServiceDeploymentProjection
  -> ServiceDeployment
```

PackageSourceModel拥有name/type resolution、public API graph、effect/provenance与dependency facts。
Lowering只消费typed source facts。package direct call降低为`PackageCallable` target；service call降低为
`ServiceCallRef`。Package projection不读deployment配置。

Service call lowering只生成`ServiceCallRef`和contract value plan refs。Assembly linking为每个
ActivationContext生成service binding vector / thunk；它不是stub package，也不让consumer依赖provider
PackageLocalAbi。

Deployment projection不拥有AST、source text或lowering helper。平台即使持有全部源码，也只能将其用于
统一调度、诊断和可选whole-assembly优化；正确性必须只依赖typed artifacts，否则Package不再是独立
编译单元。

compiler内部不存在`PublicationInput`、`PublicationKind`、`CompiledPublication`、
`LoweredPublication`或带package/service option的共同projection bundle。

## 10. 依赖与 Identity

package dependency、contract compile dependency和service runtime dependency是三种edge：

```text
PackageRequirement
  alias + packageId + exactVersion + expectedLocalAbi

ContractRequirement
  alias + serviceId + contractVersion + expectedProtocolIdentity

ServiceRequirement
  contractRequirement + serviceBindingSlot + usedOperations
```

ContractRequirement允许package解析contract types和operation signatures，但不要求provider。
只有实际service call sites产生ServiceRequirement和runtime binding slot。二者都不包含provider package、
provider build、deployment revision或runtime route；最终assembly只为ServiceRequirement选择deployment。

必须分开的identity：

- PackageId / PackageVersion：代码发布坐标。
- PackageBuildId：具体不可变代码build。
- PackageLocalAbiIdentity：local public code ABI。
- ServiceId / ContractVersion：consumer依赖坐标。
- ServiceProtocolIdentity：canonical boundary surface内容身份。
- DeploymentRevision / DeploymentArtifactIdentity：某次implementation、配置与route revision。
- AssemblyIdentity：完整resolved deployment/package graph。
- RuntimeReplicaId：某个assembly实例，不进入artifact contract。

任何identity都不能因为display string相同而互换。ServiceProtocolIdentity不包含provider package或
deployment字段；AssemblyIdentity可以记录最终选择的build作为复现事实，但不能回写consumer
requirement。

## 11. Config、State 与 Resource Owner

Package可以声明运行所需config path、外部resource capability、DB/schema或native adapter requirement，
但不拥有deployment中的实际值和state namespace。

ServiceDeployment负责：

- 绑定package及其transitive requirements；
- 提供config/secrets；
- 选择DB、Redis、actor、queue等外部state namespace；
- 定义timeout、quota、principal与lifecycle policy。

Package静态资源随PackageArtifact发布，并按当前执行callable的package owner读取。ServiceDeployment
没有用户代码资源；deployment-only证书、secret和环境文件属于activation输入，不进入code artifact。

同一个PackageArtifact被两个service使用时，代码和静态资源可共享，ActivationContext、config、state
owner和lifecycle必须分开。

## 12. RuntimeAssembly 与扩容

RuntimeAssembly由显式root deployment set做依赖闭包：

```text
RuntimeAssembly
  roots: ServiceDeployment[]
  resolvedServiceDeployments
  resolvedPackageArtifacts
  linkedProgramImage
  serviceBindingTemplatesByActivation
  ActivationContext templates
  assemblyIdentity
```

第一版每个environment只有一个active assembly，root set是该环境全部active services。每个runtime
replica加载完整相同assembly：

- package code在replica内只链接一次；
- service binding全部解析为`InProcessBoundary`；
- 每个service拥有独立ActivationContext；
- 每个ActivationContext拥有自己的service/config/state binding vector，共享package executable不共享这些
  bindings；
- replica内共享的是只读code/type/link image；activation-owned config view、state handle、callback table与
  任何mutable runtime state不得因PackageBuildId相同而共享；
- replica之间heap、CPU调度、request lifecycle与failure独立；
- MongoDB、Redis等外部数据层按deployment配置共享。

这能整体扩CPU、内存与副本可用性，但不能单独隔离或扩缩某个service；一个service的CPU/memory故障
可能影响同replica内其它service。第一版明确接受这一限制，并要求assembly admission、health、drain和
atomic reload在runtime层可观测。

未来若需要独立扩缩容，平台可以为不同root set生成多个assembly。届时assembly projection把当前完整
本地闭包拆成`LocalExecutableClosure`与`RemoteBindingRefs`；只有跨assembly service edge选择
`RemoteBoundary`，远端provider不进入本地code closure。ServiceContract与PackageArtifact不需要改变。

## 13. Registry、Release 与 Publish

registry分别存储不可变PackageArtifact、ServiceContract、ServiceDeployment与RuntimeAssembly record。
release pointer可以选择contract-compatible deployment revision和active assembly。

`publish`是操作：校验typed artifact、写入不可变内容、更新允许更新的pointer。它不产生
`Publication`对象，也不要求四类artifact实现共同kind enum。registry可以在一个事务/workflow中发布
多个artifact，但每个artifact保持独立schema与identity。

## 14. Fail-closed 条件

以下情况必须在compile、deployment projection或assembly阶段失败，不能留到请求时猜测：

- contract schema不闭合或operation identity冲突；
- deployment operation缺失、重复、额外或descriptor不匹配；
- provider boundary signature使用package-local nominal type冒充ContractTypeId，或descriptor不匹配；
- selected callable boundary unavailable；
- service/package dependency缺失、版本或identity不匹配；
- assembly内同一service requirement有零个或多个provider；
- callback/native adapter缺失或lifetime无法表达；
- runtime需要重读源码、display name或raw JSON才能链接；
- shared package call site被全局绑定到单一service provider而绕过caller ActivationContext；
- 第一版需要remote provider才能闭合assembly。

## 15. 非目标

- 不提供任意package function的透明RPC。
- 不让所有package public API都强制boundary-safe。
- 不实现RemoteBoundary、service级进程隔离或独立扩缩容。
- 不定义历史artifact、manifest或数据库内容的兼容迁移。
- 不在本文冻结ServiceContract authoring文件格式、deployment YAML字段名或CLI命令；这些表面语法必须
  在保持本文owner与数据流不变的前提下另行定义。
