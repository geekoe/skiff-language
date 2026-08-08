# Package、Service Contract 与 Deployment 架构

本文定义Skiff长期目标中的代码编译、service协议、deployment装配与runtime执行边界。它是
compiler、artifact、runtime、router和registry共同遵守的canonical架构契约，不是实现计划。本文冻结
manifest owner与跨层数据流；精确YAML shape由`../reference/service-yml.md`冻结，CLI拼写不在本文定义。

Skiff尚未发布。实现应直接收敛到本文模型，不为旧service源码、旧共同发布对象artifact或旧
compiler pipeline保留兼容层。

## 1. 核心结论

目标模型只有三个canonical发布artifact：

```text
PackageArtifact     package源码的不可变编译产物
ServiceContract     service package公开API的code-free boundary projection
ServiceDeployment   工具由service package生成的不可变deployment记录
```

若publish/verify/promotion需要聚合一批immutable refs，可写入可选`ReleaseBundle`：它只聚合exact
`ServiceDeploymentRef`与verification receipt ref/receipt；`BundleIdentity`从这些事实确定性计算。Bundle不是
runtime load、routing或request单位，也不拥有linked executable。

业务配置在发布时冻结为ServiceDeployment-owned protected payload；deployment保存不可替换的opaque ref，
该ref参与buildId。配置没有独立selector或发布生命周期。

`Publication`不是领域对象、共同父类型、artifact kind或compiler pipeline。`publish`只允许作为
registry/release写入不可变artifact与更新pointer的动作名称。

三个artifact分别回答不同问题：

- PackageArtifact：有哪些代码，如何在同一`DeploymentExecutionImage`内直接调用。
- ServiceContract：`service.yml`从同一个package公开API中选择哪些callable作为service调用，以及跨boundary
  的语言语义。
- ServiceDeployment：工具把哪个service package build及生成的operation/gateway routing装配为一次不可变
  运行revision，同时冻结该build看到的配置与deployment-owned capability descriptor。

可选`ReleaseBundle`只回答“这次发布/验证聚合了哪些immutable refs与receipts”。Runtime以release pointer
解析出的deployment buildId为唯一加载单位，并只执行由该exact build构造的immutable
`DeploymentExecutionImage`。

`PackageSchemaTypeRecord`与`PackageSchemaIndex`是PackageArtifact拥有并按内容寻址的schema子记录，用于
逐类型去重、精确引用和枚举本build的schema surface；它们没有独立authoring、release pointer、version
selector或deployment lifecycle，因此不是额外的领域对象。

三类artifact与这些schema子记录没有共同aggregate。可以共享canonical type、signature、operation、value-plan和identity
framing等叶子类型，但不能用共享DTO重新制造隐式父模型。

## 2. 不变量

以下约束是实现和演进的硬边界：

1. Package是唯一用户源码与独立编译单元；service首先是package，只比普通package多`service.yml`、
   可选`http.yml`/`websocket.yml`和`config.*.yml`。
2. 普通package root包含`.skiff`、`package.yml`和`api.yml`；service root在此基础上增加
   `service.yml`、可选`http.yml`/`websocket.yml`与零个或多个`config.*.yml`。`http.yml`或
   `websocket.yml`不能脱离`service.yml`单独出现。service root不得缺少`package.yml`，也不存在开发者
   维护的`deployment.yml`。`api.yml`也不得省略；没有公开API时必须显式写为`{}`。
3. `api.yml`是package call与service-to-service call共用的公开API owner；`service.yml.serviceCalls`
   只按public path选择已有callable roots。HTTP与WebSocket外部入口分别由`http.yml`和`websocket.yml`
   拥有；`service.yml`不得再内联这两类配置。外部handler不因成为ingress而进入`api.yml`，也不因此对
   其它service可调用。
4. `service.yml.serviceCalls`不重复source selector、signature或type；未选择的callable只是Package API。
   ServiceContract只由compiler/tooling从这些显式roots及其typed
   PackageSchema closure确定性投影；
   consumer只依赖发布后的code-free projection，不读取provider实现源码或外部ingress。
5. ServiceDeployment不解析AST、不重新做type/effect分析；它由工具消费typed PackageArtifact、
   ServiceContract、compiler已经形成的typed ingress projection、`service.yml`、可选
   `http.yml`/`websocket.yml`和所选profile配置生成。Deployment以owned protected payload ref冻结配置，
   不另建可独立选择的配置对象。
6. package call与service call是不同语义；物理同进程不允许把service call退化成普通package call。
7. Service dependency slot只冻结service coordinate、expected protocol和operation事实。每次boundary
   invocation开始时解析provider release pointer并pin exact provider buildId；同进程可用VM child-fiber
   trampoline优化，跨进程走runtime transport，二者保持相同boundary语义。
8. 普通即时service call只要求linkable；跨request或持久边界才要求recoverable。
9. runtime replica按buildId独立懒加载immutable `DeploymentExecutionImage`；replica之间heap、CPU调度和
   lifecycle独立，外部数据层按deployment配置共享。
10. code identity、service API identity、deployment revision/buildId与可选bundle identity必须分开；任何
    人类可读`version`都不参与内容identity计算。
11. 每个request-scoped `DeploymentExecutionContext`必须随fiber、stream和callback显式传播。service child
    切换到exact provider build owner；callback切回capability记录的exact owner。普通continuation不得从
    ambient context补owner事实。
12. actor、dispatch及其它跨request control必须携带其契约要求的exact deployment build/actor
    implementation owner。Router按buildId registration或lazy-load capability验证；不能按display name、
    ambient service或已退役的全局release state补事实。
13. 所有boundary命名类型由Package拥有；ServiceContract只能引用PackageSchemaTypeId，不能复制descriptor、
    展开为anonymous root或生成service-owned nominal identity。
14. concrete executable的suspension summary由body、调用图和内建等待点推断。concrete public Package
    callable保留该summary作为Local ABI fact；interface requirement与conformance不拥有或比较该位。
    service call自身是caller的潜在挂起点，ServiceContract不携带provider内部summary，也不从它派生
    protocol identity或operation级内部停止类别。
15. external ingress分两阶段选择。Router外部的ingress可以按HTTP Host等平台规则映射service坐标，并向
    Router注入可信`x-skiff-service`与`x-skiff-version`；Router必须先用release pointer解析唯一精确
    `ServiceDeploymentRef`/buildId，再只在该deployment内按`IngressSelector`选择gateway entry。
16. HTTP Host不是`IngressSelector`字段，不参与Router中的service、deployment或handler选择。原始Host仍随
    标准HTTP request envelope进入业务metadata，handler可以读取；这不赋予它路由语义。Skiff不拥有
    Router外部的Host映射实现，也不在Router中重做local ingress。
17. `IngressSelector`只在一个精确deployment范围内有意义。HTTP selector是
    `(protocol, method, path)`，WebSocket upgrade selector是`(protocol, path)`；JSON-RPC method继续在
    connection已pin的deployment build内选择。不同service可以声明相同selector，同一deployment内重复
    selector必须失败。
18. 缺失、非法或歧义的service/version selector、pointer记录不兼容、以及Router到Runtime的跨build替换都
    fail closed。Router发出的request frame必须携带精确deployment/buildId；WebSocket连接同样pin exact
    build，不能从Host或ambient connection state重新推导。

## 3. Package 与 PackageArtifact

Package source由`.skiff`源码、`package.yml`、`api.yml`和静态资源组成。`package.yml`拥有package的人类
可读`id`/`version`、package dependencies与service dependencies；两类dependency alias共享同一namespace，
版本selector第一版都必须精确。`version`只用于人类展示和解析坐标，不参与任何artifact、ABI或内容identity
运算。

Service仍走同一个package compiler入口；存在`service.yml`时，compiler/tooling再执行service projection。
`service.yml`只拥有service id与`serviceCalls` public-path选择；它不拥有HTTP、WebSocket或deployment
policy字段。可选`http.yml`直接保存以稳定名字为key的HTTP entry mapping；可选`websocket.yml`
直接保存当前service唯一WebSocket entry的path、可选connect callback与可选`jsonRpc` method mapping。
HTTP/WebSocket external handler selector指向当前service package中的普通source callable，
不要求该callable出现在`api.yml`。`serviceCalls`中的每个元素则必须精确解析到`api.yml`已有的public
function或public-instance root；它不是source function映射。三个service authoring manifest都不含
version、dependency、service-call API signature/type映射、与handler类型重复的业务JSON schema、
实现artifact binding或平台组织角色。Request/response大小等平台limit不由源码manifest配置。
External schema和runtime codec plan由compiler从精确linked handler signature、adapter kind与参数来源
确定性生成。`config.yml`、`config.<profile>.yml`和`config.<profile>.secret.yml`只构造发布时冻结给
ServiceDeployment的typed protected config payload，不改变package/service dependency graph或
Package/ServiceContract identity；配置变化会产生新的deployment buildId。

三个authoring文件的层次为：

```yaml
# service.yml
id: example/service
serviceCalls:
  - users.get
```

```yaml
# http.yml
createUser:
  method: POST
  path: /users
  kind: typedJson
  handler: http.createUser
  adapterArgs:
    - param: input
      source: { kind: http.body }
```

```yaml
# websocket.yml
path: /ws
connect:
  handler: websocket.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
jsonRpc:
  getStatus:
    method: status.get
    handler: websocket.getStatus
    adapterArgs:
      - param: input
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
```

