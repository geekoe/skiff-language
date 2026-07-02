# Skiff Publication / Artifact Reference

## 本文负责 / 不负责

本文负责定义 Skiff Publication、package/service、API graph、local vs remote linkage、
service protocol identity、ingress entry identity、dependency lock、artifact/unit/bundle 边界，
以及 runtime program / linking 的稳定概念。

本文不负责 registry 操作流程、命令手册、完整 YAML / JSON schema、implementation plan、
历史兼容格式或测试 fixture。发布和 registry 的操作模型属于 architecture 层。

## 1. 生命周期分层

Skiff 发布语义分成四层：

- Source layer：`.skiff` source tree、`package.yml`、`service.yml`。
- Publication layer：compiler 从 source、manifest、dependency 和 public API projection 得到的语言对象。
- Artifact layer：可信 compiler / artifact writer 产生 typed units、bundle、index 和 build record。
- Runtime layer：runtime 解析具体 typed units，dynamic link 成 `RuntimeProgram`。

这些层不能互相替代：Publication 不是 release 或 artifact bundle；Package 不是远程路由对象；
Service version 不是 package version；Bundle 不是源码层概念；`buildId` 不是 manifest 输入字段。

## 2. Publication
Skiff 的语言级发布对象叫 **Publication**。

Publication 包含：

- stable publication id。
- publication version。
- source modules。
- public API metadata。
- package dependency declarations。
- public API graph。

Publication 不包含 runtime instance、router route table、registry pointer、service release pointer、
activation `buildId` 或部署环境中的 resolved config value。

`package.yml` 和 `service.yml` 都先读成 Publication manifest。service 在 Publication core 之外
叠加 service runtime spec；package 没有 service runtime spec。

`Project` 容易和 repo / workspace / CLI project 混淆；`SourceSet` 只表达文件集合；`Bundle` 是
artifact 聚合。因此稳定概念使用 Publication。

### Publication ABI Unit

Package 和 Service 对同一 public API graph 生成同一种 contract shape：`PublicationAbiUnit`。它嵌入
`PackageUnit` 和 `ServiceUnit`，不作为额外 artifact lookup 路径。package/service 的差异只在
`implementationLinks`、service operation table、route/runtime metadata 等 linkage 层。

`PublicationAbiUnit` 至少包含：

- `publicationId`、`version`、`abiIdentity` 和 schema version。
- public API graph bindings：public path、source module path、source symbol 和 symbol kind。source
  provenance 只用于诊断，不参与 dependency linkage。
- ABI declarations：public/closure-only types、aliases、interfaces、callables、constants。
- operation exports：public function 和 public instance method 的 `OperationAbiRef`。
- `operationAbi`：以 `operation_abi_id` 为 key 的 canonical public signature / schema closure /
  stream-effect-throw-config metadata 表。link/load、provider decode 和 binding compatibility 只能从该表
  查询 public signature。
- `sourceCallOperationIndex`：dependency alias 之后的完整 source-call path 到唯一 `OperationAbiRef` 的
  索引。public function path 与 public instance method path、嵌套 public instance path 或重复 alias
  组合发生冲突时，producer projection 必须报错，consumer 不做 fallback 或 shadowing。
- public instance exports：`public_instance_key`、implemented `InterfaceInstantiationRef`、method
  operations 和 source-call method index。method index 只用于 compiler/linker 解析，不是 runtime
  dispatch key。
- package public `any I` entry boundaries：package entry signatures may carry same-process
  interface boxes; remote boxes are locked by the owning service dependency lock when boxed.
- public/closure-only nominal type 到 `InterfaceInstantiationRef` 的 explicit conformance facts。

`PublicationAbiUnit.abiIdentity` 是输出字段，不是 hash 输入。canonical hash 输入包括 ABI declarations、
operation identity projections、`operationAbi` public signatures、`sourceCallOperationIndex`、
public instance exports、package public `any I` entry boundary facts、schema closure、public conformance facts 和 public
contract effect/config metadata。以下内容不进入 hash：`abiIdentity` 自身、publication id/version/schema
version、display/provenance 字段、source module path、source symbol、runtime-only config value、
timeout、routing、gateway、deployment revision、service protocol identity、build id、file refs、
private receiver concrete type、private receiver conformance、`implementationLinks`、operation target、
DB/spawn/actor metadata。

## 3. Package 和 Service
Package 是 Publication 的本地链接形态：

