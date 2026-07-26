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
`skiff-service-protocol-v2:sha256:<64 lowercase hex>`。Register frame不得另带`protocolVersion`：
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