`http.yml`与`websocket.yml`都是可选文件；文件存在时必须是非`null` mapping，显式空HTTP surface写`{}`。
`websocket.yml`存在时`path`必填，`connect`和`jsonRpc`可分别省略；但一个既无connect也无JSON-RPC
handler、只供Skiff主动发送的connection仍是合法entry。旧`service.yml.http`与
`service.yml.websocket`直接报错，不提供兼容读取。`host`不是`http.yml`或`websocket.yml`的route字段；
旧Host-bearing service route直接报错，不提供兼容读取。

Authoring层不要求开发者分别维护entry表与route表，也没有只起分类作用的`routes`中间层。已冻结的HTTP
写法中，`http.yml`顶层本身就是以稳定名字为key的entry mapping，不再套一层`http:`；每个value把
external selector与该entry的handler/adapter声明写在一起。Mapping key就是service-owner-local
`GatewayEntryKey`。Compiler必须把
这一个HTTP authoring record确定性拆成`IngressSelector -> GatewayEntryKey`和
`GatewayEntryKey -> resolved gateway entry`两个artifact事实。第一版一个HTTP authoring route只定义
一个entry，不提供多个selector复用同一entry定义的别名/引用语法。`guard`/`pre`属于具体HTTP entry，
不占用顶层保留key，也没有隐式全局继承。这里的`IngressSelector`是当前service deployment内部的
`(http, method, path)`；它不是跨service/deployment的裸全局key。

HTTP entry kind决定external response协议，不能只按handler的`Stream<T>`外形推断。`typedJson`只允许
unary handler return；runtime wrapper把该单个值编码为一次JSON response，compiler必须拒绝
`typedJson`与任意`Stream<T>`返回组合。`rawHttp`允许返回单个`std.http.HttpResponse`，也允许精确返回
`Stream<std.http.HttpResponseStreamEvent>`；后一种是第一版唯一的external HTTP server-stream surface，
由handler显式控制status、headers和后续body chunks。Compiler不得把raw HTTP stream改投影成typed JSON
chunks，也不得要求external caller理解Skiff的`Stream<T>`类型。

第一版每个service最多一个`websocket.yml`，文件本身就是WebSocket entry，不再套一层`websocket:`。
它拥有连接path、可选`connect`回调，以及可选`jsonRpc` mapping。`jsonRpc`下每个稳定key把外部JSON-RPC
`method`与一个typed handler/adapter声明写在一起；compiler同样拆成
`(websocketEntry, profile, method) -> GatewayEntryKey -> resolved gateway entry`。Peer发来的合法
request创建新的runtime ingress并执行该handler；Skiff通过
`std.websocket.requestJsonToConnection`发起的request，其response只恢复原调用，不创建ingress。

WebSocket本身只是通用双向transport；双向request/response correlation由编码无关的平台broker拥有，
第一版内置`jsonrpc-2.0-text`编码配置。不存在raw `receive`、按任意event name分派的message handler或
把transport `id`交给业务代码的surface。第一版不声明任何JSON-RPC notification handler或peer主动取消
request的能力。Raw outbound text/binary send仍合法；未声明method的request返回JSON-RPC
`Method not found`，其它未被配置接纳的业务notification不进入用户代码。

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
  own typed config requirements
  database schema metadata
  callable semantic facts
  boundary callable projections
  unresolved ServiceCallRefs
```

PackageArtifact不保存`serviceCallRoots`或其它service selection字段，也不读取`service.yml`、
`http.yml`或`websocket.yml`。
它发布完整Package public callable graph及每个callable的boundary projection；ServiceContract projection
再用`service.yml.serviceCalls`选择roots。只改变`serviceCalls`而Package source、`package.yml`与
`api.yml`不变时，PackageArtifact与PackageLocalAbi identity必须bit-identical；operation集合变化只进入
ServiceContract/ServiceProtocolIdentity及其后续deployment。只改变`http.yml`或`websocket.yml`时，
PackageArtifact与ServiceContract也必须bit-identical；变化只进入typed ingress projection、
GatewayEntryIdentity、ServiceDeployment及包含它的可选`ReleaseBundle`。

`PackageLocalAbi`描述同一linked program内的public symbol、canonical signature、nominal type、
public instance、const与executable link信息。concrete public callable的canonical signature包含其推断
suspension summary，供依赖Package编译调用图；interface method requirement只包含调用形状，不复制该
summary，conformance也不比较它。目标VM下Local ABI还可以显式包含`InOut` parameter mode；它只表达
`NoPending` exact package/local call期间的exclusive caller-writable loan。普通参数仍是value semantics。
`InOut`不得投影进ServiceContract、gateway/interface/callback/Actor external ABI、host effect ABI或recoverable
payload。

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
HTTP body、query/path/header参数与HTTP response、WebSocket connect metadata，以及
`websocket.yml.jsonRpc`中每个method handler的params/result。Skiff主动发起的request/response codec来自
调用点concrete类型，不进入entry-local schema；peer主动request的codec来自被声明handler的linked
signature和adapter sources。Pre/guard内部context及其它只在runtime adapter与handler之间流动的值不进入
该closure。
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

普通package允许同时拥有Available和Unavailable public functions。未被`service.yml.serviceCalls`选择的
callable无论Available或Unavailable都只是Package API。存在`service.yml`时，只有被显式选择且Available的
public function或public-instance method成为service operation；被选择但Unavailable必须让service projection
以完整结构化原因失败，不能静默排除。公开type、alias或interface也可以只服务package linkage而没有合法
PackageSchema投影；generic declaration等不能进入service-call schema的public symbol必须保留在
PackageLocalAbi，并以结构化boundary-unavailable事实阻止相关service operation，不能让整个Package因一个
未被service-call使用的公开generic declaration失败。compiler/tooling必须输出完整、稳定、可机器读取的
列表及结构化原因；构建摘要、CLI/JSON、artifact receipt与IDE应消费同一projection，不能静默排除。

`service.yml.serviceCalls`是第一版唯一service-call选择机制。数组元素是`api.yml`已有function或
public-instance root的canonical public path；它不复制source selector、参数、返回或错误类型。compiler从
被选择的callable signature递归闭合已在`api.yml`拥有canonical public path的PackageSchema types。
重复、unknown或non-callable path必须报错；字段省略或空数组生成零operation contract。public instance
选择一次表示其显式listed interfaces的全部methods；若只需部分methods，
作者必须使用更窄interface或wrapper function。

`http.yml`或`websocket.yml`引用的每个external handler另行生成typed ingress projection。它可以引用
非public callable和非public named type，但必须有完整linked signature、精确PackageCallableId、合法
adapter plan以及可执行的external codec。Ingress availability与service-call availability分开报告；
不能通过把handler补进`api.yml`来绕过ingress校验。第一版handler/pre/guard不能是generic function
declaration；其concrete signature可以引用fully instantiated generic platform types。

缺字段不表示不可用或尚未分析。PackageArtifact必须保存完成boundary判断所需的typed effect、
provenance和link facts，使deployment无需读取源码。

`BoundaryOperationContract`只承载boundary可观察的signature、stream/callback、value plan与公开effect
保证；其中所有命名类型引用都保留`PackageSchemaTypeId`及其package owner。ServiceContract projection
在该body外增加稳定operation key/id，不把Package类型重写成`ContractTypeId`。service operation统一拥有
§6.3定义的开放错误通道，不在operation contract中列出可能抛出的类型集合。

Service call的pending wait统一参与caller request deadline与ancestor内部停止；这是调用种类本身的
语义，不是operation descriptor从provider body推断出的承诺。`BoundaryOperationContract`不得携带
provider concrete `maySuspend`，也不得保留由该位机械映射出的`NotCancellable`/`Cooperative`类别。
caller停止等待后provider是否、何时观察internal stop hint是runtime/deployment执行机制，不承诺callee
业务工作已经停止。Stream关闭等已由stream contract定义的内部停止语义不依赖callee内部summary。

第一版不另外定义consumer dependency timeout或callee operation timeout。Service call的可见deadline
就是调用点effective execution deadline，已经包含caller request deadline和外层`timeout(...)`的收紧；
需要更短调用预算时由caller显式使用`timeout(...)`。Service业务配置与ServiceDeployment都不拥有
request timeout override。

具体config/native capability requirement和完整may-effect（包括concrete suspension summary）属于
`BoundaryImplementationRequirements`或由deployment从PackageArtifact形成的implementation metadata，
不能泄漏进ServiceProtocolIdentity。External gateway的deadline与consumer-disconnect处理归gateway
entry/deployment owner，不能复用ServiceContract operation字段或从callable summary推导。

callback-capable interface的Package schema operation同样只保存interface requirement的参数、返回与其它
调用形状，不保存concrete implementation summary。interface schema type identity不得因某个implementor
从non-suspending变为suspending而改变。

同一个PackageArtifact可以同时：

- 被其它package直接链接；
- 在存在`service.yml`时从`serviceCalls`显式选择的roots生成一个ServiceContract并实现其全部operations；
- 为同一个service生成不进入ServiceContract的typed external ingress entries；
- 被多个ServiceDeployment revision复用；
- 由runtime按PackageArtifact identity共享validated/decoded code，同时在不同deployment image中保持各自
  owner/config/capability context。

## 4. ServiceContract

ServiceContract是独立发布、无代码的service-to-service typed API projection artifact。它的operation
authoring source是service package自己的`.skiff` declarations、`api.yml` public graph与
`service.yml.serviceCalls` public-path选择。不存在独立contract YAML/IDL、source映射或重复signature/type
清单。ServiceContract不引用provider
build、config、runtime replica或HTTP/WebSocket ingress。

```text
ServiceContract
  serviceId
  packageVersionLabel
  serviceProtocolIdentity
  operations: name/id -> BoundaryOperationDescriptor
  packageSchemaRequirements
