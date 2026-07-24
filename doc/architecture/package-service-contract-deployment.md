# Package、Service Contract 与 Deployment 架构

本文定义Skiff长期目标中的代码编译、service协议、deployment装配与runtime执行边界。它是
compiler、artifact、runtime、router和registry共同遵守的canonical架构契约，不是实现计划，也不
冻结最终YAML字段名或CLI拼写。

Skiff尚未发布。实现应直接收敛到本文模型，不为旧service源码、旧publication artifact或旧
compiler pipeline保留兼容层。

## 1. 核心结论

目标模型只有四个发布/运行记录：

```text
PackageArtifact     package源码的不可变编译产物
ServiceContract     service package公开API的code-free boundary projection
ServiceDeployment   工具由service package与config profile生成的不可变运行记录
RuntimeAssembly     一组deployment及其完整依赖闭包的可执行装配
```

`Publication`不是领域对象、共同父类型、artifact kind或compiler pipeline。`publish`只允许作为
registry/release写入不可变artifact与更新pointer的动作名称。

四个对象分别回答不同问题：

- PackageArtifact：有哪些代码，如何在同一linked program内调用。
- ServiceContract：同一个package公开API中哪些函数可作为service调用，以及跨boundary的语言语义。
- ServiceDeployment：工具把哪个service package build、config profile和生成的routing/resource binding
  装配为一次不可变运行revision。
- RuntimeAssembly：哪些deployment放在一起运行，它们的package/service依赖如何闭合。

它们没有共同aggregate。可以共享canonical type、signature、operation、value-plan和identity
framing等叶子类型，但不能用共享DTO重新制造隐式父模型。

## 2. 不变量

以下约束是实现和演进的硬边界：

1. Package是唯一用户源码与独立编译单元；service首先是package，只比普通package多`service.yml`和
   `config.*.yml`。
2. 普通package root包含`.skiff`、`package.yml`和`api.yml`；service root在此基础上增加
   `service.yml`与零个或多个`config.*.yml`。service root不得缺少`package.yml`，也不存在开发者维护的
   `deployment.yml`。
3. `api.yml`是package call与service call共用的唯一公开API owner；service不得在`service.yml`中重复声明
   type、function或interface。
4. ServiceContract由compiler/tooling从同一份typed package API确定性投影；consumer只依赖发布后的
   code-free projection，不读取provider实现源码。
5. ServiceDeployment不解析AST、不重新做type/effect分析；它由工具消费typed PackageArtifact、
   ServiceContract、`service.yml`和所选`config.*.yml`生成。
6. package call与service call是不同语义；物理同进程不允许把service call退化成普通package call。
7. 第一版service binding全部是`InProcessBoundary`；缺少本地provider时assembly失败，不经router
   fallback。
8. 普通即时service call只要求linkable；跨request或持久边界才要求recoverable。
9. runtime replica加载完整同一assembly；replica之间heap、CPU调度和lifecycle独立，外部数据层按
   deployment配置共享。
10. code identity、service API identity、deployment revision与assembly identity必须分开；任何人类可读
    `version`都不参与内容identity计算。
11. 当前ActivationContext必须随async continuation、stream和callback显式传播；任何service call都以它
    解析caller binding slot并切换到provider owner。
12. actor、spawn及其它跨request control必须携带当前完整ActivationIdentity；Router只按发送该frame的
    exact assembly registration及active/draining generation验证，不能按serviceId、package build、display
    name或legacy runtime registration补事实。

## 3. Package 与 PackageArtifact

Package source由`.skiff`源码、`package.yml`、`api.yml`和静态资源组成。`package.yml`拥有package的人类
可读`id`/`version`、package dependencies与service dependencies；两类dependency alias共享同一namespace，
版本selector第一版都必须精确。`version`只用于人类展示和解析坐标，不参与任何artifact、ABI或内容identity
运算。

