# Package、Service Contract 与 Deployment 架构

本文定义Skiff长期目标中的代码编译、service协议、deployment装配与runtime执行边界。它是
compiler、artifact、runtime、router和registry共同遵守的canonical架构契约，不是实现计划。本文冻结
manifest owner与跨层数据流；精确YAML shape由`../reference/service-yml.md`冻结，CLI拼写不在本文定义。

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
- ServiceContract：`service.yml`从同一个package公开API中选择哪些callable作为service调用，以及跨boundary
  的语言语义。
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
   `http.yml`/`websocket.yml`和所选
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
14. concrete executable的suspension summary由body、调用图和内建等待点推断。concrete public Package
    callable保留该summary作为Local ABI fact；interface requirement与conformance不拥有或比较该位。
    service call自身是caller的潜在挂起点，ServiceContract不携带provider内部summary，也不从它派生
    protocol identity或operation级内部停止类别。
15. external ingress分两阶段选择。Router外部的ingress可以按HTTP Host等平台规则映射service坐标，并向
    Router注入可信`x-skiff-service`与`x-skiff-version`；Router必须先用这两个header选择active
    RuntimeAssembly中的唯一精确`ServiceDeploymentRef`，再只在该deployment内按
    `IngressSelector`选择gateway entry。
16. HTTP Host不是`IngressSelector`字段，不参与Router中的service、deployment或handler选择。原始Host仍随
    标准HTTP request envelope进入业务metadata，handler可以读取；这不赋予它路由语义。Skiff不拥有
    Router外部的Host映射实现，也不在Router中重做local ingress。
17. `IngressSelector`只在一个精确deployment范围内有意义。HTTP selector是
    `(protocol, method, path)`，WebSocket upgrade selector是`(protocol, path)`；JSON-RPC method继续在
    已pin的WebSocket entry/deployment generation内选择。不同service可以声明相同selector，同一service
    内重复selector必须失败。
18. 缺失、非法或歧义的service/version selector、同一active assembly中同一
    `serviceId + contractVersion`的多个deployment revision、以及Router到Runtime的跨deployment替换都
    fail closed。Router发出的request frame必须携带精确deployment；WebSocket连接同样固定精确deployment
    与generation，不能从Host或ambient connection state重新推导。

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
确定性生成。`config.*.yml`只绑定已经声明的
config/secret/state/resource requirement，不改变package/service dependency graph。

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
`(http, method, path)`；它不是跨assembly的裸全局key。

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
  config/resource/runtime capability requirements
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
GatewayEntryIdentity、ServiceDeployment及其后续assembly。

`PackageLocalAbi`描述同一linked program内的public symbol、canonical signature、nominal type、
public instance、const与executable link信息。concrete public callable的canonical signature包含其推断
suspension summary，供依赖Package编译调用图；interface method requirement只包含调用形状，不复制该
summary，conformance也不比较它。Local ABI允许同一heap引用、alias、原地mutation和其它只在local code
composition中成立的值。

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
就是调用点current execution deadline，已经包含caller request deadline和外层`timeout(...)`的收紧；
需要更短调用预算时由caller显式使用`timeout(...)`。Deployment `policy.timeoutMs`只属于external
ingress/request policy，不复用为内部service call的callee默认值。

具体config/state/native capability requirement和完整may-effect（包括concrete suspension summary）属于
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
- 在同一assembly内只链接一份代码，由多个activation context调用。

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
  ingress: serviceLocalIngressSelector -> gatewayEntryKey
  config/secrets bindings
  state/DB/actor/queue ownership
  external request timeout/resource policy
  activation lifecycle bindings
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

Router必须严格解析`x-skiff-service`与`x-skiff-version`，并在当前active assembly内解析出恰好一个
`ServiceDeploymentRef`。header缺失、重复冲突、格式非法、未知坐标或解析歧义都不能继续按Host、path、
display name、latest pointer或其它deployment猜测。直接向Router发送这两个header的receipt就是Skiff
生产边界证据；Host到header的映射属于Router外部ingress，不在Skiff内重复实现。

