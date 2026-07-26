# Package、Service Contract 与 Deployment 架构

本文定义Skiff长期目标中的代码编译、service协议、deployment装配与runtime执行边界。它是
compiler、artifact、runtime、router和registry共同遵守的canonical架构契约，不是实现计划，也不
冻结最终YAML字段名或CLI拼写。

Skiff尚未发布。实现应直接收敛到本文模型，不为旧service源码、旧共同发布对象artifact或旧
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

`PackageSchemaTypeRecord`与`PackageSchemaIndex`是PackageArtifact拥有并按内容寻址的schema子记录，用于
逐类型去重、精确引用和枚举本build的schema surface；它们没有独立authoring、release pointer、version
selector或deployment lifecycle，因此不是第五个领域对象。

它们没有共同aggregate。可以共享canonical type、signature、operation、value-plan和identity
framing等叶子类型，但不能用共享DTO重新制造隐式父模型。

## 2. 不变量

以下约束是实现和演进的硬边界：

1. Package是唯一用户源码与独立编译单元；service首先是package，只比普通package多`service.yml`和
   `config.*.yml`。
2. 普通package root包含`.skiff`、`package.yml`和`api.yml`；service root在此基础上增加
   `service.yml`与零个或多个`config.*.yml`。service root不得缺少`package.yml`，也不存在开发者维护的
   `deployment.yml`。`api.yml`也不得省略；没有公开API时必须显式写为`{}`。
3. `api.yml`是package call与service-to-service call共用的公开API owner；HTTP、WebSocket等外部入口
   由`service.yml`拥有。外部handler不因成为ingress而进入`api.yml`，也不因此对其它service可调用。
4. `api.yml`中的`serviceCall: true`只标记service-to-service callable roots，不重复列type；未标记
   callable只是Package API。ServiceContract只由compiler/tooling从这些显式roots及其typed
   PackageSchema closure确定性投影；
   consumer只依赖发布后的code-free projection，不读取provider实现源码或外部ingress。
5. ServiceDeployment不解析AST、不重新做type/effect分析；它由工具消费typed PackageArtifact、
   ServiceContract、compiler已经形成的typed ingress projection、`service.yml`和所选
   `config.*.yml`生成。
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
13. 所有boundary命名类型由Package拥有；ServiceContract只能引用PackageSchemaTypeId，不能复制descriptor、
    展开为anonymous root或生成service-owned nominal identity。

## 3. Package 与 PackageArtifact

Package source由`.skiff`源码、`package.yml`、`api.yml`和静态资源组成。`package.yml`拥有package的人类
可读`id`/`version`、package dependencies与service dependencies；两类dependency alias共享同一namespace，
版本selector第一版都必须精确。`version`只用于人类展示和解析坐标，不参与任何artifact、ABI或内容identity
运算。

Service仍走同一个package compiler入口；存在`service.yml`时，compiler/tooling再执行service projection。
`service.yml`拥有service id与HTTP/WebSocket等外部ingress。HTTP包含route、handler/pre/guard source
selector、adapter参数来源及外部协议metadata；WebSocket当前只冻结连接path与可选connect callback，业务
消息handler部分待设计。External handler selector指向当前service package中的普通source callable，
不要求该callable出现在`api.yml`。`service.yml`不含version、dependency、service-call API type/function
映射、与handler类型重复的业务JSON schema、实现artifact binding、平台组织角色或request/response大小
策略。External schema和runtime codec plan由compiler从精确linked handler signature、adapter kind与参数
来源确定性生成。`config.*.yml`只绑定已经声明的
config/secret/state/resource requirement，不改变package/service dependency graph。

Authoring层不要求开发者分别维护entry表与route表，也没有只起分类作用的`routes`中间层。已冻结的HTTP
写法中，`http`本身就是以稳定名字为key的entry mapping；每个value把external selector与该entry的
handler/adapter声明写在一起。Mapping key就是service-owner-local `GatewayEntryKey`。Compiler必须把
这一个HTTP authoring record确定性拆成`IngressSelector -> GatewayEntryKey`和
`GatewayEntryKey -> resolved gateway entry`两个artifact事实。第一版一个HTTP authoring route只定义
一个entry，不提供多个selector复用同一entry定义的别名/引用语法。`guard`/`pre`属于具体HTTP entry，
不占用`http`下的保留key，也没有隐式全局继承。