Service仍走同一个package compiler入口；存在`service.yml`时，compiler/tooling再执行service projection。
`service.yml`只拥有service id与HTTP/WebSocket ingress，不含version、dependency、API type/function映射、
实现artifact binding、平台组织角色或request/response大小策略。`config.*.yml`只绑定已经声明的
config/secret/state/resource requirement，不改变package/service dependency graph。

PackageArtifact至少包含：

```text
PackageArtifact
  packageId / packageVersion / packageBuildId
  FileIrUnit refs
  PackageLocalAbi
  implementation links
  package dependency requirements
  service runtime requirements
  config/resource/runtime capability requirements
  callable semantic facts
  boundary callable projections
  unresolved ServiceCallRefs
```

`PackageLocalAbi`描述同一linked program内的public symbol、canonical signature、nominal type、
public instance、const与executable link信息。它允许同一heap引用、alias、原地mutation和其它只在
local code composition中成立的值。

File IR executable signature与PackageLocalAbi不是同一个type owner。File IR只保存本地执行需要的
execution type representation。Package API与Service API必须共用同一套parser、name resolution、nominal
type、field/constructor、generic、interface conformance和typed expression机制；不得从ServiceContract
descriptor再建立一套只支持签名、不支持普通表达式的contract type system。

Service dependency导入的code-free API module是同一canonical public API representation的materialized view。
它的nominal type identity由service API schema稳定拥有，不绑定provider build；但其语言行为与package API
type完全一致。compiler不得用display string、结构相同的临时local type、JSON encode/decode或手写wrapper
冒充相同identity。

PackageArtifact的public-instance discovery只需要`publicPath + receiver execution target`。它不得把File IR
execution signature转成public signature、执行conformance比较或生成`OperationAbiRef`/operation protocol
identity；exact Local ABI继续只由source exact facts经compiled/projection-input handoff附着。若legacy runtime
adapter仍需要旧operation DTO，只能在canonical PackageArtifact之外消费typed semantic input生成。

当前runtime若仍从File IR signature推导service boundary materialization，可以在后续runtime阶段暂时fail
closed；终态必须改读ServiceContract descriptor，不能据此反向扩张File IR语义。

每个进入Package API、因而可能被deployment选择的callable还携带一个显式boundary状态：

```text
BoundaryCallableProjection
  = Available {
      operationContract: BoundaryOperationContract
      implementationRequirements: BoundaryImplementationRequirements
    }
  | Unavailable([BoundaryUnavailableReason...])
```

普通package允许同时拥有Available和Unavailable public functions。存在`service.yml`时，`api.yml`中的每个
Available public function自动成为service operation；Unavailable function仍是合法package API，但不会进入
ServiceContract。compiler/tooling必须输出完整、稳定、可机器读取的列表及结构化原因；构建摘要、CLI/JSON、
artifact receipt与IDE应消费同一projection，不能静默排除。

缺字段不表示不可用或尚未分析。PackageArtifact必须保存完成boundary判断所需的typed effect、
provenance和link facts，使deployment无需读取源码。

`BoundaryOperationContract`只承载boundary可观察的signature、error/stream/cancel/callback、value plan与
公开effect保证。ServiceContract projection在该body外增加稳定operation key/id。具体
config/state/native capability requirement和完整may-effect属于
`BoundaryImplementationRequirements`，不能泄漏进ServiceProtocolIdentity。

同一个PackageArtifact可以同时：

- 被其它package直接链接；
- 在存在`service.yml`时生成一个ServiceContract并实现其全部自动投影operations；
- 被多个ServiceDeployment revision复用；
- 在同一assembly内只链接一份代码，由多个activation context调用。

## 4. ServiceContract

ServiceContract是独立发布、无代码的typed API projection artifact。它的唯一authoring source是service
package自己的`.skiff` declarations与`api.yml`，加上`service.yml`中的service id；不存在独立contract
YAML/IDL、类型映射或第二套函数清单。它不引用provider build、config或runtime replica。