- 由 `package.yml` 声明。
- 发布与 service 相同的 `PublicationAbiUnit`。
- 在 `PackageUnit.implementationLinks` 中保存 local executable / const / type target。
- 通过 manifest dependency alias 被 import。
- 不拥有 router identity。
- 不作为远程 operation 调度。

Service 是 Publication 的远程运行形态：

- 由 `service.yml` 声明。
- 在 Publication core 之外叠加 service runtime spec。
- 发布与 package 相同的 `PublicationAbiUnit`。
- 在 `ServiceUnit.operations` 中保存 `operation_abi_id` 到 service-local target 的 runtime table。
- 在 `ServiceUnit.operationRouteBindings` 中保存 gateway / service ingress selector 到
  `operation_abi_id` 的预编译映射。
- 拥有 service version、protocol identity、ingress metadata、runtime activation 和 release pointer。
- 可以被 router / runtime 作为远程请求目标。

复用业务逻辑时，应抽成 package，再由 service 引用。service 不作为代码复用单元。

## 4. Manifest 语义边界

Publication manifest 的稳定事实：

- `id` 是 stable publication id。
- `version` 是 publication version。
- public symbol、rename 和 namespace projection 不由 `package.yml` 或 `service.yml` 声明。
- `packages` 声明源码级 package dependency：id、精确 version、alias 和解析元数据。
- package manifest 不再声明 capability binding requirements；抽象能力依赖写成 package public entry
  的 `any I` 参数/返回，由 consumer 在调用点用 `as I` 装箱后传入。

Publication public API metadata 产生 public API graph。该 graph 绑定 public path 和当前
production source set 中的 source declaration；public path 是外部源码可写名字，source declaration
是 compiler 内部链接和 projection 的目标。

source import 只看到 dependency alias。复杂 package id、registry authority、source revision、
package build identity、assembly identity 和 source hash 都不写入源码 import。

service-only runtime spec 包括 HTTP / WebSocket ingress metadata、remote operation projection、
cross-service dependency lock、timeout table、component / construction metadata，以及 service
revision、protocol identity 和 routing 所需 metadata。这些字段不属于 Publication core，也不属于
package dependency entry。

旧的 service manifest `packages[].bindings` 输入已退役。service 若要给 package 提供能力，应在调用
package public entry 时传入 `any I` 参数；该值可以来自本地装箱，也可以来自 service dependency public
instance 装箱，例如 `remoteLlm/managedLlm as api.LlmClient`。

HTTP ingress metadata 区分 raw route 和 compiler-generated typed route。Typed HTTP route 的
effective method 固定为 `POST`，manifest 必须记录 literal path、wrapper target、inferred body /
response schema refs 和 typed HTTP ingress identity。该 identity 由 service id、effective method、
literal path、body schema closure 和 response schema closure 共同决定。

## 5. Public API Graph

Publication API graph 在 package 和 service 中表示同一件事：当前 publication 的 public source
surface。compiler 应先构建统一 graph，再按 package 或 service 选择 projection。API graph 覆盖
显式 public symbols、types / aliases / interfaces、constants、callable functions、public instances 和
boundary schema closure 所需类型。

Public root 引用到的 named types 会自动进入 ABI / schema closure，但不会自动成为外部源码可写的
public name。外部源码可写名字只来自 public API graph 中显式声明的 public path。

Public instance 是 public API graph 中可作为 binding target 和 dependency receiver root 的 explicit
instance export。instance 自身不等于 operation；它显式 exposed 的 interface methods 才 projection 成
public instance method operations。

第一版中，public instance 只能来自 `api.yml` 显式公开的 top-level `const + interfaces` leaf。该 const
必须有显式 nominal receiver type；receiver type 必须显式 implements 一个或多个 interface；`interfaces`
列表必须显式写出要暴露的 fully substituted `InterfaceInstantiationRef`。receiver type implements 但未列入
`interfaces` 的 interface 不进入 public instance export、operation export、source-call method index 或
ABI hash。未显式进入 public API graph 的 const、type、interface、alias、function 和 impl method 都不是
public instance。普通 public const 即使类型 implements interface，也不会自动成为 receiver root。

`public_instance_key` 是 `api.yml` 左侧完整 API graph public path。嵌套 public instance `llm.managed`
的 key 是完整 path `llm.managed`，不是 leaf `managed` 或 display name。dependency lookup、binding
target、public instance operation id 和 ABI identity 都使用完整 `public_instance_key`。