HTTP entry kind决定external response协议，不能只按handler的`Stream<T>`外形推断。`typedJson`只允许
unary handler return；runtime wrapper把该单个值编码为一次JSON response，compiler必须拒绝
`typedJson`与任意`Stream<T>`返回组合。`rawHttp`允许返回单个`std.http.HttpResponse`，也允许精确返回
`Stream<std.http.HttpResponseStreamEvent>`；后一种是第一版唯一的external HTTP server-stream surface，
由handler显式控制status、headers和后续body chunks。Compiler不得把raw HTTP stream改投影成typed JSON
chunks，也不得要求external caller理解Skiff的`Stream<T>`类型。

`websocket`仍由`service.yml`拥有，连接path和可选`connect`回调也属于这里；但业务消息入口的authoring与
identity层级尚未冻结。Raw frame `receive`是平台transport阶段，不是与HTTP业务handler对等的service
入口，目标设计不得把单一用户`receive`回调当成整个WebSocket业务API。后续必须先定义平台如何从frame
得到业务消息selector、如何选择typed message handler，以及unknown/binary/error策略，再决定
WebSocket connection key与嵌套message entry key如何进入artifact。该设计冻结前，不从HTTP写法类推或
实现新的WebSocket `receive` authoring。

PackageArtifact至少包含：

```text
PackageArtifact
  packageId / packageVersion / packageBuildId
  FileIrUnit refs
  PackageLocalAbi
  PackageSchemaIndex ref
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

所有能进入package或service边界的命名类型都由Package拥有，Service不重新拥有或复制一套类型。
Package从public API graph和其schema closure生成逐类型content-addressed record，并为当前artifact生成
一个index：

```text
PackageSchemaTypeRecord
  packageId
  stableSchemaKey
  packageSchemaTypeId
  canonicalDescriptor

PackageSchemaIndex
  packageId
  packageSchemaIndexIdentity
  types: stableSchemaKey -> {
    packageSchemaTypeId
    publicPath?
    nameability
  }
