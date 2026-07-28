# Runtime Deployment Topology

本文定义第一版RuntimeAssembly放置、replica、Router/Runtime bootstrap、共享artifact filesystem、HTTP
实例限制与未来跨assembly扩展。Package、ServiceContract、ServiceDeployment和RuntimeAssembly本身的语义由
[`package-service-contract-deployment.md`](package-service-contract-deployment.md)负责；release/registry
事务由[`release-registry.md`](release-registry.md)负责。

这些规则是当前部署/runtime拓扑，不是语言类型或Package API语义。

## 第一版 Assembly 放置

第一版每个environment只有一个active RuntimeAssembly，root set是该环境全部active services。每个runtime
replica加载完整相同assembly：

- Package code在replica内只链接一次；
- service binding全部解析为`InProcessBoundary`；
- 每个service拥有独立ActivationContext；
- 每个ActivationContext拥有自己的service/config/state binding vector，共享Package executable不共享这些
  bindings；
- replica内共享的是只读code/type/link image；activation-owned config view、state handle、callback table与
  mutable runtime state不得因PackageBuildId相同而共享；
- replica之间heap、CPU调度、request lifecycle与failure独立；
- MongoDB、Redis等外部数据层按deployment配置共享。

该模型可以整体增加CPU、内存和副本可用性，但不能单独隔离或扩缩某个service；一个service的CPU/memory故障
可能影响同replica内其它service。第一版接受该限制，并要求assembly admission、health、drain与atomic reload
可观测。

## Activation 与 Generation

Canonical runtime connection上的actor、spawn及其它跨request control frame必须显式携带当前
ActivationIdentity，至少包含assembly identity、generation、runtime replica与deployment revision。Runtime
从发起动作的当前ActivationContext填充；callback、continuation或spawn source不能重建、删减或用ambient
connection state替代。

`runtime.register.serviceProtocolIdentity`必须原样携带canonical
`skiff-service-protocol-v5:sha256:<64 lowercase hex>`。Register frame不得另带`protocolVersion`：
transport版本只由frame `schemaVersion`表达，禁止从ServiceProtocolIdentity前缀推导或兼容读取第二份版本。

Router先把frame绑定到发送者的exact assembly registration，再按active或draining generation snapshot验证
完整identity：

- active generation可以发起新控制动作；
- 被request、stream、WebSocket或callback显式pin住的draining generation只在pin生命周期内继续使用原
  ActivationContext；
- 未注册sender、identity缺失/歧义、tuple不匹配、generation已完成drain或只有serviceId/legacy register
  fact时fail closed；
- actor/spawn response按同一request与sender correlation返回，Router不恢复service/build inference。

## Artifact Filesystem Bootstrap

Router与Runtime通过共享artifact filesystem装载immutable records，不依赖或感知registry service：

- Router配置唯一`artifactsPath`与`serviceDb.mongoUrl`；
- Runtime连接后，Router在任何activation/register前发送一次连接级bootstrap，包含规范化绝对
  `artifactsPath`、`serviceDb: { mongoUrl }`与`http: { maxResponseBytes }`；
- 同一连接内bootstrap缺失、重复冲突或变更都fail closed；
- Router与Runtime可以位于不同机器，但路径必须具有相同字符串与内容语义；当前production用共享网络文件
  系统实现；
- Router只读取release/assembly routing projection，不解析或链接Package executable；
- Runtime读取选定RuntimeAssembly及其精确PackageArtifact/ServiceDeployment闭包，完成link与加载；
- immutable record先完整写入并校验identity，再原子更新pointer；reload不接受半写入record。

`artifactsPath`和`serviceDb.mongoUrl`是部署拓扑配置，不进入PackageArtifact、ServiceContract、
ServiceDeployment或RuntimeAssembly identity。Runtime不为二者另设文件配置、环境变量或默认值。

Runtime持有bootstrap DB transport binding不表示所有activation获得DB。只有声明并由deployment绑定DB
requirement的ActivationContext才能得到`std.db` capability；service代码看不到provider URL。

## Activation Prepare Budget

RuntimeAssembly activation是Router协调的控制面事务，不是external business request。Router operator
配置使用：