public instance projection 必须为每个 exposed interface method 构建 source-call method index。源码
method name 在同一个 public instance 内必须唯一；跨 interface 重名即使签名相同也是 projection compile
error。publication-level `sourceCallOperationIndex` 同时覆盖 public function path 和
`<public_instance_key>.<method>`，任何路径冲突都必须报错。

普通 source file 中不再使用 `export function` / `export type` / `export impl` / `export const`。
未进入 public API graph 的 symbol 只在当前 publication 内部可见。它不进入 package ABI，也不进入
service remote contract，除非作为 ABI / schema closure 的内部节点被 explicit public root 引用。

Publication API graph 是 source projection，不是运行时路由表。HTTP path、WebSocket route key 和
service-to-service target id 都是 service projection 之后的 metadata。

## 6. Local vs Remote Linkage

Package 和 Service 的差异不在 public API metadata，而在 linkage policy。

Package public callable 使用 local linkage：

- import alias 绑定到 package dependency。
- operation call 先通过 dependency `PublicationAbiUnit.sourceCallOperationIndex` 从完整 source-call path
  解析到 `OperationAbiRef`；非 operation public symbol 才通过 public path 解析到 public symbol。
  两类结果再由 dependency `PackageUnit.implementationLinks` 链接到 type、const、function executable 或
  local const receiver executable。
- package call 不经过 router。
- package ABI identity 只用于兼容校验，不作为 deployment selector。

Service public callable 使用 remote linkage：

- public callable 和 public instance method 被 projection 成以 `operation_abi_id` 为 key 的 service
  operation。
- call 遵守 protocol identity、timeout、cancel、trace、error envelope 和 revision routing。
- runtime 即使优化为进程内执行，语义仍是 remote operation call。
- service operation target 必须链接到 Service Unit / File IR Unit 中的 executable address。
- gateway、HTTP、WebSocket 和 service-call ingress 必须先通过 `OperationRouteBinding` 映射成
  `operation_abi_id`；provider 执行阶段只用校验后的 `operation_abi_id` 查 `ServiceUnit.operations`，
  不按 public path、display name、source method name 或 interface id + method name 查找。

因此 compiler 不应维护两套“package exports”和“service contract API”解析逻辑；分叉点应是
linkage policy、`implementationLinks` 和 service runtime projection。public operation 的 canonical
signature、schema closure 和 effect/throw/config metadata 的唯一查询表是
`PublicationAbiUnit.operationAbi[operation_abi_id]`。同一 source callable 可以被多个 public path 暴露；
每个 public path 生成独立 `operation_abi_id` 和 `OperationAbiRef`，但可以链接到同一个 executable target。

### Package abstract capabilities

Package 通过 `any I` 参数/返回表达“调用者提供一个实现某个 interface 的实例”。interface /
conformance 的通用语义见 `interface.md`；`any I` 值布局与本地/远程装箱见 `any-interface.md`。

示例：

```skiff
// package source
function run(input: AgentInput, llm: any llm.ManagedLlmService) -> Stream<LlmStreamEvent> {
  return llm.streamChat(toLlmRequest(input))
}
```

consumer 在调用点装箱并传入能力：

```skiff
agent/run(input, localManagedLlm as llm.ManagedLlmService)
agent/run(input, remoteLlm/managedLlm as llm.ManagedLlmService)
```

旧的 `requires.bindings` 和 service `packages[].bindings` manifest 写法已删除，compiler 对旧字段
fail closed。package source 不再拥有 binding alias receiver root；能力就是普通 `any I` 形参，可以传递、
返回或放入允许的同进程 package entry 类型图中。service operation、public instance operation、wire
schema 和 persistent schema 仍拒绝 `any I`。

public instance 自身不是 service operation；它是 public callable projection 的 receiver root。
instance interface methods projection 成 operations，并以 `OperationAbiRef.operation_abi_id` 作为唯一
linkage/dispatch identity。dependency source call 例如 `remoteLlm/managedLlm.sendChat(...)` 使用
callee `PublicationAbiUnit.sourceCallOperationIndex` 解析完整 call path，得到唯一 `OperationAbiRef`；runtime
不根据 `managedLlmService.sendChat` 这样的名字查找 target。public instance 的 exposed
`InterfaceInstantiationRef`、source-call method index 和 method operations 是 publication ABI contract；
private receiver concrete type 和 private receiver conformance 只用于 runtime projection validation，不进入
`PublicationAbiUnit` 或 ABI hash。若一个 public instance 暴露的多个 interface 中存在同名 method，producer
projection 必须报错。