```text
ServiceContract
  serviceId
  packageVersionLabel
  serviceProtocolIdentity
  operations: name/id -> BoundaryOperationDescriptor
  boundary schema closure
```

每个operation descriptor包含canonical参数、返回、throw/error、stream、cancel、callback与value
plan契约。Contract schema必须闭合，consumer不读取provider源码补充类型事实。

Service API identity由canonical boundary surface内容确定；`package.yml.version`只作为人类可读、精确解析
label，不参与identity运算，`service.yml`没有version。新implementation build可以在identity不变时替换当前
active revision而无需consumer同意；API内容变化必然产生新identity，即使作者未修改version label也不能
静默绑定旧consumer。

为支持循环service dependency，compiler/tooling按两阶段处理同一批service package source：

```text
project/publish all service API declarations
  -> compile package bodies against exact service API projections
  -> generate ServiceDeployments
```

这不是第二套语言前端：declaration projection与package compile共用canonical typed API机制，只把函数体
执行编译推迟到所有service API closure可用之后。第一版不允许ServiceContract schema通过跨contract type
引用重新制造循环closure。

## 5. ServiceDeployment

ServiceDeployment是无源码的生成artifact，不是开发者维护的`deployment.yml`：

```text
ServiceDeployment
  serviceId / packageVersionLabel / expectedProtocolIdentity
  deploymentRevision
  implementation PackageArtifact ref
  operationBindings: contractOperationId -> packageCallableId
  dependencyBindings
  ingress: externalSelector -> contractOperationId
  config/secrets bindings
  state/DB/actor/queue ownership
  timeout/resource/activation policy
```

operation mapping由同一service package的ServiceContract projection与PackageArtifact public callable
identity确定性生成。所有Available public functions自动进入；不得要求开发者在`service.yml`或
`deployment.yml`重复映射。生成artifact必须写入稳定callable id，runtime禁止按display name猜target。

Ingress只绑定ContractOperationId，不直接绑定package path/callable。这样换implementation package时，
外部entry仍先经过同一个contract，再由operationBindings选择provider executable。

Package source中的service dependency alias使用现有qualified namespace，不新增另一套type/import语法：

- `payments.User`按`package.yml.services`中的`payments`解析到发布的service API module；
- `payments/charge(...)`按同一validated ServiceContract中的operation descriptor解析，并在source typed
  analysis阶段检查参数与返回类型；
- dependency source address复用现有`<dependencyAlias>/<publicPath>`语法。`/`分隔dependency resolver root与
  public source-call path；`.`只用于type qualified path和address之后的成员访问，例如
  `payments.User`、`payments/managed.charge(...)`；
- package call与contract/service call都使用`/`地址；linkage kind来自validated dependency alias，不由
  分隔符或物理local/remote binding猜测。`payments.charge(...)`不作为旧兼容拼写接受；
- package dependency alias与service dependency alias共享一个dependency alias namespace，任何冲突在
  compile input trust boundary fail closed，不能靠type/call上下文猜测；
- qualified alias只选择typed dependency，不进入ContractTypeId/ContractOperationId本体，也不能从provider
  package、deployment或display name补事实。

依赖种类只来自`package.yml`的validated `packages`/`services` entry，不由call syntax、物理local/remote
binding或运行时猜测。

`dependencyBindings`只表达当前deployment对implementation package requirements的provider selector/约束，
不拥有全局解析结果。RuntimeAssembly projection负责在root set及闭包中解析唯一provider、验证闭包并生成
每个ActivationContext的binding vector。

deployment validation必须保证：

- 每个自动生成的service operation恰好映射到其source public callable；
- 不存在手工增加、遗漏或重复operation；
- target callable的boundary projection是`Available`；
- operation descriptor、schema closure与同一canonical API projection逐项精确匹配；
- implementation may-effect满足contract公开effect保证，且所有implementation requirements得到binding；
- 第一版不生成用户语义adapter、字段兼容或fallback；
- implementation package及其依赖闭包可解析；
- config、state与runtime capability requirements全部得到唯一binding。