```

每个operation descriptor包含canonical参数、返回、stream、callback与value plan契约，并统一使用开放
错误通道；它不包含operation-specific throw set、provider内部suspension summary或由该summary派生的
内部停止类别。ServiceContract不拥有第二套boundary类型，不内嵌或复制Package字段定义。它记录精确
`PackageTypeRequirement`：

```text
PackageTypeRequirement
  packageId
  requiredTypeIds: PackageSchemaTypeId[]
```

Service operation的参数和返回始终按value plan跨heap materialize。Package public callable若含`InOut`
parameter，其boundary projection必须为`Unavailable(InOutNotAllowedAtServiceBoundary)`，不能被
`service.yml.serviceCalls`选择；tooling不得把它降级成普通input、隐式copy-in/copy-out或remote reference。

Contract closure由operations引用的Package类型及各`PackageSchemaTypeRecord`描述的传递闭包组成。
tooling必须只按`PackageSchemaTypeId`读取content-addressed type record并闭合、校验；consumer不读取provider
源码，也不按release pointer target或任意同version PackageArtifact猜测类型。不同PackageSchemaIndex只要
解析到相同type id就是同一类型，不需要整包index identity相等。缺type record、owner/key不匹配、descriptor
重新计算出的identity不匹配或闭包不完整都fail closed。

Service package自己声明的类型与依赖package声明的类型遵守同一规则：都由各自Package拥有。ServiceContract
只拥有service operation集合及其协议身份，不拥有`ServiceType`、service-owned `ContractTypeId`或类型映射层。

External ingress不进入`operations`或`packageSchemaRequirements`。增加、删除或修改HTTP/WebSocket route、
handler、adapterArgs、external JSON schema或gateway policy不得改变`ServiceProtocolIdentity`；只有
service-call API及其Package schema closure变化才改变该identity。

Service API identity由canonical boundary surface内容确定；`package.yml.version`只作为人类可读、精确解析
label，不参与identity运算，`service.yml`没有version。新implementation build可以在identity不变时成为
release pointer的新target而无需consumer同意；API内容变化必然产生新identity，即使作者未修改version label也不能
静默绑定旧consumer。

operation引用的任一`PackageSchemaTypeId`或其闭包变化都属于API内容变化。反之，只要operation与所有引用
类型identity不变，Package内部实现、无关public function、未被该contract引用的Package类型或version label
变化都不能机械改变ServiceProtocolIdentity。尤其是callee concrete callable的`maySuspend`变化只影响
Package Local ABI/build与implementation/deployment identity；request/response/stream/callback shape及
开放错误通道不变时，`ContractOperationId`与`ServiceProtocolIdentity`都保持不变。

第一版`package.yml.services`形成的service dependency graph必须是DAG。Compiler/tooling按拓扑顺序解析已生成
的精确ServiceContract并编译consumer；同一编译批次或已解析dependency closure中出现环必须在编译
Package body前fail closed，不得隐式启动跨Package的两阶段全局源码批编译。需要反向调用时使用显式
callback capability、actor或重新划分Service边界，而不是制造静态service dependency cycle。

## 5. ServiceDeployment

ServiceDeployment是无源码的生成artifact，不是开发者维护的`deployment.yml`：

```text
ServiceDeployment
  serviceId / packageVersionLabel / expectedProtocolIdentity
  deploymentRevision / deploymentBuildId
  implementation PackageArtifact ref
  operationBindings: contractOperationId -> packageCallableId
  packageDependencyBindings
  serviceDependencySlots: coordinate + expectedProtocolIdentity + usedOperations
  bakedConfigPayloadRef
  gatewayEntries: gatewayEntryKey -> {
    gatewayEntryIdentity
    protocol
    handler/pre/guard packageCallableId
    typed adapter plan
    external protocol metadata
  }
  ingress: serviceLocalIngressSelector -> gatewayEntryKey
```

operation mapping由同一service package的ServiceContract projection与PackageArtifact public callable
identity确定性生成。只有`service.yml.serviceCalls`显式选择的roots进入；该数组只选择public path，
不得再要求开发者在`deployment.yml`或其它位置重复source/callable映射。生成artifact必须写入稳定
callable id，runtime禁止按display name猜target。

deployment execution plan可以从绑定的PackageArtifact读取concrete callable suspension summary，用于选择
provider内部执行lane、internal stop signal投递或其它Host机制；该summary只验证implementation callable与其
executable一致，不与ServiceContract对账。summary改变会因implementation build改变而产生新的deployment
revision/identity，但不能倒灌成新的service protocol。

Ingress不绑定`ContractOperationId`，也不进入ServiceContract。Compiler从`http.yml`或`websocket.yml`
的source selector
解析当前implementation中的精确`PackageCallableId`，校验handler signature与adapter source，生成typed
gateway entry并计算`GatewayEntryIdentity`。`gatewayEntryKey`只是对应external manifest内稳定、
owner-local的entry键，使同一协议identity可以绑定不同implementation或被多个selector复用；它不是内容identity。
Deployment只消费该typed projection并把external selector绑定到gateway entry；
Router和Runtime不得按source path、display name或同名service operation猜handler。

### 5.1 External ingress的两阶段选择

ServiceDeployment只拥有service内部route，不拥有公网域名或HTTP Host：

```text
external ingress
  Host / platform mapping
  -> x-skiff-service + x-skiff-version

Skiff Router
  trusted headers
  -> exact ServiceDeploymentRef
  -> service-local IngressSelector
  -> GatewayEntryKey
  -> GatewayEntryIdentity + exact handler