```

第一版所有进入package/service-call boundary closure的命名类型都必须在owner Package的`api.yml`中显式公开，
`stableSchemaKey`就是其canonical public API path。未公开的内部命名类型不能作为operation参数、返回值、
字段闭包或其它boundary payload；compiler必须fail closed，不能用源码文件路径、模块路径、遍历顺序、
display string或某个ServiceContract的发现路径生成隐藏稳定键。匿名discriminator branch不是独立named
type，没有自己的key；它只作为owner named union descriptor的一部分。

同一个source nominal declaration第一版只能拥有一个canonical public path；record、representation、union
或interface被`api.yml`重复绑定到多个public path必须fail closed。function可以显式拥有多个public path；
每个path是独立callable surface并拥有独立public callable identity。

外部HTTP/WebSocket ingress不是service-call boundary。其handler参数与返回值由compiler保存的linked
callable signature和专用gateway adapter plan编解码，不要求内部业务类型为了external ingress而进入
`api.yml`或PackageSchema。对外文档所需的JSON schema是entry-local协议描述，不是Skiff名义类型identity，
也不能反向成为runtime binary codec的事实源。

Compiler只对adapter实际映射到external source/sink的值计算entry-local schema closure。已冻结部分包括
HTTP body、query/path/header参数与HTTP response。未来typed WebSocket业务消息handler的输入/输出也应形成
外部wire shape，但其消息路由模型尚未冻结，当前不得据raw `receive`回调提前投影。Pre/guard内部context、
WebSocket connection context及其它只在runtime adapter与handler之间流动的值不进入该closure。
私有named type可以贡献外部shape，但其source name、module path与Skiff nominal identity不得泄露为public
type；只改私有名字而保持canonical external shape不改变`GatewayEntryIdentity`。

第一版Package boundary schema graph必须无递归环；用户递归record本来就不是SchemaClosed。projection在计算
identity前对所有named-type引用建图并拒绝self-cycle或SCC。随后按拓扑序计算
`PackageSchemaTypeId = hash(packageId, stableSchemaKey, canonicalDescriptor)`；descriptor中的named child
引用包含child的`packageId + stableSchemaKey + PackageSchemaTypeId`，因此不存在循环哈希。package version
label、PackageBuildId、service id、nameability、publicPath和deployment信息都不参与逐类型identity。

`PackageSchemaIndexIdentity`只标识某个PackageArtifact的完整schema目录，其canonical preimage是packageId加
按stableSchemaKey排序的`(stableSchemaKey, PackageSchemaTypeId, publicPath?, nameability)`列表。它可因无关
类型、公开路径或nameability变化而改变，但不进入ServiceProtocolIdentity。第一版index中的boundary
named types都必须是`PublicNameable`；`ClosureOnly`保留为未来模型枚举值，当前projection不得生成。

同一类型内容在不同Package build中不重复生成新的类型身份；Artifact store可以按
`PackageSchemaTypeId`去重和解析单个type record，并按`PackageSchemaIndexIdentity`去重index。
PackageArtifact引用index，而不是把同一份类型定义复制到每个ServiceContract。类型内容不变时，
implementation build和人类version可以变化而不改变类型identity；canonical descriptor变化必然产生新的
类型identity。

File IR executable signature与PackageLocalAbi不是同一个type owner。File IR只保存本地执行需要的
execution type representation。Package API与Service API必须共用同一套parser、name resolution、nominal
type、field/constructor、generic、interface conformance和typed expression机制；不得从ServiceContract
descriptor再建立一套只支持签名、不支持普通表达式的contract type system。

Service dependency导入的code-free API module是同一canonical Package API representation的materialized view。
其中operation由ServiceContract选择，类型仍引用其owner Package的`PackageSchemaTypeId`，不迁移为service-owned
类型，也不绑定provider implementation build。compiler不得用display string、结构相同的临时local type、
JSON encode/decode、匿名record展开或手写wrapper冒充相同identity。

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

普通package允许同时拥有Available和Unavailable public functions。未带`serviceCall: true`的callable无论
Available或Unavailable都只是Package API。存在`service.yml`时，只有显式带marker且Available的public
function或public-instance method成为service operation；显式marker但Unavailable必须让service projection
以完整结构化原因失败，不能静默排除。公开type、alias或interface也可以只服务package linkage而没有合法
PackageSchema投影；generic declaration等不能进入service-call schema的public symbol必须保留在
PackageLocalAbi，并以结构化boundary-unavailable事实阻止相关service operation，不能让整个Package因一个
未被service-call使用的公开generic declaration失败。compiler/tooling必须输出完整、稳定、可机器读取的
列表及结构化原因；构建摘要、CLI/JSON、artifact receipt与IDE应消费同一projection，不能静默排除。

`serviceCall: true`是第一版唯一service-call选择机制，只能标在function leaf或public-instance leaf。
它不复制参数、返回或错误类型；compiler从标记的callable signature递归闭合已在`api.yml`拥有canonical
public path的PackageSchema types。普通Package出现marker必须报错；`service.yml`不维护第二份
include/exclude清单。public instance标记一次表示其显式listed interfaces的全部methods；若只需部分methods，
作者必须使用更窄interface或wrapper function。

`service.yml`引用的每个external handler另行生成typed ingress projection。它可以引用非public callable和
非public named type，但必须有完整linked signature、精确PackageCallableId、合法adapter plan以及可执行的
external codec。Ingress availability与service-call availability分开报告；不能通过把handler补进
`api.yml`来绕过ingress校验。第一版handler/pre/guard不能是generic function declaration；其concrete
signature可以引用fully instantiated generic platform types。

缺字段不表示不可用或尚未分析。PackageArtifact必须保存完成boundary判断所需的typed effect、
provenance和link facts，使deployment无需读取源码。

`BoundaryOperationContract`只承载boundary可观察的signature、stream/cancel/callback、value plan与
公开effect保证；其中所有命名类型引用都保留`PackageSchemaTypeId`及其package owner。ServiceContract
projection在该body外增加稳定operation key/id，不把Package类型重写成`ContractTypeId`。service operation
统一拥有§6.3定义的开放错误通道，不在operation contract中列出可能抛出的类型集合。具体
config/state/native capability requirement和完整may-effect属于
`BoundaryImplementationRequirements`，不能泄漏进ServiceProtocolIdentity。

同一个PackageArtifact可以同时：

- 被其它package直接链接；
- 在存在`service.yml`时从显式service-call roots生成一个ServiceContract并实现其全部operations；
- 为同一个service生成不进入ServiceContract的typed external ingress entries；
- 被多个ServiceDeployment revision复用；
- 在同一assembly内只链接一份代码，由多个activation context调用。

## 4. ServiceContract

ServiceContract是独立发布、无代码的service-to-service typed API projection artifact。它的唯一
operation authoring source是service package自己的`.skiff` declarations与`api.yml`中显式
`serviceCall: true` roots。`service.yml`只向这项projection提供service id；不存在独立contract
YAML/IDL、类型映射或第二套service-call函数清单。ServiceContract不引用provider
build、config、runtime replica或HTTP/WebSocket ingress。

```text
ServiceContract
  serviceId
  packageVersionLabel
  serviceProtocolIdentity
  operations: name/id -> BoundaryOperationDescriptor
  packageSchemaRequirements