ServiceDeployment可以换package build、config或resource policy而保持同一service id/version label；
前提是service API identity完全不变。变化由deployment revision表达。

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
它还为全部function/impl method拥有exact executable signature facts，并为全部interface operation拥有exact
requirement facts；public callable signature只是executable事实表按public path产生的view。interface conformance
只比较这两类exact facts。Lowering只消费typed source facts，把exact source type投影为File IR execution
representation。package direct call降低为`PackageCallable` target；service call降低为`ServiceCallRef`。
compiled/projection-input只转交source-validated interface binding/method key与execution target所需结构事实，
不从File IR或`TypeResolutionModel`重算conformance。Package projection不读deployment配置，也不从File IR
execution signature重做source conformance或ABI identity。

Service call lowering只生成`ServiceCallRef`和contract value plan refs。Assembly linking为每个
ActivationContext生成service binding vector / thunk；它不是stub package，也不让consumer依赖provider
PackageLocalAbi。

Deployment projection不拥有AST、source text或lowering helper。平台即使持有全部源码，也只能将其用于
统一调度、诊断和可选whole-assembly优化；正确性必须只依赖typed artifacts，否则Package不再是独立
编译单元。

compiler内部不存在`PublicationInput`、`PublicationKind`、`CompiledPublication`、
`LoweredPublication`或带package/service option的共同projection bundle。

## 10. 依赖与 Identity

package dependency与service dependency是两种edge，都由`package.yml`声明：

```text
PackageRequirement
  alias + packageId + exactVersion + expectedLocalAbi

ServiceRequirement
  alias + serviceId + exactVersionLabel + expectedProtocolIdentity
  serviceBindingSlot + usedOperations
```

PackageRequirement在link阶段解析为不可变PackageArtifact。ServiceRequirement允许package解析发布后的
service API types和operation signatures，但不要求provider implementation；只有实际service call sites产生
runtime binding slot。它不包含provider package、provider build、deployment revision或runtime route；
最终assembly只为ServiceRequirement选择deployment。

必须分开的identity：

- PackageId / PackageVersion：人类可读代码发布坐标；version不参与任何identity hash。
- PackageBuildId：具体不可变代码build。
- PackageLocalAbiIdentity：local public code ABI。
- ServiceId / exact PackageVersion label：consumer依赖坐标；service.yml不重复version。
- ServiceProtocolIdentity：canonical boundary surface内容身份。
- DeploymentRevision / DeploymentArtifactIdentity：某次implementation、配置与route revision。
- AssemblyIdentity：完整resolved deployment/package graph。
- RuntimeReplicaId：某个assembly实例，不进入artifact contract。

任何identity都不能因为display string相同而互换。ServiceProtocolIdentity不包含provider package或
deployment字段；AssemblyIdentity可以记录最终选择的build作为复现事实，但不能回写consumer
requirement。

## 11. Config、State 与 Resource Owner

Package可以声明运行所需config path、外部resource capability、DB/schema或native adapter requirement，
但不拥有环境中的实际值和state namespace。普通package可以在`package.yml.services`声明service
dependency；这使其可复用业务编排在最终宿主service的ActivationContext中解析provider，不把具体provider
写入PackageArtifact。

Service source的`config.*.yml`选择或提供：

- 提供config/secrets；
- 选择DB、Redis、actor、queue等外部state namespace；
- 定义timeout、quota、principal与lifecycle policy。

tooling把所选profile与精确PackageArtifact、生成的ServiceContract及闭合dependency resolution投影为
ServiceDeployment。profile不得增加/删除`package.yml`中的package或service dependency。

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

Package link与service binding使用不同的可变性边界：

- package requirement在link完成后绑定到不可变`PackageArtifact` identity。最终linked image记录精确
  `PackageArtifactId`；`packageBuildId`可以作为构建过程与诊断标识，但不能成为允许原地覆盖内容的引用。
  package升级必须重新link/build consumer。
