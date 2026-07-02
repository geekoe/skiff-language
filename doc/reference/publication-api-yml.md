# Skiff Publication API YAML Reference

## 本文负责 / 不负责

本文负责定义 `api.yml` 的目标语义：package / service 如何显式声明 public API
surface、public path 如何绑定到当前 publication 的 source symbol、ABI / schema closure 如何从这些
显式 public roots 派生，以及 package / service projection 如何使用同一份 API graph。

本文不负责 compiler 迁移步骤、历史 `export` surface 语法、artifact JSON 字段细节、registry 操作流程、
runtime 调度或完整 YAML parser 实现。

## 1. Source Layer

每个 package 或 service 可以在 publication source root 放置一个固定名字的 `api.yml`。

`api.yml` 是 source-layer metadata，不是 `.skiff` source module：

- 它不参与 `root.*` module namespace。
- 它不能被 `import` 或从源码引用。
- 它不生成 File IR unit。
- 它必须参与 publication source identity / build identity。

没有 `api.yml`、`api.yml` 为空、或顶层 mapping 为空，都表示该 publication 没有 public API。

`package.yml` 和 `service.yml` 不列 public symbol，也不声明 rename / namespace projection。
public API 的唯一符号事实来源是 `api.yml`。

Skiff source declaration 不使用 `export` 关键字表达 public visibility。普通 source file 没有 public
visibility marker；source file 不是包内 privacy 边界。

## 2. YAML Shape

`api.yml` 顶层必须是 mapping。mapping key 是 public path segment。普通 public symbol leaf 是 source
selector：

```yaml
decode: decode.decode
LlmRequest: types.LlmRequest
```

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

同一个 publication API graph 内 public path 不得重复。不同 symbol kind 也不能共享同一个 public
path。

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
公开 `std.*` 或外部 package alias 下的 symbol。需要公开外部能力时，应在当前 publication 中定义明确的
wrapper、type、interface、function 或 const，再由 `api.yml` 公开该当前 source set 的 symbol。

第一版不限制 source selector 只能指向顶层文件或顶层目录下的 source module。是否公开某个内部目录中的
symbol 由 `api.yml` 显式声明决定，而不是由文件位置隐式决定。项目可以对 `internal.*` 等路径提供 lint
或诊断建议，但它不是语言级错误。

## 5. API Graph

compiler 从 `api.yml` 构建统一 Publication API graph。每个 leaf 生成一条 public binding：

- public path。
- source module path。
- source symbol name。
- source symbol kind。

public identity 使用 public path。source selector 只用于链接、类型检查、ABI closure 和诊断。

API graph 覆盖：

- 显式 public symbols。
- public types / aliases / interfaces。
- public constants。
- public callable functions。
- public instance roots。
- ABI / schema closure 中需要的 closure-only types。

Public instance 是 API graph 中可作为 binding target 的 receiver root。第一版中 public instance
只能来自 `api.yml` 显式公开的 top-level `const`；该 const 必须有显式 nominal receiver type，且该
receiver type 必须显式 implements 一个或多个 interface。public instance leaf 还必须显式列出 exposed
interfaces；普通 public const 不会自动成为 public instance。instance 自身不等于 operation；它公开的
interface methods 才能 projection 成 public instance method operations。

`public_instance_key` 是完整 API graph public path。dependency lookup、binding target、
`OperationAbiRef.public_instance_key` 和 public instance method `operation_abi_id` 都使用该完整 path，而
不是 leaf/display name。

## 6. ABI / Schema Closure

显式 public roots 的 callable signature、type body、alias target、interface method signature、
const type 和 public instance metadata 会递归引用其他 named types。compiler 必须自动收集这些引用作为
ABI / schema closure。

closure 中的 named type 分两类：

- explicit public type：在 `api.yml` 中有 public path，外部源码可以写这个 public name。
- closure-only type：只因为 explicit public root 的边界形状需要而进入 ABI / schema closure，外部源码
  不能写这个 public name。

closure-only type 参与 ABI identity、schema encoding、compatibility checking 和 artifact linking，
但不会自动扩大 public namespace。它没有 public path，却必须有 canonical ABI type id；compiler 依赖
该 id 判断推断出的值能否传给另一个 API，而不是依赖外部源码是否能书写该类型名。

compiler 可以对 callable 参数、返回类型、public type 字段或 public instance metadata 中出现的
closure-only named type 给出诊断建议；诊断不得把该 type 自动提升为外部源码可写 public name。

## 7. Package Projection

Package projection 使用同一份 Publication API graph：

- dependency alias 绑定到 package publication。
- caller 只能通过 dependency alias 加 public path 访问显式 public symbols。
- package dependency call 使用 dependency `PublicationAbiUnit.sourceCallOperationIndex` 将完整 source-call
  path 解析到唯一 `OperationAbiRef`。
- package local linkage 再通过 dependency `PackageUnit.implementationLinks` 解析到 source symbol /
  executable / const receiver target。
- package ABI expectation 记录 `PublicationAbiUnit.abiIdentity`、public path、`operation_abi_id`、
  canonical signature / type descriptor 和 closure。
- public function 和 public instance method 都写入 `PublicationAbiUnit.operationAbi`；package operation
  target 只写入 `implementationLinks.operationTargets`，key 是 `operation_abi_id`。

未出现在 `api.yml` 的 source symbol 不能被 package caller 书写；它只可能作为 closure-only ABI 节点被
链接。

## 8. Service Projection

Service projection 使用同一份 Publication API graph：

- public callable function 可以 projection 成 public function operation。
- public instance root 按其 explicitly listed interface methods projection 成 public instance method
  operations。
- package/service 共享 `PublicationAbiUnit` contract：public operation 的 canonical signature、schema
  closure 和 stream/effect/throw/config metadata 都写入 `operationAbi[operation_abi_id]`。
- service runtime projection 额外生成 `ServiceUnit.operations` 和 `OperationRouteBinding`。HTTP/WebSocket
  ingress、service-call selector 和 gateway route 必须先映射成 `operation_abi_id`；provider 执行阶段只按
  `operation_abi_id` 查 target，不按 public path、method name 或 display name 查找。
- operation / protocol identity 使用 public path、`operation_abi_id`、canonical signature、
  `public_instance_key`、exposed `InterfaceInstantiationRef` 和 schema closure。

source module path 不作为 service protocol identity。HTTP path、WebSocket route key、timeout、routing
revision 和 runtime activation 属于 service projection 或部署 metadata，不属于 `api.yml`。

## 9. Validation Summary

必须报错的情况包括：

- `api.yml` 不是 mapping。
- public key 不是合法 identifier segment。
- leaf 不是合法 source selector string。
- source selector 少于两个 segment。
- source selector 无法解析到当前 production source set 的 top-level symbol。
- source selector 指向 test source、dependency symbol、`std.*` symbol 或 impl method。
- public path 重复。
- public path 与保留 public namespace 规则冲突。
- public instance leaf 缺少 `const` 或非空 `interfaces`。
- public instance `const` 不是当前 source set 的 top-level const。
- public instance interface selector 不是 public/imported public interface，或 generic type args 未 fully
  substituted。
- receiver concrete type 未显式 implements listed interface。
- public instance exposed interfaces 中出现重复 canonical `InterfaceInstantiationRef`。
- 同一个 public instance 暴露的多个 interface 中出现相同 source method name。
- public function path 与 `<public_instance_key>.<method>` 在 `sourceCallOperationIndex` 中冲突。

不应作为语言级错误的情况：

- source selector 指向深层目录下的 module。
- source selector 指向名字包含 `internal` 的 module。
- closure-only type 未显式列入 `api.yml`。