第一版不定义 runtime 传递远程 instance handle、函数值 callback、service locator、按字符串选择
provider、或跨请求持有远程 capability handle。远程 `any I` 的 dependency lock 归属于产生远程装箱的
service assembly。

## 7. Service Protocol Identity

Service protocol identity 由 service Publication API remote projection 的 canonical schema 生成。
它描述 service-to-service remote operation contract，不描述具体 build，也不替代 service version。

canonical schema 的 roots 包括：

- service public API paths。
- remote projection 选中的 `OperationAbiRef`、`operation_abi_id` 和 canonical public signatures。
- public instance / binding target receiver roots 的完整 `public_instance_key`、显式 exposed
  `InterfaceInstantiationRef`、source-call method index 和 method operations。
- operation 参数和返回类型递归引用到的 schema closure named types。
- lang / runtime prelude / std / package schema 类型的 canonical owner identity。
- cross-service dependency lock 引入的 callee protocol identity。

会改变 protocol identity 的典型变化：

- remote operation `operation_abi_id`、参数、返回、stream/effect/throw/config metadata 或 public path 变化。
- public instance / binding target 的 `public_instance_key`、exposed `InterfaceInstantiationRef`、
  source-call method index 或 method operation table 变化。
- boundary schema closure 中的 public/schema type 变化。
- package schema 类型 owner identity 变化。
- dependency lock 中的 callee protocol identity 变化。

private receiver concrete type、receiver const source module path、private receiver conformance、
implementation target、gateway path、route alias 和 display name 不属于 protocol identity 输入，除非它们
改变了 public operation ABI、schema closure 或 route entry identity。

不应改变 protocol identity 的变化：

- 注释、空白、源码文件顺序。
- 未进入 remote boundary 的 private helper。
- 只影响 implementation 的 code revision。

当前不定义自动兼容、字段投影、adapter 或 protocol fallback。schema closure 失败时不得生成
protocol identity。

## 8. Ingress Entry Identity

Ingress entry identity 只属于 schemaful ingress entry 的外部协议 schema。它不能替代 service
protocol identity，也不能作为 service release selector。

当前稳定边界：

- raw HTTP dispatch 没有 per-route entry identity。
- typed HTTP route 拥有 typed HTTP ingress identity，用于 client generation、drain 和发布验证；它不替代 service protocol identity。
- WebSocket entry 可以有独立 entry identity。
- entry identity 记录其 selector、绑定的 `operation_abi_id` 和 service protocol identity。

WebSocket entry identity 输入包括 connect request / result schema、Connection context schema、
route key 列表、route event schema 和 receive event schema。

更改 entry 暴露的 connect、route、receive schema 会改变 entry identity。更改 service API 中未被该
entry 暴露的 public/schema type，不改变该 entry identity。

Connection 是 WebSocket entry identity 和 drain 的 runtime owner。旧 socket 继续按旧 entry schema
解码，runtime 不能用当前源码临时猜测旧连接格式。

## 9. Dependency Lock

Skiff 有两类依赖锁。

Package dependency 是 local linkage 输入：

- manifest 声明 package id、精确 version 和 alias。
- production activation 通过当前 package version pointer 解析 Package Unit。
- Service Unit 保存 package ABI expectations，用于 activation fail closed。
- Service compile 必须满足 Package Unit 发布的 ABI expectations；package capability injection 使用显式
  `any I` 参数/返回，而不是 Package Unit 发布 binding requirements。
- service runtime dependency 不固定 package source revision、package build identity 或 assembly identity。

Service dependency 是 remote linkage 输入：

- consumer dependency lock 绑定 callee service id、API surface、`OperationAbiRef` / `operation_abi_id`
  expectations、public instance exposed `InterfaceInstantiationRef`、callee protocol identity 和 route
  selectors。
- runtime 只路由到支持 exact callee protocol identity 的 service revision。
- callee protocol 变化后，consumer 必须重新编译发布，才会绑定新 identity。
- 没有匹配 revision 时，调用方得到 provider unavailable 类错误。