```yaml
activation:
  prepareTimeoutMs: 120000
```

`activation.prepareTimeoutMs`缺失时默认`120000`毫秒；显式值必须是正safe integer，零、负数、小数、
字符串、对象和超出safe-integer范围的值都在配置边界fail closed。该预算从coordinator开始一次prepare
事务起计算，覆盖participants完成candidate resolve/load/link/admit并返回prepared ACK的等待时间。
只有这个预算到期时，coordinator才以timeout原因abort该pending activation，并让
`POST /__skiff/activate-assembly`返回504。普通reject、disconnect、CAS冲突或admission失败仍使用各自
已有的fail-closed结果，不能伪装成prepare timeout。

`requestTimeoutMs`只限制external business request；deployment `policy.timeoutMs`只会进一步收紧该
request的effective deadline。两者都不参与assembly activation。Test-runner或其它activation client必须
拥有独立deadline，并严格大于Router的prepare budget；默认prepare budget下建议使用`150000`毫秒。这样
client不会在Router仍合法等待participant时先断开，也不会反过来延长Router事务预算。

WebSocket generation release使用独立release timeout，不继承business request或activation prepare预算。
当前契约不要求公开该release timeout为operator配置，但禁止从`requestTimeoutMs`或deployment
`policy.timeoutMs`派生。旧的跨域timeout binding直接删除，不兼容读取。

## Service-scoped External Ingress

Router外部的ingress可以按HTTP Host、域名或其它平台规则选择service坐标，并注入
`x-skiff-service`与`x-skiff-version`。该映射不属于RuntimeAssembly，也不在Skiff Router内重复实现；
原始HTTP Host只作为request业务metadata继续传递。

Router收到HTTP request或WebSocket upgrade后必须：

1. 严格解析两个可信selector header；
2. 在active RuntimeAssembly中按`serviceId + contractVersion`选择唯一精确
   `ServiceDeploymentRef`；
3. 在该deployment内按HTTP `(protocol, method, path)`或WebSocket `(protocol, path)`选择
   gateway entry；
4. 把精确deployment、assembly identity/generation和gateway entry写入Router到Runtime frame。

Runtime不得从Host、path、latest pointer、service display name或ambient registration重建deployment。
它只接受当前admitted activation中逐项匹配的精确deployment。WebSocket连接在upgrade时固定同一个
deployment与generation；后续JSON-RPC method也只在该pin内解析。

同一assembly中不同service可以共享相同method/path。相同`serviceId + contractVersion`不得同时解析为
多个deployment revision；同一service内部重复selector、缺失/非法header、未知坐标、歧义坐标或
跨deployment frame substitution全部fail closed。

这次路由模型变化使用ServiceDeploymentInput v5、ServiceDeployment/DeploymentArtifact v4、
RuntimeAssembly v3和runtime frame v2硬切。GatewayEntryIdentity v2、ServiceProtocol、Package identities
与WebSocketEntryId不因路由scope变化而升级；旧Host-bearing route、裸全局ingress和旧frame不兼容读取。

## Router HTTP 实例限制

Router配置：

```text
http:
  port: positive integer
  maxRequestBytes: positive integer
  maxResponseBytes: positive integer
```

两项大小是Router实例必填规则，没有隐藏默认值或per-service override：

- Router在读取完整request body前执行`maxRequestBytes`；
- Runtime按bootstrap中的`maxResponseBytes`尽早停止生成过大response；
- Router在external HTTP边界再次校验；
- HTTP streaming按同一response生命周期累计，不能通过拆chunk绕过；
- WebSocket不复用这两个字段。

## 未来多 Assembly

若需要独立扩缩容，平台可以为不同root set生成多个RuntimeAssembly。Assembly projection届时把完整本地闭包
拆成`LocalExecutableClosure`与`RemoteBindingRefs`；只有跨assembly service edge选择`RemoteBoundary`，
远端provider不进入本地code closure。

该扩展不改变PackageArtifact、PackageSchema或ServiceContract。它需要新增RemoteBoundary transport、
admission、stream/cancel/error parity及跨assembly availability策略，不能靠Router fallback把缺少的本地
provider临时转成远端调用。