选择deployment后，HTTP只按`(protocol, method, path)`，WebSocket upgrade只按
`(protocol, path)`查询该deployment的`ingress`。因此Relay与AIHub可以同时声明
`GET /v1/models`；它们由不同service坐标定界。同一deployment内重复selector仍是authoring/projection错误。
请求的原始Host、URL、headers等继续作为标准HTTP envelope传给业务代码，但Host不能改变已选择的deployment
或gateway entry。

Router到Runtime的dispatch必须携带精确`ServiceDeploymentRef`、assembly generation与gateway entry事实。
Runtime只接受当前admitted activation中逐项匹配的deployment与entry，禁止用同service的另一revision、
同path的另一service或ambient registration替换。WebSocket upgrade先执行同一deployment选择，再把精确
deployment与generation固定到connection；连接内JSON-RPC method只在该pin内解析。

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
| RuntimeAssembly schema / identity marker / prefix | `skiff-runtime-assembly-v3` / `skiff-runtime-assembly-identity-v3` / `skiff-runtime-assembly-v3:sha256` |
| Router↔Runtime frame schema | `skiff-runtime-frame-v2` |

`GatewayEntryIdentity`/GatewayEntry保持v2；ServiceContract/ServiceProtocol、Package artifact/build/local
ABI/schema与WebSocketEntryId不变。旧Host route字段、裸全局ingress key、旧assembly/wire不得兼容读取。

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

同一request、同一linked assembly内的wrapper调用`PackageDirect` stream producer时，stream handle必须继续
属于当前request已有的`StreamRuntime` registry；package call不能为该handle新建registry，也不能把handle
当作boundary value重新materialize。这里不要求producer与consumer使用同一个`RequestHeap`：两侧heap可以
不同，stream item通过既有`StreamInternalItem`及canonical item materialization从producer heap搬运到
consumer heap。只有6.2定义的service call boundary继续按value plan rematerialize参数、item与返回值，
不得把package-local registry共享规则扩张到service boundary。

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
使用同一error/stream/callback contract与统一runtime内部停止语义，再materialize返回值。它不能因为
地址可见就直接传递本地引用或method table。

Consumer lowering不会链接provider executable，也不生成伪PackageArtifact。它保存结构化调用引用：

```text
ServiceCallRef
  serviceRequirementSlot
  contractOperationId
  expectedProtocolIdentity
```

source effect analysis只要解析到`ServiceCallRef`就把该call site视为`maySuspend=true`；ServiceContract
operation不提供callee summary位。InProcessBoundary即使绑定到当前立即返回的provider executable，也不能
把该调用重新分类成package direct call。caller只在response尚未就绪而实际等待时释放actor executor。
provider runtime若需其concrete summary选择内部lane，只能从deployment绑定的PackageArtifact取得。

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

HTTP、WebSocket及未来其它gateway entry不是service boundary call。Router先按可信
`x-skiff-service + x-skiff-version`选择当前active assembly中的精确`ServiceDeploymentRef`，再按
`ServiceDeploymentRef -> IngressSelector -> GatewayEntryKey -> GatewayEntryIdentity`进入对应
activation，并由deployment gateway entry binding中冻结的精确
`PackageCallableId`执行handler；它不经过service dependency slot，也不伪造`ContractOperationId`。

Ingress仍复用普通语言函数、Package本地链接、ActivationContext、错误通道和runtime内部停止机制，但不复用
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

`std.websocket`发送从当前`ActivationContext`解析当前service deployment中唯一的WebSocket entry，不能按
path、display name或任意字符串猜entry；零entry或损坏的多entry状态fail closed。Skiff代码也可以向一个
**精确connection id**发起request，外部peer接受该request并在同一socket上返回response；该response只恢复
原调用。Peer向Skiff发起的request则按socket pin住的deployment/activation generation解析method并创建新的
runtime ingress。两个方向共享frame codec但不共享pending identity namespace。

WebSocket transport、request/response broker与编码配置分层：

```text
业务调用
  -> request/response语义与pending生命周期
  -> 编码配置
  -> WebSocket text或binary frame
  -> TCP
```

Broker只拥有request identity、pending、deadline/内部停止、connection/generation归属和容量限制，不把
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
相同connection/generation/direction上仍active或仍在bounded settled tombstone中的id不得再次
发起request；重复id是`1002`协议错误并关闭socket，不能先返回一个同id错误再让旧execution或晚到result
作用于新request。Tombstone到期或按容量驱逐后可以复用该id；peer应优先生成connection-lifetime唯一id。
平台生成的outbound string id在同一connection generation内不得复用。

