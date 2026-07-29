# Skiff API YAML Reference

## 本文负责 / 不负责

本文负责定义 `api.yml` 的目标语义：Package 如何显式声明 public API surface、public path 如何绑定到
当前 Package 的 source symbol、Package Local ABI 如何从这些显式 public roots 派生，以及
`service.yml`如何从同一个Package API graph选择ServiceContract roots。

本文不负责 compiler 迁移步骤、历史 `export` surface 语法、artifact JSON 字段细节、registry 操作流程、
runtime 调度或完整 YAML parser 实现。

## 1. Source Layer

每个 Package source root 必须放置一个固定名字的 `api.yml`。Service 首先是 Package，因此使用同一份
`api.yml`。`service.yml`只按public path选择哪些已有callable成为service-to-service API，不复制source
selector、签名或类型。

`api.yml` 是 source-layer metadata，不是 `.skiff` source module：

- 它不参与 `root.*` module namespace。
- 它不能被 `import` 或从源码引用。
- 它不生成 File IR unit。
- 它必须参与 Package build identity。

空 API 必须显式写成顶层空 mapping `{}`，表示该 Package 没有 Package public API，也不生成
service-to-service operation。空文件会被 YAML 解析为 `null`，不是合法 API mapping；缺少
`api.yml` 也必须报错。这样 compiler 能区分“作者明确选择没有 public API”和“漏交 API 声明”，并且永远
不得从其它 manifest 推断 public API。任何Package都可以显式选择空API；它不能向dependency提供可写
public symbol。Service可以据此只暴露external ingress或只承担内部/test运行角色，其HTTP/WebSocket
handler仍分别由`http.yml`/`websocket.yml`直接引用。

`package.yml`不列public symbol。`service.yml.serviceCalls`可以引用`api.yml`已经声明的public callable
root，但不声明新的public symbol，也不做rename/namespace projection；public API的唯一符号事实来源仍是
`api.yml`。`http.yml`/`websocket.yml`可以为external ingress显式引用非public handler/pre/guard，这只
创建gateway entry，不把这些source symbol加入public API。`service.yml`不得内联HTTP/WebSocket配置。

Skiff source declaration 不使用 `export` 关键字表达 public visibility。普通 source file 没有 public
visibility marker；source file 不是包内 privacy 边界。

## 2. YAML Shape

`api.yml` 顶层必须是 mapping。mapping key 是 public path segment。普通 public symbol leaf 是 source
selector：

```yaml
decode: decode.decode
LlmRequest: types.LlmRequest
```

Scalar leaf声明Package public binding。Function、type、alias、interface与普通const都只使用这一种
scalar写法；`api.yml`不再提供带`source`或`serviceCall`字段的function object leaf。Service若要选择
其中的callable，必须在`service.yml.serviceCalls`中引用其public path。

嵌套 mapping 表达 dotted public path：

```yaml
http:
  Request: http.HttpRequest
  sse: http.sse

namespace1:
  namespace2: dir1.fileB.func2
```

上述最后一项定义 public path `namespace1.namespace2`，绑定到 source selector
`dir1.fileB.func2`。

YAML key 必须是单个 identifier segment。第一版不接受 dotted key；需要 dotted public path 时使用
嵌套 mapping。

source selector 是当前 production source set 内的 `modulePath.symbol`：

- selector 至少有两个 segment。
- 最后一段是 top-level source symbol name。
- 前面的 segment 组成 source module path。
- selector 不带 `root.` 前缀；语义上等价于解析 `root.<modulePath>.<symbol>`。

public instance 使用显式 object leaf，而不是从普通 public const 自动派生：

```yaml
managedLlm:
  const: root.llm.managedLlm
  interfaces:
    - root.llm.ManagedLlmService
```

public instance leaf 的 `const` 必须解析到当前 production source set 的 top-level const；`interfaces` 必须
是非空列表，每一项都是 public 或 imported public interface selector，可带 fully substituted type args。
`api.yml` 左侧完整 path 是 `public_instance_key`。嵌套写法：

```yaml
llm:
  managed:
    const: root.llm.managedLlm
    interfaces:
      - root.llm.ManagedLlmService
```

生成的 `public_instance_key` 是 `llm.managed`，不是 leaf `managed`。

`const`与`interfaces`是public-instance object leaf内的保留字段，不是service-call选择字段。该instance及
其methods始终进入Package Local ABI；只有`service.yml.serviceCalls`列出该public instance path时，
`interfaces`中显式列出的所有methods才成为service-call roots。第一版不维护method include/exclude清单；
只希望暴露部分methods时，应定义更窄的interface或显式wrapper functions。