```

Router必须严格解析`x-skiff-service`与`x-skiff-version`，并通过release pointer解析出恰好一个
`ServiceDeploymentRef`/buildId。header缺失、重复冲突、格式非法、未知坐标或pointer记录不兼容都不能
继续按Host、path、display name或其它deployment猜测。直接向Router发送这两个header的receipt就是Skiff
生产边界证据；Host到header的映射属于Router外部ingress，不在Skiff内重复实现。

选择deployment后，HTTP只按`(protocol, method, path)`，WebSocket upgrade只按
`(protocol, path)`查询该deployment的`ingress`。因此Relay与AIHub可以同时声明
`GET /v1/models`；它们由不同service坐标定界。同一deployment内重复selector仍是authoring/projection错误。
请求的原始Host、URL、headers等继续作为标准HTTP envelope传给业务代码，但Host不能改变已选择的deployment
或gateway entry。

Router到Runtime的dispatch必须携带精确`ServiceDeploymentRef`、buildId与gateway entry事实。Router只选择
已注册该buildId或具备对应lazy-load能力的session；Runtime按frame中的exact build执行load/verify，二者都
禁止用同service的另一revision、同path的另一service或ambient registration替换。WebSocket upgrade先执行
同一deployment选择，再把exact build固定到connection；连接内JSON-RPC method只在该pin内解析。

Artifact模型允许多个selector绑定同一个key，但第一版`http.yml`和`websocket.yml.jsonRpc`不暴露独立
entry引用或复用语法：named entry同时声明唯一selector与entry definition。该限制只简化authoring，不得把selector并入
`GatewayEntryIdentity`，也不得让Router跳过上述两步查找。

`GatewayEntryIdentity`只标识external protocol surface。已冻结的HTTP canonical preimage覆盖entry kind、
外部request/response/stream shape、HTTP adapter source映射、公开错误投影及其它会改变gateway wire
兼容性的metadata。WebSocket connect entry identity覆盖connect request/result shape、允许的frame类别、
JSON-RPC profile版本和connection policy shape。每个WebSocket JSON-RPC method另有gateway entry identity，
覆盖params/result external shape、adapter source映射和固定错误投影；外部`method`字符串仍是
`IngressSelector`，不并入entry identity。Identity不包含source selector、handler/pre/guard
`PackageCallableId`、内部名义类型identity、PackageArtifact/build或deployment policy。
Compiler仍必须验证由linked handler signature导出的external schema和typed adapter plan与该surface逐项
一致。HTTP stream mode只能来自`rawHttp`的精确
`Stream<std.http.HttpResponseStreamEvent>`返回；`typedJson` identity始终是unary，不能生成或接受
server-stream shape。WebSocket JSON-RPC handler第一版也只能unary return，不能返回`Stream<T>`。

具体handler/pre/guard callable、完整typed adapter execution plan、implementation artifact、external
selector和policy只由ServiceDeployment及其revision覆盖。只替换实现且external protocol不变时，
`GatewayEntryIdentity`保持不变而deployment revision改变；改变external wire surface时两者都改变。

本次service-scoped ingress是未发布格式的hard cut。代际固定为：

| 记录 / wire | 新代际 |
| --- | --- |
| ServiceDeploymentInput | `skiff-service-deployment-input-v5` |
| ServiceDeployment schema | `skiff-service-deployment-v4` |
| DeploymentArtifact identity marker / prefix | `skiff-deployment-artifact-identity-v4` / `skiff-deployment-artifact-v4:sha256` |
| Router↔Runtime frame schema | `skiff-runtime-frame-v4` |

`GatewayEntryIdentity`/GatewayEntry保持v2；ServiceContract/ServiceProtocol、Package artifact/build/local
ABI/schema与WebSocketEntryId不变。可选`ReleaseBundle`的schema不进入runtime frame代际。
Reader只接受上述代际的service-scoped ingress与exact-build frame，不为更早shape提供dual-read。

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

`packageDependencyBindings`精确闭合implementation的Package Local ABI requirements；
`serviceDependencySlots`只保存service coordinate、expected protocol与used operations，不保存invocation profile的
provider build或executable address。每次service invocation开始时由boundary scheduler解析release pointer、
校验provider protocol并pin exact provider build owner。

deployment validation必须保证：

- 每个由`service.yml.serviceCalls`显式选择的root生成的operation恰好映射到其source public callable；
- 不存在手工增加、遗漏或重复operation；
- target callable的boundary projection是`Available`；
- operation descriptor、schema closure与同一canonical API projection逐项精确匹配；
- implementation boundary-visible may-effect满足contract公开effect保证，且所有implementation
  requirements得到binding；concrete suspension summary只与Package callable/executable事实对账，不与
  interface requirement或ServiceContract比较；
- 每个`http.yml` HTTP selector和`websocket.yml.jsonRpc` method selector恰好绑定一个canonical
  gateway entry；
- gateway entry中的handler/pre/guard全部解析到当前implementation的精确callable，adapterArgs与其linked
  signature逐项匹配，且不会被加入ServiceContract；
- gateway entry protocol identity与deployment binding/revision分别覆盖各自规定的全部事实，互不吞并；
- 第一版不生成用户语义adapter、字段兼容或fallback；
- implementation package及其依赖闭包可解析；
- service/package dependency与runtime callable requirements全部得到唯一binding。

ServiceDeployment可以换package build而保持同一service id/version label；前提是service API identity完全
不变。变化由deployment revision/buildId表达。业务配置变化同样生成新的immutable deployment/buildId并
原子更新该service/version release pointer；配置不能单独发布。

## 6. 两类调用与三层契约

### 6.1 Package direct call

package dependency调用使用`PackageLocalAbi`和implementation links：

- 同一`DeploymentExecutionImage`/owner内直接调用，复用当前managed heap；
- 普通参数遵守aggregate value semantics，runtime可按liveness做move或O(1) share/COW；
- 只有signature显式声明、Local ABI记录且callee为`NoPending`的`InOut`参数可以write-through修改caller path；
- 不经过service dispatcher，不切换deployment owner；
- 不做boundary materialization，也不要求recoverable。

`InOut`写入在ordinary throw时不回滚。它不能出现在interface requirement、service/gateway/callback/Actor
external signature、host effect ABI或recoverable payload中；Package boundary projection遇到它必须明确
`Unavailable`，不能静默复制或删除mode。

同一request、同一deployment owner内的wrapper调用`PackageDirect` stream producer时，stream handle必须继续
属于当前request已有的`StreamRuntime` registry；package call不能为该handle新建registry，也不能把handle
当作boundary value重新materialize。只有6.2定义的service call按value plan跨owner materialize参数、item
与返回值；不得把package-local registry共享规则扩张到service boundary。

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
`PackageRequirement`覆盖；deployment linker再次校验local ABI并解析`callableLinks`。不得把
`PackageCallableId`编码进`OperationAbiRef`，也不得恢复`PackageOperationIndex`、publication ABI builder或
used-symbol closure作为bridge。

### 6.2 Service boundary call

service dependency调用只解析到`ServiceContract` operation。Consumer lowering不链接provider executable，
也不生成伪PackageArtifact：

```text
ServiceCallRef
  serviceRequirementSlot
  contractOperationId
  expectedProtocolIdentity
```

执行call site时，boundary scheduler按当前profile解析slot的`serviceId + exact version` release pointer，
得到provider `ServiceDeploymentRef`/buildId，验证其protocol identity满足caller冻结的expectation，然后取得
或lazy-load immutable provider image。Pointer缺失、record/identity不匹配、provider load失败都在进入provider
前fail closed；不得按display name、ambient service或另一个version猜测。

物理binding有两种，但语言boundary相同：

```text
InProcessBoundary
  -> VM scheduler EnterChild(provider fiber + fresh heap)

RemoteBoundary
  -> runtime transport Pending/response
```

进程内路径不能因为地址可见就传递caller heap handle、mutable root、`InOut` loan或local method table；它只
省transport和native future。Provider同步完成时child trampoline直接返回，不制造虚假Pending；只有child
或transport实际等待时caller fiber才park、caller Actor才释放segment lease。

原子重绑定按owner把执行上下文拆成两类：

- deployment-scoped owner全部替换为exact provider build的事实：Package-scoped `ConfigView`、service DB、
  file capability、actor registry/capability、dispatch、WebSocket service/entry lookup、telemetry
  service attribution及service dependency slots；
- request-scoped owner保持caller request的同一事实：deadline、runtime内部停止/cancellation、clock/time
  source、request lifecycle、trace、error channel、runtime transport request identity、
  stream sink/source lifecycle、test effect registry、opaque test-case capability与heap limits。

request-scoped继承不表示共享caller heap。Provider必须获得fresh request-local heap；参数按contract value
plan从caller heap materialize到provider heap，unary return、错误payload、callback参数/返回和stream item
再按相同boundary contract跨heap materialize。Caller的call frames、slot values、mutable roots和
`ActorExecutionFrame`不得进入provider。Caller actor只在service call实际等待时按既有suspension规则释放
自己的executor；provider执行不属于caller actor segment。

Package静态资源不属于deployment-scoped capability rebind。它继续随当前callable的
provider image解析：进入provider后使用provider executable对应的Package resource projection，Package
direct call仍按其当前callable package owner读取。不得把静态资源复制到ambient context或因deployment
相同而改写Package owner。

Runtime内部`ActorRef`携带显式actor type/id/route owner。Rebind只替换“当前service可使用哪个actor
registry/capability”的deployment owner，不重写已经存在的`ActorRef`显式owner，也不把caller actor frame
传给provider。

source effect analysis只要解析到`ServiceCallRef`就把该call site视为`maySuspend=true`；ServiceContract
operation不提供callee summary位。InProcessBoundary即使本次provider立即返回，也不能
把该调用重新分类成package direct call。caller只在response尚未就绪而实际等待时释放actor executor。
provider runtime若需其concrete summary选择内部lane，只能从deployment绑定的PackageArtifact取得。

Service slot逻辑key是`(callerDeploymentBuildId, callerPackageBuildId, serviceRequirementSlot)`，不是裸slot
index。全局把call site patch到某个当前provider executable是错误的：同一consumer build在不同profile/
时刻可解析到不同但protocol-compatible provider build。

普通挂起/恢复和stream producer/consumer必须保留创建它们时的exact deployment owner；不能依赖thread-local
“当前service”。Service provider entry和callback capability dispatch是owner transfer入口。Callback调用
切回capability记录的owner，返回后恢复receiver owner。

一次service invocation在开始时pin provider build。若返回stream，producer、consumer bridge、callback与
每个item materialization继续使用该build直到stream end/error/drop；release pointer更新不迁移既有stream。
新的invocation重新解析pointer。Skiff不提供跨service的ambient atomic release snapshot。

这里区分三层：

- ServiceContract：位置无关的语言语义。
- Binding ABI：`InProcessBoundary`或`RemoteBoundary`的物理适配接口。
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

HTTP、WebSocket及未来其它gateway entry不是service boundary call。Router先按可信
`x-skiff-service + x-skiff-version`经release pointer选择精确`ServiceDeploymentRef`/buildId，再按
`ServiceDeploymentRef -> IngressSelector -> GatewayEntryKey -> GatewayEntryIdentity`进入对应
deployment owner，并由deployment gateway entry binding中冻结的精确
`PackageCallableId`执行handler；它不经过service dependency slot，也不伪造`ContractOperationId`。

Ingress仍复用普通语言函数、Package本地链接、DeploymentExecutionContext、错误通道和runtime内部停止机制，但不复用
ServiceContract作为对外声明：

- handler/pre/guard只需由`http.yml`或`websocket.yml`显式引用，不需要出现在`api.yml`；
- ingress callable不会出现在service dependency的code-free API module中；
- handler参数和返回值的runtime codec来自linked callable signature及typed adapter plan；
- gateway只持有route、adapter metadata和opaque payload bytes，不解析业务类型；
- external JSON schema、HTTP status/header规则和WebSocket connection metadata属于gateway entry，
  不进入PackageSchema或ServiceProtocolIdentity；
- ingress抛出的错误可复用固定service error carrier交给gateway做脱敏投影，但外部caller不会因此成为一个
  可`catch<E>`的Skiff service caller。

同一个source function可以通过`api.yml` public path被`service.yml.serviceCalls`选择，同时又被
`http.yml`或`websocket.yml`引用为external handler，但这是两个显式surface：前者生成service-call
operation，后者生成external gateway entry。Compiler必须分别验证并生成不同identity，不能因source
target相同而把两者合并或互相推断。

WebSocket external ingress包含upgrade/connect，以及`websocket.yml.jsonRpc`显式声明的peer-initiated
request。它不包含raw frame receive、任意event name分派或任何业务notification handler。HTTP仍是浏览器和外部
系统普通请求的默认入口；需要一条已建立双工connection、低延迟反向调用的场景可以显式声明JSON-RPC
method。流式业务响应仍使用HTTP server stream，异步主动通知使用WebSocket下行。

`std.websocket`发送从当前`DeploymentExecutionContext`解析当前service deployment中唯一的WebSocket entry，不能按
path、display name或任意字符串猜entry；零entry或损坏的多entry状态fail closed。Skiff代码也可以向一个
**精确connection id**发起request，外部peer接受该request并在同一socket上返回response；该response只恢复
原调用。Peer向Skiff发起的request则按socket pin住的deployment build与transport connection incarnation解析method并创建新的
runtime ingress。两个方向共享frame codec但不共享pending identity namespace。

WebSocket transport、request/response broker与编码配置分层：

```text
业务调用
  -> request/response语义与pending生命周期
  -> 编码配置
  -> WebSocket text或binary frame
  -> TCP