```

每个operation descriptor包含canonical参数、返回、stream、cancel、callback与value plan契约，并统一使用
开放错误通道；它不包含operation-specific throw set。ServiceContract不拥有第二套boundary类型，不内嵌或
复制Package字段定义。它记录精确
`PackageTypeRequirement`：

```text
PackageTypeRequirement
  packageId
  requiredTypeIds: PackageSchemaTypeId[]
```

Contract closure由operations引用的Package类型及各`PackageSchemaTypeRecord`描述的传递闭包组成。
tooling必须只按`PackageSchemaTypeId`读取content-addressed type record并闭合、校验；consumer不读取provider
源码，也不按当前active deployment或任意同version PackageArtifact猜测类型。不同PackageSchemaIndex只要
解析到相同type id就是同一类型，不需要整包index identity相等。缺type record、owner/key不匹配、descriptor
重新计算出的identity不匹配或闭包不完整都fail closed。

Service package自己声明的类型与依赖package声明的类型遵守同一规则：都由各自Package拥有。ServiceContract
只拥有service operation集合及其协议身份，不拥有`ServiceType`、service-owned `ContractTypeId`或类型映射层。

External ingress不进入`operations`或`packageSchemaRequirements`。增加、删除或修改HTTP/WebSocket route、
handler、adapterArgs、external JSON schema或gateway policy不得改变`ServiceProtocolIdentity`；只有
service-call API及其Package schema closure变化才改变该identity。

Service API identity由canonical boundary surface内容确定；`package.yml.version`只作为人类可读、精确解析
label，不参与identity运算，`service.yml`没有version。新implementation build可以在identity不变时替换当前
active revision而无需consumer同意；API内容变化必然产生新identity，即使作者未修改version label也不能
静默绑定旧consumer。

operation引用的任一`PackageSchemaTypeId`或其闭包变化都属于API内容变化。反之，只要operation与所有引用
类型identity不变，Package内部实现、无关public function、未被该contract引用的Package类型或version label
变化都不能机械改变ServiceProtocolIdentity。

第一版`package.yml.services`形成的service dependency graph必须是DAG。Compiler/tooling按拓扑顺序解析已生成
的精确ServiceContract并编译consumer；同一编译批次或已解析dependency closure中出现环必须在编译
Package body前fail closed，不得隐式启动跨Package的两阶段全局源码批编译。需要反向调用时使用显式
callback capability、actor或重新划分Service边界，而不是制造静态service dependency cycle。

## 5. ServiceDeployment

ServiceDeployment是无源码的生成artifact，不是开发者维护的`deployment.yml`：

```text
ServiceDeployment
  serviceId / packageVersionLabel / expectedProtocolIdentity
  deploymentRevision
  implementation PackageArtifact ref
  operationBindings: contractOperationId -> packageCallableId
  dependencyBindings
  gatewayEntries: gatewayEntryKey -> {
    gatewayEntryIdentity
    protocol
    handler/pre/guard packageCallableId
    typed adapter plan
    external protocol metadata
  }
  ingress: externalSelector -> gatewayEntryKey
  config/secrets bindings
  state/DB/actor/queue ownership
  timeout/resource/activation policy