第一版所有notification都没有业务或平台取消语义。Pending key至少包含direction、connection id、
socket/generation identity、配置id与request id；response必须来自原connection，unknown、duplicate或
跨generation的response不能命中其它调用。配置adapter只解析JSON-RPC request/response控制字段；
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

- `(websocket entry id, jsonrpc-2.0-text, method)`必须在socket pin住的deployment generation中精确命中
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
  connection/generation上仍在运行的inbound execution；晚到完成被丢弃，但这只是runtime内部停止，
  不产生cancel response或rollback承诺。
- 任何JSON-RPC notification都不调用用户代码、不产生response；平台保留前缀也没有特殊取消语义。
  即使notification method与已声明request method同名也一样，只记录有界诊断。Binary frame不是本配置的一部分；没有用户raw
  receive时以`1003`拒绝。
- Response object只允许恢复Skiff发起的outbound pending；request object只允许创建上述declared ingress。
  畸形/伪造response、wrong connection/generation或unknown outbound response id属于`1002`协议错误，不能
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
- 生命周期到顶层request结束，stream存在时延长到stream关闭；内部停止或owner退出会提前失效；
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
它还为全部function/impl method拥有exact executable signature facts（包括推断的suspension summary），并为
全部interface operation拥有不含该summary的exact requirement facts；public callable signature只是
executable事实表按public path产生的view。interface conformance只比较receiver、参数、返回及其它
requirement-owned调用形状，不比较suspension。Lowering只消费typed source facts，把exact source type投影为
File IR execution representation。package direct call降低为`PackageCallable` target；service call降低为
`ServiceCallRef`。
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
最终assembly只为ServiceRequirement选择deployment。

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
- AssemblyIdentity：完整resolved deployment/package graph。
- RuntimeReplicaId：某个assembly实例，不进入artifact contract。

任何identity都不能因为display string相同而互换。ServiceProtocolIdentity包含operation实际引用的
PackageSchemaTypeId/closure identity，但不包含provider implementation build或deployment字段；
AssemblyIdentity可以记录最终选择的build作为复现事实，但不能回写consumer requirement。

suspension变化的identity边界固定如下：

- concrete public Package callable的`maySuspend`变化保持`PackageCallableId`稳定，但改变
  `PackageLocalAbiIdentity`与`PackageBuildId`，直接package依赖方必须重编译；
- interface requirement没有suspension位，implementor summary变化不改变interface requirement、
  conformance或interface method stable identity；
- service operation保持`ContractOperationId`稳定；request/response/stream/callback与公开错误语义不变时，
  provider内部summary变化不改变`ServiceProtocolIdentity`；
- provider build、ServiceDeployment revision/identity及包含它的RuntimeAssembly identity可以随实现变化；
  这些implementation identity不得被解释为protocol变化；
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
拥有相同module/type也因PackageArtifactRef不同而保持无冲突；物理collection映射仍由各自
PackageRequirement/PackageBinding edge单独投影和校验。

同一stateful `PackageBuild`可因direct与transitive dependency形成多条真实edge。Assembly/loader先解析每条
edge，再按以下规则形成activation中的active collection projection与metadata owner：

- exact `PackageBuild`相同，且resolved source→target collection mappings与所有owner-relevant facts的
  canonical表示相同：合并成一个active projection与一个metadata owner；
- exact build相同但resolved mapping不同：拒绝；
- build不同但指向同一physical target：拒绝；
- dependency projection与service root collection冲突：拒绝。

“active projection”是一次activation内最终生效的collection mapping/metadata owner，不是另一份数据库或
另一条package binding。合并不改变原始dependency graph，也不抹去用于诊断和identity的edge事实。
`config.skiff-test.yml`仍是test activation state binding的唯一来源；direct/transitive菱形不能生成第二份
config、namespace或state owner。

该身份覆盖所有DB operation target、`DbQuery`、lease claim、lease state read和claim write guard。
Transaction本身只是当前service DB上的执行边界，没有独立target；内部operation各自携带目标。缺失link/type/
DB declaration、ABI或build不匹配、cross-artifact substitution都必须在link/admission阶段fail closed。