```

Broker只拥有request identity、pending、deadline/内部停止、connection incarnation归属和容量限制，不把
JSON字段写死在核心状态机中。第一版只内置`jsonrpc-2.0-text`配置；未来binary RPC必须以独立配置显式定义
版本、framing、codec与协商规则，不能把任意binary frame自动当作RPC。现有`sendText*`/`sendBinary*`保持
raw outbound send，不因为RPC配置而改变语义。

JSON-RPC 2.0只定义message编码，不假设某一种连接生命周期；`jsonrpc-2.0-text`配置把每个message绑定为
一个WebSocket text frame，并精确使用JSON-RPC 2.0单请求/单响应对象：

```json
{"jsonrpc":"2.0","id":"<id>","method":"<method>","params":{}}
{"jsonrpc":"2.0","id":"<id>","result":null}
{"jsonrpc":"2.0","id":"<id>",
 "error":{"code":-32603,"message":"<message>","data":null}}
```

第一版不执行JSON-RPC batch；收到peer batch时返回单个`Invalid Request`且不执行其中成员。平台发起请求时
只生成非空string `id`，外部peer只能在response中原样回显。Peer发起请求时可以使用非空string或JavaScript
safe integer `id`，平台按原JSON类型和值回显；fraction、超出safe integer范围、`null`或其它类型非法。
Skiff业务源码不生成、解析或持久化任一方向的transport id。

Inbound response id规则同样固定：parse失败、batch或无法识别出合法request id的Invalid Request使用
`"id": null`；已经识别出合法typed id后的method/params/capacity/timeout/internal error必须回显
原string或safe-integer值。缺少`id`的合法object是notification，一律不dispatch也不response。
相同connection incarnation/direction上仍active或仍在bounded settled tombstone中的id不得再次
发起request；重复id是`1002`协议错误并关闭socket，不能先返回一个同id错误再让旧execution或晚到result
作用于新request。Tombstone到期或按容量驱逐后可以复用该id；peer应优先生成connection-lifetime唯一id。
平台生成的outbound string id在同一connection incarnation内不得复用。

第一版所有notification都没有业务或平台取消语义。Pending key至少包含direction、connection id、
socket incarnation identity、配置id与request id；response必须来自原connection，unknown、duplicate或
跨incarnation的response不能命中其它调用。配置adapter只解析JSON-RPC request/response控制字段；
`method`、`params`、`result`与error `data`保持opaque。

`method`必须非空。`value`按`std.json.encode<TRequest>`语义编码，并且顶层结果必须是JSON object或array，
以满足JSON-RPC `params`约束；无参数方法传空object。Success `result`按
`std.json.decode<TResponse>`语义解码。编码、非法params shape或typed decode失败抛
`std.json.DecodeError`，不改写成transport error，也不要求Router理解业务schema。

标准库入口是：

```skiff
native function requestJsonToConnection<TRequest, TResponse>(
  connectionId: string,
  method: string,
  value: TRequest
) -> TResponse
```

它只允许精确connection target，不提供business-identity fan-out版本，因为多个socket不能共同拥有一个
unary response。调用受当前execution deadline与runtime内部停止约束；等待尚未完成时是真实suspension
point。当前有效deadline到达时抛`TimeoutError`；ancestor内部停止只结束当前request/lane且不可被用户
`catch`。二者都先删除pending state并丢弃late response；第一版不向peer发送request cancellation
notification。目标不存在或
已关闭、发送后transport丢失、response协议错误、平台容量拒绝和peer JSON-RPC error分别投影为
`std.websocket.WebSocketRequestError`的封闭分支；本地分支只暴露固定、脱敏信息，remote分支保留经过
大小和shape校验的JSON-RPC integer `code`、`message`与可选`data`。

平台保留有界短期settled tombstone以丢弃完成/内部停止竞态产生的晚到或重复response。Pending数量和payload
大小达到上限时新request fail closed；tombstone达到容量时按最旧到期顺序驱逐，不因已完成请求占满表而
拒绝新request。驱逐后的晚到response按unknown id处理，仍不能恢复任何调用。Transport不自动retry；
断线可能发生在peer收到request之前或之后，因此外部副作用的幂等、去重和补偿仍由业务ID承担。

Peer主动request的dispatch规则：

- `(websocket entry id, jsonrpc-2.0-text, method)`必须在socket pin住的deployment build与connection
  incarnation中精确命中
  `websocket.yml.jsonRpc`的一个entry；unknown method返回`-32601 Method not found`。
- `params`必须是object或array，并按该entry的linked handler signature和
  `websocket.jsonRpcParams` adapter source做typed decode；缺失、shape不符或typed decode失败返回
  `-32602 Invalid params`，handler不执行。
- 第一版可用adapter source只有`websocket.jsonRpcParams`、`websocket.connectionId`和
  `websocket.businessIdentity`。每个handler必须且只能绑定一次完整params；后两者由平台连接状态提供，
  业务不能伪造；handler永远拿不到transport id。
- Handler只能unary return。普通return按linked return type编码到`result`；`void`编码为`null`。预期业务
  失败应使用返回union。未捕获throw统一投影为`-32603 Internal error`，不暴露Skiff名义错误、message、stack
  或私有字段。
- JSON parse失败返回`-32700 Parse error`，非法request object或batch返回`-32600 Invalid Request`；
  平台容量拒绝返回`-32000 Server busy`，有效deadline到达返回`-32001 Request timed out`。
  这些平台错误使用固定message且默认省略`data`。
- 有`id`的合法request必须恰好产生一次result或error，除非socket已经关闭。Peer disconnect会内部停止该
  connection incarnation上仍在运行的inbound execution；晚到完成被丢弃，但这只是runtime内部停止，
  不产生cancel response或rollback承诺。
- 任何JSON-RPC notification都不调用用户代码、不产生response；平台保留前缀也没有特殊取消语义。
  即使notification method与已声明request method同名也一样，只记录有界诊断。Binary frame不是本配置的一部分；没有用户raw
  receive时以`1003`拒绝。
- Response object只允许恢复Skiff发起的outbound pending；request object只允许创建上述declared ingress。
  畸形/伪造response、wrong connection incarnation或unknown outbound response id属于`1002`协议错误，不能
  被误判成peer request。

HTTP request与其unary response/server stream已经由transport精确关联。External payload、response
envelope和stream item不得保留只为模拟旧WebSocket req/res而存在的`requestId`或同义correlation字段；
平台内部request/trace id也不进入业务schema。若操作真正需要幂等键、异步任务句柄或业务run identity，
必须分别建模为`idempotencyKey`、`jobId`或`runId`，不能继续借用`requestId`。这一规则同时适用于
Agine普通HTTP RPC、Host主动发起的HTTP上行和AIHub HTTP event stream；它不删除上述平台拥有、并在
`jsonrpc-2.0-text` wire中表示为`id`的transport request identity。

### 6.5 Test service HTTP ingress

`kind: test` service通过现有HTTP client与真实external ingress测试HTTP entry，不定义第二套协议：

- test service显式提供自己的`http.yml`，entry引用该test service `*.test.skiff`中的wrapper；
  runner不自动复制或推断被测package/service的production ingress；
- 测试源码调用普通`std.http.request`或`std.http.stream`，并传普通绝对`http`/`https` URL；
  不新增标准库入口、特殊URL、语言关键字或测试metadata；
- 非live runner拥有隔离Router的动态business ingress URL，并通过现有resolved config view只读提供
  `skiff.test.ingressUrl`。同一次runner invocation中的cases共享该URL；authored config不能覆盖该
  保留path，这不是per-case config override；
- runner已为每个case生成唯一service id和对应contract version。测试执行适配只对origin精确等于上述
  动态ingress URL的调用，在inline effect匹配前识别为self-ingress，并自动加入现有
  `x-skiff-service`与`x-skiff-version` selector；其它URL仍按普通outbound HTTP double与network
  policy处理；
- Router只执行普通`service + version + method + path`选择，Host不参与路由。本能力不增加Router
  test route、session header、token、签名、control-plane业务转发、runtime wire字段或schema版本；
- runner拥有隔离Router和单一Runtime。Runtime/test execution按精确case deployment使self-ingress
  子请求复用父case的inline-effect registry；子请求不另行setup或finalize，父case是唯一finalization
  owner；
- self-ingress父请求与HTTP子请求仍是两个独立request，各自拥有Interpreter、request heap和
  `StreamRuntime` registry。父子之间只共享`TestEffectCaseContext`；其中的stream effect以wire item
  snapshots保存，并在HTTP子请求当前runtime中生成新的stream handle，不能把父request的stream id或
  registry handle带入子request。子request内部的wrapper→`PackageDirect`调用再遵守6.1的同request
  registry规则；
- 第一版同一case最多有一个active self-ingress子请求。`request`在完整response结束后释放；
  `stream`在EOF、失败或consumer drop/break后释放。已有active子请求时再发起一个应使case失败；
- stream复用普通HTTP client stream、Router backpressure和disconnect取消链，不新增显式cancel API。
  测试断言完整response body或解码后的协议frame；TCP/HTTP chunk边界不是业务合同，SSE按完整event断言。

测试代码提供的headers不能覆盖大小写不敏感的`x-skiff-service`、`x-skiff-version`、`Host`、
`Content-Length`、`Transfer-Encoding`或hop-by-hop headers；冲突在发送前使case失败。Host来自实际
隔离Router连接且不参与路由。普通production execution、live target和非self-ingress URL不获得这些
适配。

## 7. Linkable、Recoverable 与 Callback Capability

即时service call使用lane-scoped linkable plan：

```text
LinkableValuePlan<ServiceCallLane>
  = 当前调用期间可materialize的carrier、encoding、owner和lifetime计划