```

operation mapping由同一service package的ServiceContract projection与PackageArtifact public callable
identity确定性生成。只有显式service-call roots进入；不得要求开发者在`service.yml`或`deployment.yml`
重复映射。生成artifact必须写入稳定callable id，runtime禁止按display name猜target。

Ingress不绑定`ContractOperationId`，也不进入ServiceContract。Compiler从`service.yml`的source selector
解析当前implementation中的精确`PackageCallableId`，校验handler signature与adapter source，生成typed
gateway entry并计算`GatewayEntryIdentity`。`gatewayEntryKey`只是`service.yml`内稳定、owner-local的entry
键，使同一协议identity可以绑定不同implementation或被多个selector复用；它不是内容identity。
Deployment只消费该typed projection并把external selector绑定到gateway entry；
Router和Runtime不得按source path、display name或同名service operation猜handler。

Artifact模型允许多个selector绑定同一个key，但第一版`service.yml`不暴露独立entry引用或复用语法：
named route同时声明唯一selector与entry definition。该限制只简化authoring，不得把selector并入
`GatewayEntryIdentity`，也不得让Router跳过上述两步查找。

`GatewayEntryIdentity`只标识external protocol surface。已冻结的HTTP canonical preimage覆盖entry kind、
外部request/response/stream shape、HTTP adapter source映射、公开错误投影及其它会改变gateway wire
兼容性的metadata。WebSocket identity必须在业务消息入口层级冻结后另行补齐；不能只对`connect/receive`
两个transport回调做hash并把它误当成业务协议identity。Identity不包含source selector、
handler/pre/guard `PackageCallableId`、内部名义类型identity、PackageArtifact/build或deployment policy。
Compiler仍必须验证由linked handler signature导出的external schema和typed adapter plan与该surface逐项
一致。HTTP stream mode只能来自`rawHttp`的精确
`Stream<std.http.HttpResponseStreamEvent>`返回；`typedJson` identity始终是unary，不能生成或接受
server-stream shape。

具体handler/pre/guard callable、完整typed adapter execution plan、implementation artifact、external
selector和policy只由ServiceDeployment及其revision覆盖。只替换实现且external protocol不变时，
`GatewayEntryIdentity`保持不变而deployment revision改变；改变external wire surface时两者都改变。

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
- qualified alias只选择typed dependency，不进入PackageSchemaTypeId/ContractOperationId本体，也不能从provider
  package、deployment或display name补事实。

依赖种类只来自`package.yml`的validated `packages`/`services` entry，不由call syntax、物理local/remote
binding或运行时猜测。

`dependencyBindings`只表达当前deployment对implementation package requirements的provider selector/约束，
不拥有全局解析结果。RuntimeAssembly projection负责在root set及闭包中解析唯一provider、验证闭包并生成
每个ActivationContext的binding vector。

deployment validation必须保证：

- 每个由显式service-call root生成的operation恰好映射到其source public callable；
- 不存在手工增加、遗漏或重复operation；
- target callable的boundary projection是`Available`；
- operation descriptor、schema closure与同一canonical API projection逐项精确匹配；
- implementation may-effect满足contract公开effect保证，且所有implementation requirements得到binding；
- 每个`service.yml` ingress selector恰好绑定一个canonical gateway entry；
- gateway entry中的handler/pre/guard全部解析到当前implementation的精确callable，adapterArgs与其linked
  signature逐项匹配，且不会被加入ServiceContract；
- gateway entry protocol identity与deployment binding/revision分别覆盖各自规定的全部事实，互不吞并；
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

### 6.3 Service error channel 与异常栈

语言不在函数签名、Package Local ABI或ServiceContract operation中声明、推导或发布可能抛出的类型集合。
每个service operation都有同一个开放错误通道；实现新增一种可能抛出的错误，不改变operation signature或
`ServiceProtocolIdentity`。预期调用方穷尽处理的业务失败仍应建模为普通返回union。

任意用户`type`声明的名义值都可以在request内被抛出，不因此自动获得boundary schema。跨service时传输的是
错误payload和固定错误envelope，不是request-local的`Exception<E>`对象：

```text
ServiceErrorEnvelope
  = PublicTypedError {
      packageId
      stableSchemaKey
      packageSchemaTypeId
      encodedPayload
      traceId
      errorId
    }
  | InternalError {
      payload: std.service.InternalError
    }
  | PlatformError {
      builtinErrorIdentity
      encodedPayload
      traceId
      errorId
    }