这是当前未发布artifact模型内的同代hard cut，不改变File IR v9、PackageArtifact v9、Package local ABI v7
或ServiceContract v5代际；不增加兼容reader、fallback或旧target双读。

## 11. Config、State 与 Resource Owner

Package可以声明运行所需config path、外部resource capability、DB/schema或native adapter requirement，
但不拥有环境中的实际值和state namespace。普通package可以在`package.yml.services`声明service
dependency；这使其可复用业务编排在最终宿主service的ActivationContext中解析provider，不把具体provider
写入PackageArtifact。

Service source的`config.*.yml`选择或提供：

- 提供config/secrets；
- 选择DB、Redis、actor、queue等外部state namespace；
- 定义timeout、quota、principal与明确支持的service lifecycle policy。

并发不属于service lifecycle或deployment policy。Service profile中的`lifecycle.maxConcurrency`非法；
ServiceDeployment、DeploymentArtifact、RuntimeAssembly和artifact identity都不得复制并发上限。初期唯一
并发配置是`router.yml`现有`runtime`段的required正安全整数`maxConcurrency`，Router按每条Runtime
WebSocket连接统一限制所有普通pending request；Actor/control frame不计。该门禁不做动态CPU、内存或
数据库资源估算，满载立即overload且不排队。

`timeout`是可选的deployment override。profile缺省或显式`null`都表示不覆盖平台/外层request
deadline；生成的`DeploymentPolicy`不包含`timeoutMs`。只有显式的正整数毫秒值才生成
`timeoutMs`，零、负数、小数、字符串或对象都必须fail closed。tooling不得为了通过artifact校验而
填入虚假的默认timeout。External HTTP中，Router以平台HTTP request上限和该override的较小值生成
request deadline；Host从已admit activation读取同一policy并再次收紧、执行。Deployment override只能
缩短平台/外层deadline，不能放宽，也不能因wire遗漏或伪造而失效。

Router实例的`requestTimeoutMs`同样只定义external business request的平台上限。它和deployment
`policy.timeoutMs`都不得参与RuntimeAssembly resolve/load/link/admit、participant prepare ACK、
activation commit/abort或WebSocket generation release。Assembly activation是控制面事务，不是一个
service request；把业务request deadline复用为activation deadline会让部署耗时被service policy意外改变。

tooling把所选profile与精确PackageArtifact、生成的ServiceContract及闭合dependency resolution投影为
ServiceDeployment。profile不得增加/删除`package.yml`中的package或service dependency。

Package静态资源随PackageArtifact发布，并按当前执行callable的package owner读取。ServiceDeployment
没有用户代码资源；deployment-only证书、secret和环境文件属于activation输入，不进入code artifact。

同一个PackageArtifact被两个service使用时，代码和静态资源可共享，ActivationContext、config、state
owner和service-owned lifecycle必须分开；Router连接级并发门禁不随service复制。

## 12. RuntimeAssembly 与扩容

RuntimeAssembly由一个操作面选择的精确root deployment set做依赖闭包：

```text
RuntimeAssembly
  roots: ServiceDeployment[]
  resolvedServiceDeployments
  resolvedPackageArtifacts
  ingressByDeployment: (ServiceDeploymentRef, IngressSelector) -> GatewayEntryKey
  linkedProgramImage
  serviceBindingTemplatesByActivation
  ActivationContext templates
  assemblyIdentity
```

Root set不是developer-authored source config，也没有`assembly.yml`。它的来源按运行场景区分：

- dev sync/watch从watch registry选择的service roots生成各自deployment，再把这些精确deployment refs作为
  roots；一次性开发、测试或验收命令也可以显式传入service roots或deployment receipts；
- production由平台部署状态选择当前environment的精确deployment refs；
- 每个项目的package/service依赖仍只由该项目自己的`package.yml`声明；assembly projection从roots沿这些
  已编译依赖闭合，不在仓库顶层复制一份依赖图。

因此，一个放置多个项目的源码仓库仍然只是项目集合，不拥有environment assembly。任何tooling为了调用旧CLI
而临时写出的`assembly.yml`都只是待删除的实现adapter，不能成为公共authoring格式、配置owner或验收输入。
Host/domain到service selector的映射属于外部ingress，同样不进入root set或RuntimeAssembly。