- service requirement在consumer compile/link时只绑定`serviceId + exact packageVersion label +
  expectedProtocolIdentity`，不绑定provider package或deployment revision。
- assembly projection为每个service requirement选择一个精确`ServiceDeployment` revision及其不可变
  implementation `PackageArtifactId`。service owner可以在protocol identity不变时发布并激活新的deployment
  revision，不要求consumer重新编译；已经生成的RuntimeAssembly仍记录原选择，不能随pointer漂移。

因此“service实现可更新”表示同一service id/version label与service API identity下的active deployment
pointer可以切换到新的deployment revision，不表示ServiceContract或任何不可变artifact可以被覆盖。
API内容变化会产生新的service API identity；旧consumer不能仅凭相同人类版本label被静默迁移。

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

canonical runtime connection上的actor、spawn及后续同类跨request control frame必须显式携带当前
ActivationIdentity，至少包含assembly identity、generation、runtime replica与deployment revision。Runtime
从发起控制动作的当前ActivationContext填充该identity；callback、continuation或spawn source不能重建、删减
或用ambient connection state替代它。

`runtime.register.serviceProtocolIdentity`必须原样携带canonical
`skiff-service-protocol-v2:sha256:<64 lowercase hex>`。register frame不得再包含`protocolVersion`：runtime
transport版本只由frame的`schemaVersion`（当前为`skiff-runtime-frame-v1`）表达，禁止从
ServiceProtocolIdentity前缀推导、复制或兼容读取第二份runtime protocol版本。

Router先把frame绑定到发送者的exact assembly registration，再按该registration对应的active或draining
generation snapshot验证完整identity。active generation可发起新控制动作；仍被request、stream、WebSocket或
callback显式pin住的draining generation只在其pin生命周期内继续使用原ActivationContext。未注册sender、
identity缺失/歧义、tuple不匹配、generation已完成drain或仅有legacy `runtime.register`/serviceId事实时一律
fail closed。actor/spawn response继续按同一request与sender correlation返回，不能在Router中恢复
service/build inference。

这能整体扩CPU、内存与副本可用性，但不能单独隔离或扩缩某个service；一个service的CPU/memory故障
可能影响同replica内其它service。第一版明确接受这一限制，并要求assembly admission、health、drain和
atomic reload在runtime层可观测。

Router与Runtime只通过共享artifact文件系统装载上述不可变记录，不依赖或感知registry service：

- Router配置唯一`artifactsPath`与`serviceDb.mongoUrl`。Runtime连接后，Router必须先发送一次连接级
  bootstrap control，其中包含规范化的绝对`artifactsPath`、`serviceDb: { mongoUrl }`和
  `http: { maxResponseBytes }`；Runtime在任何activation/register之前固定这些值，同一连接内缺失、重复
  冲突或变更一律fail closed；
- Router与Runtime可以位于不同机器，但该路径在所有机器上具有相同字符串和内容语义；当前生产部署以网络文件
  系统共享该路径；
- Router从该路径读取release/assembly routing projection，只持有请求路由、generation与activation协调所需
  的事实，不解析或链接package executable；
- Runtime从同一路径读取选定RuntimeAssembly及其精确PackageArtifact/ServiceDeployment闭包，完成link与加载；
- 路径中的immutable record必须先完整写入并校验identity，再原子更新pointer。Router reload只观察已经完成的
  pointer，不接受半写入record。

`artifactsPath`和`serviceDb.mongoUrl`都是部署拓扑配置，不进入PackageArtifact、ServiceContract、
ServiceDeployment或RuntimeAssembly identity。Runtime不得为二者另设独立文件配置、环境变量或默认值。
Runtime持有bootstrap DB transport binding不表示所有activation获得DB：只有声明并被deployment绑定DB
requirement的activation才能得到`std.db` capability，service代码始终看不到URL。

Router配置的HTTP形状固定为：