```

只有同时满足以下条件的用户错误才能以原始名义类型进入`PublicTypedError`：

- 实际concrete错误类型在其owner Package的`api.yml`中显式公开且`PublicNameable`；
- 该类型的完整字段闭包满足`SchemaClosed`；
- runtime能够按owner提供的`PackageSchemaTypeId`成功编码实际值。

错误可以由service package或其任意dependency package声明；公开性、schema和identity始终在类型自己的
Package owner中判断，不能把外部类型改写成throwing service拥有的类型。接收方链接了同一
`PackageSchemaTypeId`时，runtime恢复原名义值，普通`catch<ownerAlias.Error>`即可匹配。接收方没有链接该
类型时，可以把已编码的公开错误envelope作为不可匹配的异常因果继续向外传播；runtime不得按结构猜类型，
也不得为转发而要求中间service声明operation throw set。

私有、不可name、非`SchemaClosed`或编码失败的用户错误不得发送原type identity、字段或显示字符串。
callee在第一次越过service boundary时把它替换为固定、公开且schema-closed的
`std.service.InternalError`。该错误带稳定脱敏message以及`traceId`、`errorId`，因此后续service可以像处理
其它普通错误一样捕获；未捕获时直接继续发送同一个错误payload和关联identity，不反复包装成新的错误类型。

所有错误值都由当前request的`Exception<E>`承载，因而`std.service.InternalError`也一定有source location和
stack trace。普通`throw`创建当前request的异常栈；同一request中的`rethrow`保留原envelope与throw site。
service response只传输上面的错误envelope，不把callee的request-local`Exception<E>`对象或原始私有栈当作
业务payload序列化。caller在service call site恢复为新的本地exception envelope，生成caller这一跳的栈并
附加一帧脱敏remote-boundary信息。若B调用A后不捕获错误，B对外继续发送同一个错误payload；B的caller再为
自己这一跳得到新的异常栈。

每次最初throw、私有错误转换和跨service传播都保留同一因果`traceId`，并以`errorId`关联。每个service的完整
本地栈进入受限telemetry/log；跨边界只暴露service/operation/errorId等脱敏诊断，不暴露私有源码路径、函数名
或原始错误字段。`InProcessBoundary`必须执行同样的编码、identity、转换和新栈语义，不能因为共享进程而泄漏
本地引用或callee栈。

“可抛出”与“可序列化”是两个独立性质：package内部throw不要求`SchemaClosed`；只有希望跨service后保留原始
名义类型的公开错误payload需要`SchemaClosed`。`std.service.InternalError`和固定platform error envelope本身
始终可序列化。记录日志不要求序列化用户错误payload。

### 6.4 External ingress

HTTP、WebSocket及未来其它gateway entry不是service boundary call。外部请求按
`IngressSelector -> GatewayEntryKey -> GatewayEntryIdentity`进入当前activation，再由deployment
gateway entry binding中冻结的精确
`PackageCallableId`执行handler；它不经过service dependency slot，也不伪造`ContractOperationId`。

Ingress仍复用普通语言函数、Package本地链接、ActivationContext、错误通道和结构化取消，但不复用
ServiceContract作为对外声明：

- handler/pre/guard只需由`service.yml`显式引用，不需要出现在`api.yml`；
- ingress callable不会出现在service dependency的code-free API module中；
- handler参数和返回值的runtime codec来自linked callable signature及typed adapter plan；
- gateway只持有route、adapter metadata和opaque payload bytes，不解析业务类型；
- external JSON schema、HTTP status/header规则和WebSocket connection metadata属于gateway entry，
  不进入PackageSchema或ServiceProtocolIdentity；
- ingress抛出的错误可复用固定service error carrier交给gateway做脱敏投影，但外部caller不会因此成为一个
  可`catch<E>`的Skiff service caller。

同一个source function可以被作者分别列入`api.yml`和`service.yml`，但这是两个显式surface：
`api.yml`中的function leaf只有显式带`serviceCall: true`才生成service-call operation，
`service.yml`中的引用生成external gateway entry。Compiler必须分别验证并生成不同identity，不能因source
target相同而把两者合并或互相推断。

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
mutation不能穿过service boundary。这三类事实必须分别建模：

- `writesCallerReachable`只表示callee会修改caller传入的引用图；
- `returnsCallerAlias` / `throwsCallerAlias`只表示返回值或throw payload仍引用caller图；
- `requiresSameHeapIdentity`只表示计算已经对caller-reachable引用执行了引用身份敏感操作，例如对
  heap value执行`==` / `!=`，或调用语义明确等价的identity intrinsic。

读取collection元素、投影字段、返回alias、把值写入caller对象、把值装进fresh对象或创建callback
capability本身都不是same-heap identity observation；它们只能产生各自的provenance、write、escape或
callback事实。unknown target由独立unknown/effect事实失败关闭，不能为了拒绝unknown而伪造一个已经发生的
same-heap identity observation。

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
- 是否对caller-reachable引用执行了引用身份敏感操作；
- callback/native adapter requirement；
- unknown call/effect。

分析允许保守拒绝，不允许漏掉boundary-visible行为。mutable helper、返回参数alias的函数和依赖本地
identity的算法仍是合法package API，但deployment选择它们时以各自独立的结构化原因失败。只要public
callable可能对boundary输入执行引用身份比较，它就不能成为service operation；service materialization
不得尝试保留caller heap identity来放宽该规则。

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

PackageTypeRequirement
  packageId + requiredTypeIds
```