Package link与service binding使用不同的可变性边界：

- package requirement在link完成后绑定到不可变`PackageArtifact` identity。最终linked image记录精确
  `PackageArtifactId`；`packageBuildId`可以作为构建过程与诊断标识，但不能成为允许原地覆盖内容的引用。
  package升级必须重新link/build consumer。
- service requirement在consumer compile/link时只绑定`serviceId + exact packageVersion label +
  expectedProtocolIdentity`，不绑定provider package或deployment revision。
- assembly projection为每个service requirement选择一个精确`ServiceDeployment` revision及其不可变
  implementation `PackageArtifactId`。service owner可以在protocol identity不变时发布并激活新的deployment
  revision，不要求consumer重新编译；已经生成的RuntimeAssembly仍记录原选择，不能随pointer漂移。
- assembly projection必须保证每个`serviceId + contractVersion`在一个assembly中只解析为一个精确
  deployment revision。相同坐标出现多个revision是歧义并失败；不同service的相同
  `IngressSelector`合法，因为assembly ingress key由`(ServiceDeploymentRef, IngressSelector)`组成。

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

Activation prepare使用Router operator配置`activation.prepareTimeoutMs`，默认`120000`毫秒。该值必须是
正safe integer；缺失时使用默认值，其它值在Router配置边界fail closed。预算覆盖一次prepare控制事务等待
participants完成resolve/load/link/admit并返回ACK的时间；只有该预算到期时，coordinator才为timeout原因
abort pending activation，并由control endpoint返回504。它不进入任何ServiceDeployment、
DeploymentPolicy、RuntimeAssembly或artifact identity。

调用activation control endpoint的test-runner/client必须使用独立client deadline，且严格大于Router
prepare budget；默认prepare budget下建议使用`150000`毫秒client deadline。WebSocket旧generation release
另有自己的release timeout；它也不得读取`requestTimeoutMs`或deployment `policy.timeoutMs`。是否公开
release timeout配置不由本契约决定，但该预算必须与business request和activation prepare三者解耦。

这是未发布系统的hard cut：删除把`requestTimeoutMs`或deployment timeout绑定到activation/release的旧
错误路径，不保留alias、fallback或dual-read。

`publish`是操作：校验typed artifact、写入不可变内容、更新允许更新的pointer。它不产生
`Publication`对象，也不要求四类artifact实现共同kind enum。registry可以在一个事务/workflow中发布
多个artifact，但每个artifact保持独立schema与identity。

## 14. Fail-closed 条件

以下情况必须在compile、deployment projection或assembly阶段失败，不能留到请求时猜测：

- service API schema不闭合或operation identity冲突；
- 自动生成的deployment operation缺失、重复、额外或descriptor不匹配；
- selected callable boundary unavailable；
- `service.yml`仍包含`http`/`websocket`，普通package出现`http.yml`/`websocket.yml`，或external
  manifest shape/selector重复、非法；
- `http.yml`或`websocket.yml`的handler/pre/guard无法解析到当前Package callable，或adapterArgs与
  linked signature不匹配；
- `websocket.yml.jsonRpc` method重复、非法，params/result不是可执行的unary JSON codec，或使用
  未授权adapter source；
- ingress仍指向`ContractOperationId`、要求handler先进入`api.yml`，或gateway entry identity与typed
  projection不一致；
- HTTP/WebSocket service route仍声明Host，或assembly把不同service的相同selector作为裸全局collision；
- 同一service deployment内selector重复，或同一assembly为相同`serviceId + contractVersion`解析出多个
  deployment revision；
- Router selector header缺失、重复冲突、非法、未知或歧义，或request frame中的deployment/entry与
  admitted activation不匹配；
- service/package dependency缺失、版本或identity不匹配；
- test-only topLevel DB target缺少expected package build、type implementation link、provider File IR type或
  同type DB attachment，ABI/build不匹配，或被替换为其它artifact的同名type；
- runtime DB target只能靠`typeName`、module suffix或全图扫描定位，或consumer复制的schema/collection/
  recoverable metadata与provider File IR发生分叉；
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