```text
http:
  port: positive integer
  maxRequestBytes: positive integer
  maxResponseBytes: positive integer
```

两项大小都是整个Router实例的必填规则，不存在隐藏默认值或per-service override。Router在读取完整request
body前执行`maxRequestBytes`；Runtime按bootstrap中的`maxResponseBytes`尽早停止生成过大response，Router在
外部HTTP边界再次校验。对HTTP streaming，`maxResponseBytes`按同一response生命周期累计，不能通过拆chunk
绕过。WebSocket不复用这两个字段。

未来若需要独立扩缩容，平台可以为不同root set生成多个assembly。届时assembly projection把当前完整
本地闭包拆成`LocalExecutableClosure`与`RemoteBindingRefs`；只有跨assembly service edge选择
`RemoteBoundary`，远端provider不进入本地code closure。ServiceContract与PackageArtifact不需要改变。

## 13. Registry、Release 与 Publish

registry分别存储不可变PackageArtifact、ServiceContract、ServiceDeployment与RuntimeAssembly record。
release pointer可以选择contract-compatible deployment revision和active assembly。

生产registry由可选的普通Skiff service `skiff.run/registry`实现。它和其它service一样首先是package，
源码root包含`package.yml`、`api.yml`、`service.yml`与`config.*.yml`，可以位于官方
`skiff-packages`仓库；其ServiceContract与ServiceDeployment均由tooling生成而非独立author。它不是
`skiff.run/std`、compiler platform source、语言intrinsic、native adapter或拥有compiler特权的package。
语言、compiler和runtime在没有该service时仍然完整可用。调用者通过普通typed ServiceContract调用registry；
compiler不得为`skiff.run/registry`保留package id、注入native declaration、授予特殊capability或要求外部
authoring descriptor。

Router和Runtime不知道registry service，也不通过它读取artifact。正式环境中，registry负责把已经验证和编译
完成的immutable records/materialized artifacts发布到部署配置的共享`artifactsPath`，并原子更新pointer；
开发环境由compiler相关CLI/tooling完成同一文件布局的编译与发布。当前阶段只冻结该owner和文件边界，不要求先
实现registry到共享路径的生产发布流程。

`skiff.run/registry`以Platform DB作为四类immutable record及typed release pointer current/history的唯一
production durable source of truth。它和其它需要数据库的service一样声明DB/state requirement，并只通过
普通`std.db` capability访问数据库。Mongo URL的唯一配置owner是Router的`serviceDb.mongoUrl`；该值不进入
service/package/compiler/deployment artifact，也不由runtime文件配置、环境变量或默认值提供。Router在
连接级bootstrap中把DB transport binding与`artifactsPath`一并下发给Runtime；Runtime只为当前activation中
已经声明并绑定的DB requirement建立activation-scoped capability，service代码看不到provider URL。文件型
`CanonicalArtifactStore`只作为
local/dev/CLI backend，不参与production registry，也不与Platform DB dual-write。

Router coordinator仍是environment activation prepare/commit/abort的唯一事务编排者。Router进程直接使用
自己配置的MongoDB连接持久化activation state；状态CAS与Platform audit在同一事务中追加。不得为了复用其它
语言实现而要求外部activation backend executable、子进程或NDJSON transport。registry service不能直接写
prepared/connected集合、伪造participant ACK或维护第二份activation state；它与Router activation state通过
明确的collection/schema owner隔离。

`publish`是操作：校验typed artifact、写入不可变内容、更新允许更新的pointer。它不产生
`Publication`对象，也不要求四类artifact实现共同kind enum。registry可以在一个事务/workflow中发布
多个artifact，但每个artifact保持独立schema与identity。

## 14. Fail-closed 条件

以下情况必须在compile、deployment projection或assembly阶段失败，不能留到请求时猜测：

- service API schema不闭合或operation identity冲突；
- 自动生成的deployment operation缺失、重复、额外或descriptor不匹配；
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