## 3. Public Path

public path 由 `api.yml` 左侧 mapping path 唯一决定。source module path 只是实现组织细节，不进入
package user source path 或 service protocol source path。

例如：

```yaml
Request: internal.protocol.LlmRequest
client:
  decode: codecs.json.decode
```

定义两个外部可写名字：

- `Request`
- `client.decode`

它们的 source selectors 分别是 `internal.protocol.LlmRequest` 和 `codecs.json.decode`。

同一个 Package API graph 内 public path 不得重复。不同 symbol kind 也不能共享同一个 public
path。

第一版同一个 source nominal declaration 只能拥有一个 canonical public path；重复把同一个 record、
representation、union 或 interface declaration 绑定到多个 public path 必须报错。该唯一 path 是其公开
名义身份与（适用时）`PackageSchemaTypeId` stable key 的来源。function 可以被显式绑定到多个 public
path；每个 path 是独立 callable surface，并获得独立 public callable identity。

## 4. Source Selector Resolution

compiler 为当前 production source set 建立 all-symbol `root.*` index。`api.yml` 的 source selector
必须解析到当前 source set 中的 top-level declaration：

- `type`
- `alias`
- `interface`
- `const`
- `function`

第一版不允许直接把 impl method 写成 source selector。method 仍属于 receiver 的 method namespace。

public instance leaf 的 `const` 是唯一允许带 `root.` 前缀的 selector 形态。它必须解析到当前 source set 的
top-level const；type、interface、alias、function 或 impl method 都不能成为 public instance receiver。
该 const 必须有显式 nominal receiver type，receiver type 必须显式 implements `interfaces` 中列出的每个
fully substituted `InterfaceInstantiationRef`。receiver type implements 但未列入 `interfaces` 的 interface
不会被公开。

`api.yml` 不能公开 test source 中的 symbol，不能穿透 package dependency 的 private symbol，也不能直接
公开 `std.*` 或外部 package alias 下的 symbol。需要公开外部能力时，应在当前 Package 中定义明确的
wrapper、type、interface、function 或 const，再由 `api.yml` 公开该当前 source set 的 symbol。

第一版不限制 source selector 只能指向顶层文件或顶层目录下的 source module。是否公开某个内部目录中的
symbol 由 `api.yml` 显式声明决定，而不是由文件位置隐式决定。项目可以对 `internal.*` 等路径提供 lint
或诊断建议，但它不是语言级错误。

## 5. API Graph

compiler 从 `api.yml` 构建当前 Package 的 API graph。每个 leaf 生成一条 public binding：

- public path。
- source module path。
- source symbol name。
- source symbol kind。

public identity 使用 public path。source selector 只用于链接、类型检查、ABI closure 和诊断。

Package API graph 覆盖：

- 显式 public symbols。
- public types / aliases / interfaces。
- public constants。
- public callable functions。
- public instance roots。
- Package Local ABI 中由 public root 可达、但没有独立 public path 的内部类型事实。

Public binding与service-call boundary availability不是一回事。Generic declaration、actor或其它不能生成
PackageSchema的public symbol仍可供package dependency链接；compiler必须为其记录结构化boundary-unavailable
事实。只有被service-call operation实际引用且拥有合法PackageSchema投影的类型进入ServiceContract closure。
某个无关public generic declaration不能让整个Package编译失败。

Public instance 是 API graph 中可作为 binding target 的 receiver root。第一版中 public instance
只能来自 `api.yml` 显式公开的 top-level `const`；该 const 必须有显式 nominal receiver type，且该
receiver type 必须显式 implements 一个或多个 interface。public instance leaf 还必须显式列出 exposed
interfaces；普通 public const 不会自动成为 public instance。instance 自身不等于 operation；它公开的
interface methods 才能 projection 成 public instance method operations。

公开type本身不会把它的任意impl method加入public namespace。普通package `alias`取得的值即使保留该
public type或local-closure-only type的精确名义身份，也只可调用API graph显式公开的public instance
methods；不能由concrete receiver自动发现package-local impl methods。`kind: test` service的
`topLevelAlias`对精确implementation type开放现有impl method namespace，是独立的test-only
implementation view，不改变`api.yml` graph或Package Local ABI的公开面。