RecoverableValuePlan<Lane>
  = LinkableValuePlan<Lane> + FutureValidityPlan
```

DB、dispatch、queue、persistent work item或其它跨request lane才要求recoverable。普通service参数、返回
和error payload不因为是boundary call就必须在未来request中恢复。

ordinary data按contract生成detached value graph。目标aggregate value semantics不暴露physical alias/
backing identity：普通参数即使在callee中mutation也只修改callee value，不需要ServiceContract表达
caller writeback；返回caller输入同样只是一个逻辑snapshot。只有显式`InOut`表示caller-writable，而它在
service boundary一律非法。

Compiler仍可为Package Local ABI/优化追踪physical provenance、move/share和write facts，但boundary
eligibility不能把COW backing alias当成语言可观察identity。Native/resource/interface capability是否可跨界由
显式type/value plan和adapter contract决定；unknown target/type继续独立fail closed。

本地`any I`或native handle若要跨service，只能投影成request-scope callback capability：

```text
CallbackCapability
  ownerDeploymentBuildId / ownerRuntimeRoute
  requestIdentity
  interfaceOrAdapterContract
  opaqueCapabilityId
```

约束：

- capability由创建该值的exact deployment owner/request拥有；
- 生命周期到顶层request结束，stream存在时延长到stream关闭；内部停止或owner退出会提前失效；
- 对端只能通过contract声明的operation回调owner，不能得到method table、native object或本地地址；
- capability不能进入DB、dispatch、queue、persistent payload或其它recoverable lane；
- 失效返回稳定`CapabilityExpired`/`CapabilityUnavailable`错误，不重建、不fallback；
- `any I`只有所有被投影method都boundary-capable时才可生成callback；native value必须有显式callback
  adapter，否则对应operation不可用。

InProcessBoundary用runtime capability table实现；未来RemoteBoundary使用opaque route回到owner。
两者对语言层值保持同一lifetime与失效语义。

## 8. Effect 与 Boundary Eligibility

所有package callable都可以拥有Local ABI；只有boundary projection为`Available`的callable能实现
ServiceContract operation。

compiler执行sound may-analysis，至少追踪：

- aggregate value的move/share/write与显式`InOut` loan；
- 返回或throw payload的logical type/value plan；
- caller value是否escape到capture、callback、stream、dispatch、DB或native/external target；
- callback/native adapter requirement；
- suspension summary与unknown call/effect。

分析允许保守拒绝，不允许漏掉boundary-visible行为。`InOut`、不可materialize resource、非法callback
position、schema不闭合与unknown ABI各有独立结构化原因。普通callee-local collection/record mutation不再
使callable无法成为service operation，因为caller从未获得write-through alias。

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
它还为全部function/impl method拥有exact executable signature facts（包括推断的suspension summary），并为
全部interface operation拥有不含该summary的exact requirement facts；public callable signature只是
executable事实表按public path产生的view。interface conformance只比较receiver、参数、返回及其它
requirement-owned调用形状，不比较suspension。Lowering只消费typed source facts，把exact source type投影为
File IR execution representation。package direct call降低为`PackageCallable` target；service call降低为
`ServiceCallRef`。
compiled/projection-input只转交source-validated interface binding/method key与execution target所需结构事实，
不从File IR或`TypeResolutionModel`重算conformance。Package projection不读deployment配置，也不从File IR
execution signature重做source conformance或ABI identity。

Service call lowering只生成`ServiceCallRef`和contract value plan refs。Deployment image linker生成
service dependency slot/thunk；slot保留contract坐标，调用时解析provider build。它不是stub package，也不让
consumer依赖provider PackageLocalAbi。

Deployment projection不拥有AST、source text或lowering helper。平台即使持有全部源码，也只能将其用于
统一调度、诊断和可选whole-deployment优化；正确性必须只依赖typed artifacts，否则Package不再是独立
编译单元。

compiler内部不存在`PublicationInput`、`PublicationKind`、`CompiledPublication`、
`LoweredPublication`或带package/service option的共同projection bundle。

## 10. 依赖与 Identity

package dependency与service dependency是两种edge，都由`package.yml`声明：

```text
PackageRequirement
  alias + packageId + exactVersion + expectedLocalAbi
  optional expectedPackageBuild for test-only top-level access

ServiceRequirement
  alias + serviceId + exactVersionLabel + expectedProtocolIdentity
  serviceBindingSlot + usedOperations

PackageTypeRequirement
  packageId + requiredTypeIds
```

PackageRequirement在link阶段解析为不可变PackageArtifact。ServiceRequirement允许package解析发布后的
service API types和operation signatures，但不要求provider implementation；只有实际service call sites产生
runtime binding slot。它不包含provider package、provider build、deployment revision或runtime route；
执行service call时才由release pointer选择exact provider deployment/build。

每个package dependency entry的primary `alias`始终解析该dependency `api.yml` public paths。仅
`kind: test` service可在同一entry增加`topLevelAlias`，按
`<top-level-alias>/<source-module-path>.<top-level-name>`解析精确implementation source top-level。
`topLevelAlias`必须是合法唯一identifier，并与所有package/service alias及其它`topLevelAlias`无冲突；
两套名字没有fallback或precedence。旧`access: topLevel`字段及其互斥解析模式必须fail closed。

public alias与top-level alias是同一个direct dependency edge、`PackageRequirement`与
`PackageBinding`。Top-level source reference在lowering时canonicalize回primary alias，并由
`PackageRequirement.expectedPackageBuild`选择精确provider build；不得产生第二个requirement、code slot或
collection projection。Consumer File IR中的外部DB target复用
`PackageSymbolRef { package: Dependency(alias), symbolPath, abiExpectation }`，不另造DB专用package
reference，也不复制provider的DB metadata。

该test-only implementation view还包含精确implementation type现有的impl method namespace。Projection
把这些method以既有`PackageCallableId`、exact signature及`callableLinks`登记到
`PackageArtifact.packageLocalAbi.implementationSymbols`；不新增artifact字段。Source只在receiver为
该direct top-level view产生的精确
`PackageSymbolRef { package: Dependency(alias), symbolPath, abiExpectation }`，或以它为base的完整
generic applied nominal时，将method解析成现有package callable target。Lowering继续生成普通
`PackageCallable`调用，并把receiver作为第一项执行参数。

Artifact identity validation必须按每个`callableLinks`中的精确`PackageCallableId`，在互斥的
`publicSymbols`与`implementationSymbols` callable facts中恰好解析一次。implementation-only impl
callable id固定为
`pkg-callable:<packageId>:top-level:<sourcePath>`，并使用`implementationSymbols`中的exact
signature/type-parameter scope校验其
`implementationLinks.implMethods` executable target，并要求`OperationCallableKind::ImplMethod`；同一
executable同时拥有public-instance callable与implementation-only callable时，分别验证两个canonical id，
不能把后者伪装为`InternalFunction`或public symbol。impl method target coverage由public与implementation
`ImplMethod` callable并集闭合，不能再假定每个impl method都公开。缺失、重复、错误surface owner、
signature scope、file/index target或callable kind一律fail closed。现有canonical identity projection已经
覆盖两套surface及callable links；这项校验闭合不新增schema、字段或identity代际。

普通public alias、service boundary对象和interface receiver不获得这项implementation method view。
public alias仍只有API graph公开的public instance methods；service boundary只按ServiceContract
operation/schema materialize；interface receiver只按interface slot dispatch。Compiler不得按显示名、
短名或同名method回退，也不得因这一规则新增语法、schema、ABI代际或运行时动态lookup。

top-level权限不传递。Subject public ABI可以正常闭合其dependency public types，但test consumer不能因此
直接使用transitive dependency top-level；确需访问时必须在test manifest中为该provider声明direct
dependency并设置该entry的`topLevelAlias`。普通package edge和production service没有implementation
top-level可见性，不能借DB target扩大权限。

必须分开的identity：

- PackageId / PackageVersion：人类可读代码发布坐标；version不参与任何identity hash。
- PackageBuildId：具体不可变代码build。
- PackageLocalAbiIdentity：local public code ABI。
- PackageSchemaIndexIdentity：某个PackageArtifact完整schema目录的内容身份；不进入service protocol。
- PackageSchemaTypeId：Package拥有的单个boundary类型内容身份；version和service id不参与。
- ServiceId / exact PackageVersion label：consumer依赖坐标；service.yml不重复version。
- ServiceProtocolIdentity：canonical boundary surface内容身份。
- DeploymentRevision / DeploymentArtifactIdentity：某次implementation、配置与route revision。
- BundleIdentity：可选`ReleaseBundle`的聚合身份，不进入执行owner。
- RuntimeReplicaId：某个runtime worker；其loaded buildId集合不进入artifact contract。

任何identity都不能因为display string相同而互换。ServiceProtocolIdentity包含operation实际引用的
PackageSchemaTypeId/closure identity，但不包含provider implementation build或deployment字段；
可选BundleIdentity只提交bundle中的exact deployment refs与verification receipts，不能回写consumer
requirement或成为request owner。

suspension变化的identity边界固定如下：

- concrete public Package callable的`maySuspend`变化保持`PackageCallableId`稳定，但改变
  `PackageLocalAbiIdentity`与`PackageBuildId`，直接package依赖方必须重编译；
- interface requirement没有suspension位，implementor summary变化不改变interface requirement、
  conformance或interface method stable identity；
- service operation保持`ContractOperationId`稳定；request/response/stream/callback与公开错误语义不变时，
  provider内部summary变化不改变`ServiceProtocolIdentity`；
- provider build、ServiceDeployment revision/identity及可选bundle identity可以随实现变化；这些
  implementation identity不得被解释为protocol变化；
- callback interface的`PackageSchemaTypeId`不包含implementor suspension summary。

跨package DB target按以下链路解析：

```text
consumer PackageSymbolRef
  -> PackageRequirement(expectedPackageBuild)
  -> PackageBinding
  -> exact PackageArtifactRef
  -> implementation_links.types[symbolPath]
  -> provider FileIrRef + typeIndex
  -> provider declarations.db