PackageRequirement在link阶段解析为不可变PackageArtifact。ServiceRequirement允许package解析发布后的
service API types和operation signatures，但不要求provider implementation；只有实际service call sites产生
runtime binding slot。它不包含provider package、provider build、deployment revision或runtime route；
最终assembly只为ServiceRequirement选择deployment。

必须分开的identity：

- PackageId / PackageVersion：人类可读代码发布坐标；version不参与任何identity hash。
- PackageBuildId：具体不可变代码build。
- PackageLocalAbiIdentity：local public code ABI。
- PackageSchemaIndexIdentity：某个PackageArtifact完整schema目录的内容身份；不进入service protocol。
- PackageSchemaTypeId：Package拥有的单个boundary类型内容身份；version和service id不参与。
- ServiceId / exact PackageVersion label：consumer依赖坐标；service.yml不重复version。
- ServiceProtocolIdentity：canonical boundary surface内容身份。
- DeploymentRevision / DeploymentArtifactIdentity：某次implementation、配置与route revision。
- AssemblyIdentity：完整resolved deployment/package graph。
- RuntimeReplicaId：某个assembly实例，不进入artifact contract。

任何identity都不能因为display string相同而互换。ServiceProtocolIdentity包含operation实际引用的
PackageSchemaTypeId/closure identity，但不包含provider implementation build或deployment字段；
AssemblyIdentity可以记录最终选择的build作为复现事实，但不能回写consumer requirement。

## 11. Config、State 与 Resource Owner

Package可以声明运行所需config path、外部resource capability、DB/schema或native adapter requirement，
但不拥有环境中的实际值和state namespace。普通package可以在`package.yml.services`声明service
dependency；这使其可复用业务编排在最终宿主service的ActivationContext中解析provider，不把具体provider
写入PackageArtifact。

Service source的`config.*.yml`选择或提供：

- 提供config/secrets；
- 选择DB、Redis、actor、queue等外部state namespace；
- 定义timeout、quota、principal与lifecycle policy。

`timeout`是可选的deployment override。profile缺省或显式`null`都表示不覆盖平台/外层request
deadline；生成的`DeploymentPolicy`不包含`timeoutMs`。只有显式的正整数毫秒值才生成
`timeoutMs`，零、负数、小数、字符串或对象都必须fail closed。tooling不得为了通过artifact校验而
填入虚假的默认timeout。External HTTP中，Router以平台HTTP request上限和该override的较小值生成
request deadline；Host从已admit activation读取同一policy并再次收紧、执行。Deployment override只能
缩短平台/外层deadline，不能放宽，也不能因wire遗漏或伪造而失效。

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

第一版的完整assembly-per-environment、replica隔离、ActivationIdentity/generation、Router/Runtime
bootstrap、共享artifact filesystem、HTTP实例限制与未来多assembly扩展属于部署/runtime拓扑，不是
Package/Service语言对象。完整契约见
[`runtime-deployment-topology.md`](runtime-deployment-topology.md)。本节只冻结：RuntimeAssembly记录精确
不可变选择，已生成assembly不随pointer漂移；任何拓扑都不得改变Package direct call、service boundary与
external gateway entry三种语义。

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

State requirement由package代码owner在`package.yml`中以`state.<requirement-key>.kind`声明，并进入
`PackageArtifact.runtimeRequirements.state`；database requirement必须与同一次package lowering产生的
DB schema事实精确对应。物理`namespace`不属于package声明，只能由`config.<environment>.yml`中同key、同kind
的deployment binding提供。Compiler、deployment或Runtime都不得从package/service名称、固定key或namespace
反推state requirement。

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
- `service.yml` ingress handler/pre/guard无法解析到当前Package callable，或adapterArgs与linked signature
  不匹配；
- ingress仍指向`ContractOperationId`、要求handler先进入`api.yml`，或gateway entry identity与typed
  projection不一致；
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