`public_instance_key` 是完整 API graph public path。dependency lookup、binding target、
Package Local ABI 的 method source-call key，以及由该 method 生成的 service operation stable key，都使用
该完整 path，而不是 leaf/display name。

## 6. Local ABI 与 Service Schema Closure

显式 public roots 的 callable signature、type body、alias target、interface method signature、const
type 和 public instance metadata 会递归引用其他 named types。compiler 必须自动收集这些引用，但不能把
Package Local ABI closure 与 service-call PackageSchema closure 合并成一个规则。

Package Local ABI 中的 named type 分两类：

- explicit public type：在 `api.yml` 中有 public path，外部源码可以写这个 public name。
- local-closure-only type：只因为 explicit public root 的本地调用形状需要而进入 Local ABI closure，
  外部源码不能直接写这个 public name，但可在类型推断结果中保留其精确名义身份。

local-closure-only type 参与 Local ABI identity、类型检查和 artifact linking，但不会自动扩大 public
namespace，也不会因此自动获得 `PackageSchemaTypeId`。

第一版 service-call boundary 中的每个 named type 都必须在其 owner Package 的 `api.yml` 中拥有
canonical public path，并成功生成 `PackageSchemaTypeId`。ServiceContract 只记录 operations 实际可达的
Package type ids；不得用 local-closure-only identity、source path 或匿名展开代替。

## 7. Package Projection

Package projection 使用当前 Package API graph：

- dependency alias 绑定到 Package requirement。
- caller 只能通过 dependency alias 加 public path 访问显式 public symbols。
- package dependency call 使用 dependency `PackageLocalAbi` 的 source-call index 将完整 source-call
  path 解析到唯一 `PackageCallableId`。
- local linkage 再通过 dependency `PackageArtifact.callableLinks` 解析到 executable 或 const receiver
  target。
- `PackageRequirement` 记录精确 Package 坐标与预期 `PackageLocalAbiIdentity`；call site 和 external-ref
  table 不重复携带同一 ABI expectation。
- public function 和 public instance method都进入 Package Local ABI；其本地 target 只由
  PackageArtifact implementation links 拥有。

未出现在 `api.yml` 的 source symbol 不能被 package caller 书写；它只可能作为 closure-only ABI 节点被
链接。

## 8. Service Projection

Service通过`service.yml`中的`serviceCalls`数组，从同一Package API graph选择service-call roots：

```yaml
serviceCalls:
  - getUser
  - llm.managed
```

数组元素是`api.yml`左侧定义的完整public path，而不是source selector：

- 指向public function时，生成一个public function operation。
- 指向public instance root时，按其explicitly listed interface methods生成public instance method
  operations；不能在数组中单独选择某个instance method。
- dotted public path按字符串书写，例如`llm.managed`；它必须精确解析到一个public root。
- 同一路径不得重复。数组顺序不参与ServiceProtocolIdentity；compiler按canonical public path排序后投影。
- 每个public callable仍生成`BoundaryCallableProjection`，用于说明它技术上能否跨service boundary。
  未被选择的callable无论`Available`或`Unavailable`都只是合法Package API；被选择的callable必须
  `Available`，否则Service projection以全部结构化原因报错，不能静默排除。
- ServiceContract operation 使用稳定 `ContractOperationId` 和 canonical boundary descriptor；
  ServiceDeployment再把该 id 精确绑定到 implementation `PackageCallableId`。
- `ServiceProtocolIdentity` 使用 operation ids/descriptors 与实际可达的
  `PackageSchemaTypeId` closure，不包含 handler、route、deployment 或 implementation build。

service operation返回的对象只携带ServiceContract声明的schema/type身份，不携带provider的
implementation view；consumer不能在该对象上解析provider package-local impl methods。若contract
返回的是interface能力，其调用仍只按contract/interface operation dispatch，不因provider concrete type
存在同名method而改变。

`service.yml.serviceCalls`是第一版唯一service-call选择机制。它只引用callable public paths，不重复source
selector、参数、返回或签名引用的types；compiler从这些roots递归计算ServiceContract的PackageSchema
closure。字段省略或写成空数组时，生成合法的空operation ServiceContract，Service仍可只暴露
HTTP/WebSocket external ingress。一个wire-safe public callable未被该数组选择时就是Package-only API，
不会因`BoundaryCallableProjection::Available`而被远程暴露。