```

Linker必须核验type export、provider type declaration与DB attachment指向同一File IR type。链接后的唯一
运行时身份是`DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)`；`typeName`只用于诊断，禁止按名字、
module suffix或发现顺序lookup。Provider File IR独占collection、key、field、retention、lease、index与
recoverable metadata；consumer、PackageArtifact和linked executable不得复制这些事实。两个dependency即使
拥有相同module/type也因PackageArtifactRef不同而保持无冲突。Logical collection identity由stable
`(packageId, declared logical collection identity)`定界；Package build/version、dependency alias和edge
path不参与持久storage identity。Storage adapter把该logical identity确定性编码为当前service DB内的
physical collection name；dependency、requirement、binding和配置均不提供mapping。

同一stateful `PackageBuild`可因direct与transitive dependency形成多条真实edge。Deployment loader先解析
每条edge，再按以下规则形成该image中的collection projection与metadata owner：

- exact `PackageBuild`相同且所有owner-relevant facts相同：合并成一个active projection与metadata owner；
- 同一Package ID解析到不同build：拒绝；
- logical collection identity缺失、重复声明或system encoding collision：拒绝。

该projection是一次deployment image内最终生效的collection metadata owner，不是另一份数据库或
另一条package binding。合并不改变原始dependency graph，也不抹去用于诊断和identity的edge事实。
direct/transitive菱形不能生成第二份ConfigView、数据库或metadata owner。

该身份覆盖所有DB operation target、`DbQuery`、lease claim、lease state read和claim write guard。
Transaction本身只是当前service DB上的执行边界，没有独立target；内部operation各自携带目标。缺失link/type/
DB declaration、ABI或build不匹配、cross-artifact substitution都必须在load/link/verify阶段fail closed。

这是当前未发布artifact模型内的同代hard cut，不改变File IR v9、PackageArtifact v9、Package local ABI v7
或ServiceContract v5代际；不增加兼容reader、fallback或旧target双读。

## 11. Baked Config、Service DB 与 Platform Policy

Package可以声明自己读取的typed local config path、DB schema和native adapter requirement，但不拥有环境
配置值、数据库名字或平台policy。`PackageArtifact`只保存当前Package自己的typed config requirements；
它不复制dependency Package requirements。

发布一个service deployment时，tooling读取同root的`config.yml`、`config.<profile>.yml`和
`config.<profile>.secret.yml`。三份文件根部直接以canonical Package ID为key；三层按base、profile、secret
顺序递归overlay：mapping递归合并，scalar/sequence整体替换，`null`作为删除path的tombstone。Tooling用
exact package closure校验unknown Package ID、required path和类型，并为每个exact Package build生成只读
`ConfigView`。

验证后的config graph写入immutable、ServiceDeployment-owned `BakedConfigPayload`。Deployment只保存一个
不可替换的opaque protected ref；该ref进入deployment buildId，配置变化因此产生新buildId并通过release
pointer原子发布。Payload在语义上属于引用它的deployment；同ref的物理blob可以去重，但没有独立selector或
发布生命周期。

Protected artifact store writer是config ref的唯一identity owner，必须按以下语义计算：

```text
BakedConfigPayloadRef = KeyedIdentity(
  storeDomain.configIdentityKey,
  "skiff-baked-config-payload-v1" || canonicalBakedConfigBytes
)
```

`KeyedIdentity`必须是带版本domain separation的cryptographic PRF/MAC（例如HMAC-SHA-256），不能退化为
unkeyed content hash、可逆编码或把key identifier拼进公开preimage。

相同canonical payload在同一store security domain内必须得到同一ref，不同domain必须得到不同ref；不知道
domain key的一方不能用候选secret离线枚举ref。该ref进入公开deployment identity，但canonical config bytes与
Secret明文不得进入公开content-hash preimage、日志、receipt或control frame。密文可以使用随机nonce，但nonce
不得参与ref；重复put必须重读、解密并验证canonical bytes后幂等复用，任何同ref不同payload都fail closed。
Store保证ref下payload immutable并在load时执行authenticated decryption与完整性验证。Runtime按buildId加载时
只读取deployment固定的ref；missing、被替换、无法解密或完整性失败都使整个image load fail closed。由于ref
参与buildId，同一config发布到不同store security domain会得到不同deployment buildId，不能跨domain改写ref
而声称仍是同一build。

普通与secret文件使用相同schema，不使用字段级`SecretRef`。POSIX平台读取source前必须验证其为普通
non-symlink文件，secret mode精确为`0600`；artifact store、temporary file和protected config payload必须
使用owner-only权限或明确等价的加密store能力。配置值不得进入PackageArtifact、ServiceContract、release
pointer key、receipt、control frame、日志或诊断；它只存在于受保护的deployment内容中。具体at-rest
encryption是artifact store能力，不改变ConfigView语义。

一个service只有一个数据库identity，由operator选择的受信Mongo endpoint/storage domain、profile与
serviceId共同定界，不引入`platformId`。开发者不能在`package.yml`、service profile或源码中配置
database/namespace；service version、package version、deployment revision和runtime replica都不改变
数据库identity。只有deployment package closure包含DB metadata时才按需提供service DB handle。同一service中的Package
共享数据库，但保留各自精确Package/schema/collection identity；跨service DB访问禁止。service重命名、
profile变化或移动到另一个受信storage domain都会产生不同数据库identity，数据迁移必须显式执行。
physical database name的编码属于operator/runtime内部实现，但必须对该tuple确定、无碰撞、满足存储后端
命名限制并避免把任意service字符串直接当作未校验名称。

同理，physical collection name由stable
`(packageId, declared logical collection identity)`系统编码。`db object name`只声明logical identity，
不是physical name；Package dependency、requirement、binding和配置输入都不允许author-provided
collection-name mapping。不同Package可以使用相同的裸collection名字而不会共享storage；Package ID或
logical collection identity重命名需要显式迁移。

测试数据库按`(testRunId, generatedTestServiceId)`派生。Test-only foreign DB target gate只允许测试源码
引用dependency的精确DB metadata，实际读写仍落当前generated test service的数据库，不能打开provider
service数据库。Redis、queue或其它外部系统将来使用独立capability，不保留通用`state`枚举占位。

Runtime从exact deployment DB metadata为
`(trusted storage domain, profile, serviceId)`建立service DB index plan。每次deployment load在任何业务
storage mutation前，把其logical index definitions与durable managed definitions核对；同identity
同定义幂等通过，不同定义fail closed，不同名字做additive union。Index physical name由系统稳定编码Package ID、logical collection
identity与logical index identity，不包含version/build/alias/edge/replica。index field path复用统一DB
field policy和physical mapper，受管index固定simple/binary collation。

missing受管index在image可执行前做additive幂等创建，exact definition通过；changed definition fail closed，
不自动drop/rebuild；某个新deployment不再声明旧index也不自动drop。Unmanaged index和Mongo `_id_`保留并
忽略。多个replica/build并发exact create必须幂等收敛。Unique duplicate映射为脱敏、不可重试的
`std.db.ConstraintError`分类；普通Mongo细节不得越过adapter边界。

Partial index当前不支持。Compiler必须拒绝index `where`；File IR、runtime projection、linked metadata和
store command均不得携带raw Source AST predicate。未来支持必须先定义独立typed predicate IR，不保留当前
未执行语义的兼容字段。

Service profile没有`lifecycle`配置面；旧`maxConcurrency`和`idleTimeoutMs`均删除，出现`lifecycle`
必须fail closed。`DeploymentPolicy`和`ResourcePolicy`整体删除；ServiceDeploymentInput、
ServiceDeployment、可选`ReleaseBundle`和artifact identity都不拥有或复制service级
timeout、CPU、内存、quota、principal、并发或空闲超时。初期唯一并发配置是`router.yml`现有`runtime`
段的required正安全整数`maxConcurrency`，Router按每条Runtime WebSocket连接统一限制所有普通pending
request；Actor/control frame不计。该门禁不做动态CPU、内存或数据库资源估算，满载立即overload且不排队。

业务配置文件不拥有`state`、`principal`、`quota`、`resources`或deployment `timeout`。这些旧profile
字段全部删除；不能为了满足schema填占位值。未来CPU、memory、quota或principal等operator policy必须由
operator-owned独立配置设计，不得塞回Package业务配置或ServiceDeployment。

Router实例的`requestTimeoutMs`是external business request唯一的service外部请求截止时间来源；service
配置、ServiceDeployment和可选`ReleaseBundle`不能收紧、放宽或伪造它。Deployment image load有
独立operator load timeout；它不是一个业务request，也不改变任何release pointer。

tooling从精确PackageArtifact、生成的ServiceContract及闭合dependency resolution投影
ServiceDeployment，并把所选profile三层config的validated overlay写入同一immutable deployment。profile
不得增加/删除`package.yml`中的package或service dependency。

Package静态资源随PackageArtifact发布，并按当前执行callable的package owner读取。ServiceDeployment没有
用户代码资源；deployment-only证书与环境文件属于operator输入，不进入code artifact。

同一个PackageArtifact被两个service使用时，代码和静态资源可共享，DeploymentExecutionContext、ConfigView与
service DB handle必须分开；Router连接级并发门禁不随service复制。

## 12. DeploymentExecutionImage、可选 Bundle 与扩容

Runtime按buildId加载一个deployment及其Package Local ABI closure，生成immutable
`DeploymentExecutionImage`：

```text
DeploymentExecutionImage
  exact ServiceDeployment/buildId
  exact PackageArtifact/File bytecode closure
  linked package-direct targets/types/shapes
  frozen ConstantHeap
  operation/gateway entries
  serviceDependencySlots             # contract coordinate, not provider address
  deployment-owned capability/config plans