dependency lock 是发布产物和审计事实，不是源码 import 语法。

## 10. Typed Artifact Units

当前生产路径的可执行产物只有三类 typed unit：

- File IR Unit：单个 `.skiff` source file 的 compiled executable form，以 `fileIrIdentity` 作为
  runtime cache key，保存 declarations、link targets、type table、executables 和 structured
  external refs。
- Package Unit：package 的发布和 local link 边界，保存 package metadata、file refs、
  `publicationAbi`、dependencies、package ABI expectations、public `any I` entry boundary metadata、
  `implementationLinks` 和 package-local config/effect runtime metadata。public ABI export index 属于
  `publicationAbi`；file/executable/const/type target 属于 `implementationLinks`。
- Service Unit：service runtime production load 的主入口，保存 service metadata、service version、
  protocol identity、service-owned file refs、package/service dependencies、package ABI expectations、
  `publicationAbi`、`ServiceUnit.operations`、`operationRouteBindings`、dependency lock / remote box provenance、gateway、config、
  db 和 actor metadata。service operation table 以 `operation_abi_id` 为 key；gateway/service ingress 通过
  `OperationRouteBinding` 预映射到该 key。

File IR Unit 不保存 service version、runtime build id、resolved package build 或 package public API
surface。Package Unit 不内联 dependency package 的 File IR payload。Service Unit 不保存固定 package
build 作为运行时依赖锁，也不保存外部 activation `buildId` 作为源码编译结果本身。

## 11. Bundle、Index 和 Assembly

`serviceAssembly`、`bundle`、`index` 和 protocol schema 是 artifact locator / publishing projection。

它们可以保存 artifact path、content identity、source map projection、gateway / config metadata、
service protocol schema 和发布期诊断信息。

它们不能成为 runtime executable source of truth。runtime activation 的可执行输入必须来自 Service
Unit、Package Unit 和 File IR Unit。

Bundle 是一次发布产物的聚合，不是源码层概念。Index 是从 selector 定位 artifact 的运行时入口，
不是语言 API。

## 12. RuntimeProgram 和 Dynamic Linking

Runtime activation link 成 `RuntimeProgram`。它是某个 service version 在当前 package pointer
状态下的具体执行视图。

`RuntimeProgram` 包含 service metadata、service version、dynamic activation build identity、
service File IR refs、resolved Package Units、package File IR refs、`operation_abi_id` dispatch table、
selector-to-operation route mapping、link overlay、gateway config 和 runtime type context。

`RuntimeProgram` 不复制或压平 executable bodies。多个 service version 或 activation 可以共享相同
File IR Unit、Package Unit 和 package File IR Unit。

activation 的稳定语义：

- 通过 service id + service version 找到 Service Unit。
- 读取 Service Unit 的 package dependencies。
- 对每个 package id@version 解析当前 Package Unit。
- 校验 Service Unit 记录的 package ABI expectations。
- 构建 service/package symbol 的 link overlay。
- 校验 `publicationAbi.operationAbi`、operation exports 与 `implementationLinks` / `ServiceUnit.operations`
  一一对应。
- 从 `OperationRouteBinding` 构建 selector 到 `operation_abi_id` 的 load-time route table。
- 计算 dynamic activation build identity。
- 注册可 dispatch 的 service operation 和 ingress target；执行阶段只按 `operation_abi_id` 查 target。

缺少 package、ABI 不兼容、target 不存在、typed link 失败或 artifact schema 不匹配时，activation
必须 fail closed。

## 13. Identity 总结

- Publication id/version：源码发布对象的稳定名字和版本。
- Package version：人工维护的兼容版本线，可随同版本 bugfix 移动 pointer。
- Package source revision：registry 中不可变 package source snapshot。
- Package build identity：可信 compiler 输出 Package Unit 后的不可变 build 身份。
- Package ABI identity：package public ABI surface 的兼容校验身份。
- Service version：service 外部兼容线和长期请求语义的正式版本键。
- Service build record identity：service 源码编译产生的不可变 build record。
- Runtime activation build identity：Service Unit 加当前 resolved Package Units link 后的动态执行身份。
- Service protocol identity：remote operation canonical schema 身份。
- Ingress entry identity：schemaful ingress entry 的外部协议 schema 身份。
- Assembly / unit identity：artifact 内容校验身份。

这些 identity 可以相互引用用于审计，但不能互相替代。