HTTP/WebSocket等external ingress不属于本节的public service-call operation。已冻结的HTTP entry由
`http.yml`直接引用当前Package handler，并生成独立`GatewayEntryIdentity`与typed adapter plan。
WebSocket connect和peer-initiated JSON-RPC method由`websocket.yml`拥有；raw `receive`不是目标业务
entry。`std.websocket.requestJsonToConnection`是Skiff主动调用的平台host operation，其response由编码
无关的平台broker关联；它不要求在`websocket.yml`重复声明outbound method，也不生成ServiceContract
operation：

- handler不要求出现在`api.yml`；
- gateway entry不写入`ServiceContract.operations`，也不进入`ServiceProtocolIdentity`；
- runtime codec读取linked handler signature，compiler从精确handler类型与adapter source生成entry-local
  external schema及codec plan；external manifest不得手写一份重复的业务JSON schema；
- 同一函数同时出现在`api.yml`和external manifest时形成两个显式surface和两种identity，不自动合并。

External schema closure与Package public/type closure严格分开。Compiler只为真正跨external boundary的
adapter source/sink派生wire shape；当前包括typed HTTP body、query/path/header参数与HTTP response，以及
declared WebSocket JSON-RPC handler的params/result。Skiff-originated WebSocket request/response payload
codec来自`requestJsonToConnection<TRequest, TResponse>`调用点的concrete类型，不是external ingress
schema；peer-originated codec来自`websocket.yml.jsonRpc`所选linked handler。两者都不能从raw
`receive`或手写schema推导。`pre`产生的内部context、guard内部值及其它只在runtime adapter与handler之间
流动的值不进入external schema。即使某个跨external boundary的shape来自私有
named type，外部只看到entry-local结构/协议名，不获得该type的Skiff public path或名义identity；compiler
也不得因此把它补进`api.yml`、PackageLocalAbi、PackageSchema或ServiceContract。

上述JSON配置不把WebSocket锁定为JSON-RPC-only transport。Raw text/binary send保持独立；未来binary RPC
必须使用独立、显式版本化的配置和API，不能由manifest或payload形状隐式推断。

第一版external handler/pre/guard不能是generic function declaration；其concrete signature可以包含fully
instantiated generic platform types。两者不能混为“只要出现generic就拒绝”。

`api.yml`不声明每个operation的throw set，compiler也不把推导出的可能错误类型写入
Package Local ABI或ServiceContract operation。任意希望跨service后仍保留原名义类型的错误，必须作为其
owner Package的普通public
type出现在该owner的`api.yml`并满足`SchemaClosed`；它不因此成为某个operation signature的一部分。

source module path 不作为 service protocol identity。HTTP path、WebSocket route key、handler selector、
adapterArgs、gateway entry identity、timeout、routing revision 和 runtime activation 属于external
ingress projection或部署metadata，不属于`api.yml`或`ServiceProtocolIdentity`。

## 9. Validation Summary

必须报错的情况包括：

- 缺少 `api.yml`、文件为空或顶层不是 mapping。
- public key 不是合法 identifier segment。
- leaf 不是合法 source selector string。
- function使用object leaf而不是string source selector。
- source selector 少于两个 segment。
- source selector 无法解析到当前 production source set 的 top-level symbol。
- source selector 指向 test source、dependency symbol、`std.*` symbol 或 impl method。
- public path 重复。
- 同一个 source nominal declaration 被绑定到多个 public path。
- public path 与保留 public namespace 规则冲突。
- public instance leaf 缺少 `const` 或非空 `interfaces`。
- public instance object leaf包含`const`、`interfaces`之外的字段。
- public instance `const` 不是当前 source set 的 top-level const。
- public instance interface selector 不是 public/imported public interface，或 generic type args 未 fully
  substituted。
- receiver concrete type 未显式 implements listed interface。
- public instance exposed interfaces 中出现重复 canonical `InterfaceInstantiationRef`。
- 同一个 public instance 暴露的多个 interface 中出现相同 source method name。
- public function path 与 `<public_instance_key>.<method>` 在 Package Local ABI source-call index 中冲突。
- `service.yml.serviceCalls`不是字符串数组、包含非法/重复public path，或引用不存在的public path。
- `service.yml.serviceCalls`引用type、alias、interface、普通const或其它非callable public root。
- 被`service.yml.serviceCalls`选择的function或instance method的boundary projection为`Unavailable`。

不应作为语言级错误的情况：

- source selector 指向深层目录下的 module。
- source selector 指向名字包含 `internal` 的 module。
- 只进入 Package Local ABI 的 local-closure-only type 未显式列入 `api.yml`。