```

Loader必须先做pre-link structural validation，再link，最后做post-link semantic verification；完整契约见
[`bytecode-vm.md`](bytecode-vm.md)。Image只对应一个exact buildId，不得因load时provider pointer不同而生成
不同内容。Package requirement在image内绑定immutable PackageArtifact；package升级必须重新build/link相应
deployment。

Service requirement只绑定`serviceId + exact version + expectedProtocolIdentity`。每次service boundary
invocation开始时解析release pointer并pin provider build；因此provider实现可在protocol兼容时更新而无需
consumer重新编译。Pointer更新不覆盖任何immutable artifact，也不改变已经pin住的invocation。

多个runtime replica是可互换worker：各自按需lazy-load buildId，loaded set和eviction独立；in-flight request/
stream/callback以strong owner pin保证image存活。扩容只增加worker，不要求全量image预载或replica间同步。
完整lazy-load契约见[`runtime-lazy-load-deployment.md`](runtime-lazy-load-deployment.md)。

可选离线聚合物只有下列shape：

```text
ReleaseBundle
  deploymentRefs: ServiceDeploymentRef[]
  verificationReceipts: (VerificationReceiptRef | VerificationReceipt)[]
```

`BundleIdentity`只从上述两组canonical事实计算，不增加第三类聚合内容。Bundle可以让publish/verify/promotion
复现一批refs与receipts，但不含linked image、service binding address、ConfigView或execution context，也不
进入Router↔Runtime request frame。

Host/domain到service selector属于外部ingress；它不进入deployment image或bundle。任何runtime拓扑都不得
改变Package direct call、service boundary与external gateway entry三种语义。

## 13. Registry、Release 与 Publish

registry分别存储不可变PackageArtifact、ServiceContract、ServiceDeployment和可选`ReleaseBundle`。
`BakedConfigPayload`是ServiceDeployment-owned protected子记录，不成为第五类runtime artifact。Immutable write、
artifact graph materialization ordering、`(profile, serviceId, version) -> deployment buildId` pointer、
rollback与preload机械契约只由
[`runtime-lazy-load-deployment.md`](runtime-lazy-load-deployment.md)定义。

生产registry由可选的普通Skiff service `skiff.run/registry`实现。它和其它service一样首先是package，
源码root包含`package.yml`、`api.yml`、`service.yml`、按需存在的`http.yml`/`websocket.yml`与
`config.*.yml`，可以位于官方
`skiff-packages`仓库；其ServiceContract与ServiceDeployment均由tooling生成而非独立author。它不是
`skiff.run/std`、compiler platform source、语言intrinsic、native adapter或拥有compiler特权的package。
语言、compiler和runtime在没有该service时仍然完整可用。调用者通过普通typed ServiceContract调用registry；
compiler不得为`skiff.run/registry`保留package id、注入native declaration、授予特殊capability或要求外部
authoring descriptor。

Router和Runtime不知道registry service，也不通过它读取artifact。正式环境中，registry负责把已经验证和编译
完成的immutable records/materialized artifacts发布到部署配置的共享`artifactsPath`，并原子更新pointer；
开发环境由compiler相关CLI/tooling完成同一文件布局的编译与发布。当前阶段只冻结该owner和文件边界，不要求先
实现registry到共享路径的生产发布流程。

`skiff.run/registry`以Platform DB作为三类runtime artifact、可选bundle及typed release pointer target/
append-only audit history的唯一production durable source of truth。它和其它包含DB metadata的service一样只通过
普通`std.db` capability访问数据库。Mongo URL的唯一配置owner是Router的`serviceDb.mongoUrl`；该值不进入
service/package/compiler/deployment artifact，也不由runtime文件配置、环境变量或默认值提供。Router在
连接级bootstrap中把DB transport binding与`artifactsPath`一并下发给Runtime；Runtime只为exact loaded
deployment中含DB metadata的service建立deployment-scoped capability，service代码看不到provider URL。文件型
`CanonicalArtifactStore`只作为
local/dev/CLI backend，不参与production registry，也不与Platform DB dual-write。

`package.yml state`、`PackageRuntimeRequirements.state`、`StateBinding`、`StateBindingKind`与
`ServiceDeployment.stateBindings`全部删除。Compiler从Package自己的DB schema metadata知道它使用service
DB；Runtime按operator选择的受信Mongo endpoint/storage domain、profile与service identity定界
数据库，不引入platformId，也不从authoring配置反推。

Registry的deploy操作只调用canonical pointer writer更新一个key；rollback把同一key指回已验证旧buildId。
Runtime按buildId懒加载；可选preload hint不等待ACK、不构成事务。Load timeout是runtime/operator本地预算，
与business request timeout分离。

`publish`是操作：校验typed artifact、写入不可变内容、更新允许更新的pointer。它不产生
`Publication`对象，也不要求三类artifact/可选bundle实现共同kind enum。registry可以在一个事务/workflow中发布
多个immutable artifact，但每个artifact保持独立schema与identity；每个service pointer仍是独立单键操作，
workflow或bundle都不能改变其原子性边界。

## 14. Fail-closed 条件

以下情况必须在其最早可信边界（compile、deployment projection、pre-link validation、image link/verify、
release resolution）失败，不能靠名字或fallback猜测：

- service API schema不闭合或operation identity冲突；
- 自动生成的deployment operation缺失、重复、额外或descriptor不匹配；
- selected callable boundary unavailable，包括signature含`InOut`；
- `service.yml`仍包含`http`/`websocket`，普通package出现`http.yml`/`websocket.yml`，或external
  manifest shape/selector重复、非法；
- `http.yml`或`websocket.yml`的handler/pre/guard无法解析到当前Package callable，或adapterArgs与
  linked signature不匹配；
- `websocket.yml.jsonRpc` method重复、非法，params/result不是可执行的unary JSON codec，或使用
  未授权adapter source；
- ingress仍指向`ContractOperationId`、要求handler先进入`api.yml`，或gateway entry identity与typed
  projection不一致；
- HTTP/WebSocket service route仍声明Host，或Router把不同service的相同selector作为裸全局collision；
- 同一service deployment内selector重复；
- Router selector header缺失、重复冲突、非法、未知或歧义，或request frame中的deployment/entry与
  resolved buildId/image不匹配；
- service/package dependency缺失、版本或identity不匹配；
- test-only topLevel DB target缺少expected package build、type implementation link、provider File IR type或
  同type DB attachment，ABI/build不匹配，或被替换为其它artifact的同名type；
- runtime DB target只能靠`typeName`、module suffix或全图扫描定位，或consumer复制的schema/collection/
  recoverable metadata与provider File IR发生分叉；
- service invocation的release pointer缺失、provider protocol不匹配或provider build无法load；
- callback/native adapter缺失或lifetime无法表达；
- runtime需要重读源码、display name或raw JSON才能链接；
- shared package call site被全局绑定到某个provider executable而绕过caller deployment owner/service slot；
- linker在structural validation前读取artifact-controlled word/pool/relocation index。

## 15. 非目标

- 不提供任意package function的透明RPC。
- 不让所有package public API都强制boundary-safe。
- 不实现RemoteBoundary、service级进程隔离或独立扩缩容。
- 不定义历史artifact、manifest或数据库内容的兼容迁移。
- 不在本文冻结ServiceContract authoring文件格式、deployment YAML字段名或CLI命令；这些表面语法必须
  在保持本文owner与数据流不变的前提下另行定义。
