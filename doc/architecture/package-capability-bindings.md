# Package Capability Bindings

> **已被取代（2026-06-24）**：capability binding 机制已合并进 `any I`。binding 作为"发布期静态、单点
> 绑定（一个 requirement 恰好绑一个实现）、不可流动的受控 root"的形态退役；package 抽象依赖能力改为
> "入口吃 `any I` 参数 + consumer 调用点用 `as I`（本地/远程）装箱传入"。binding 远端形态的 dependency
> lock / protocol identity 机制被远程 `as I` 装箱点复用。权威设计见 `any-interface-value.md`（§Boxing /
> §Remote Fail-Closed / §Capability As Parameter）。本文保留作为问题背景与远端 lock 形态的历史依据，
> **不再是当前机制**。

本文定义 package capability binding 的长期架构边界。用户可见语义以
`../reference/publication.md` 为准；本文说明为什么该机制不复用 service dependency manifest，以及
compiler / runtime 应把哪些事实放在哪一层。

## Problem

有些 package 是可复用业务逻辑，但需要调用宿主 service 提供的能力。例如 server-side agent package
需要“按模型发起 managed LLM 请求并消费 `llm.LlmStreamEvent` 流”。这个能力的形状可以由
`skiff.run/llm` 里的 interface 描述，但具体实现可能来自 `remoteLlm` service，也可能来自当前 service
自己的一个实例。

直接让 package 依赖 `remoteLlm` 会把复用逻辑绑定到具体 service。把 requirement 写成
`requires.services` 也不正确，因为 interface 不是 service，且同一个 service 可以暴露多个 interface
或同一 interface 的多个 public instance。

## Decision

Package 声明 capability binding requirement；使用 package 的 service 负责把 requirement 绑定到一个
qualified instance reference。

```yaml
# package.yml
requires:
  bindings:
    - alias: managedLlm
      interface: llm.ManagedLlmService
```

```yaml
# service.yml
packages:
  - id: skiff.run/server-side-agent
    version: 0.1.0
    alias: agent
    bindings:
      - interface: llm.ManagedLlmService
        instance: remoteLlm.managedLlmService
```

consumer binding entry 中的 `interface` 是被依赖 package requirement ABI 发布的 selector，不在 consumer
service 的 package alias scope 中重新解析。consumer 也可以用 requirement `alias` 直接选择 binding。

Binding 是发布期静态事实，不是运行时传值。它不创建 first-class service object，也不允许把函数、
callback 或 arbitrary object 传进 package。

## Instance References

Binding instance reference 有两个 owner 形态。

当前 publication：

```text
root.<modulePath>.<instanceName>
```

远端 service dependency：

```text
<serviceAlias>.<publicInstanceName>
```

`root` 后面是当前 source set 的 source module path。service alias 后面不是 callee source module；
它是 callee 的 public instance / operation namespace。这个规则沿用现有 service dependency 调用模型：
`account.UserApi.get(...)` 解析为 dependency ref `account` 和 operation `UserApi.get`。

因此 `remoteLlm.managedLlmService` 是合法远端 instance reference；`remoteLlm.root.internal.managed.foo` 和
`remoteLlm.internal.managed.foo` 都不是 service dependency 语义。

## Interface vs Service

`llm.ManagedLlmService` 这样的名字如果是 Skiff interface，它只定义 method set 和签名，不拥有 router
identity、service id/version、protocol identity 或 runtime activation。

具体 instance 才决定 linkage：

- `root.*` instance：本 service 内部实现，compiler 可以 local link。
- `<serviceAlias>.*` instance：远端 service dependency，compiler 必须生成 service dependency operation
  引用和 dependency lock。

这允许同一个 interface 有多个实现：

```yaml
bindings:
  - alias: primaryLlm
    interface: llm.ManagedLlmService
    instance: remoteLlm.managedLlmService
  - alias: cheapLlm
    interface: llm.ManagedLlmService
    instance: remoteLlm.cheapManagedLlmService
```

这里的 `alias` 是被依赖 package 声明的 requirement alias；consumer 不能在 binding entry 中声明新的
package-local value alias。

如果 package 对某 interface 只有一个 requirement，consumer 可以省略 `alias` 并按 interface 唯一匹配。
如果存在多个同 interface selector requirement，必须用 requirement alias。按 interface 匹配时使用
package requirement 发布的 selector 字符串，而不是 consumer service 的 import alias；匹配后再使用
package requirement 保存的 canonical interface identity 做 conformance 校验。

## Public Instance Projection

Service public API graph 需要区分三类 public facts：

- schema symbols：public type / alias / interface。
- callable symbols：public functions and receiver methods。
- public instances：可作为 binding target 的 receiver roots。

Public instance 本身不是 service operation。它的 interface methods 派生成 service operations，例如：

```text
remoteLlm.managedLlmService.sendChat(req)
```

对应 callee operation：

```text
managedLlmService.sendChat
```

ordinary `const` 仍不自动成为 service operation。若 service 希望一个顶层名字成为 public
binding instance，compiler 必须能从 public API graph 中确认该名字是
可调用 receiver root，并确认它显式 implements 目标 interface。

第一版 public instance projection 规则：

- 只扫描显式 public bindings。
- 只有显式 public top-level const 可以成为 public instance。
- const 必须显式声明 nominal receiver type；不能靠 initializer 推断，也不能声明成 type alias /
  interface type。
- declared receiver type 必须显式 implements public API graph 中可见的 interface；不做 method-set
  structural matching。
- 未显式进入 public API graph 的 const、type、interface、alias、function 和 impl
  method 都不是 binding target receiver root。
- public instance name 在 service public API graph 内必须唯一；第一版
  `<serviceAlias>.<publicInstanceName>` 没有 public-path namespace。

Public instance projection 必须发布足够的 nominal metadata，供 consumer 编译期 fail closed 校验：

- public instance name。
- declared receiver type identity。
- explicit implemented interface identities。
- 由该 instance 的 interface methods 派生出的 public operations。

operation name 由 `<publicInstanceName>.<methodName>` 派生。若同一个 public instance 的多个 interface
method 派生出相同 operation name，且最终 target、canonical signature 或 mode 不一致，producer compile
必须失败。

这些 public instance facts 是 service protocol identity 的输入。callee 移除 public instance、替换
declared receiver type、或移除显式 interface conformance，即使 operation method signatures 没变，也必须
改变 protocol identity。dependency lock 可以只记录 package 实际调用到的 remote operations，但 fail-closed
判断必须基于包含完整 public instance metadata 的 callee protocol identity。

## Artifact Ownership

Package Unit 应记录 binding requirements，作为 package ABI surface 的一部分：

```rust
struct PackageBindingRequirement {
    alias: String,
    interface_selector: String,
    interface_identity: InterfaceIdentity,
    interface_methods: Vec<InterfaceMethodSignature>,
}
```

Service compile 在解析 package dependency entry 时记录 binding resolutions：

```rust
struct ResolvedPackageBinding {
    package_alias: String,
    requirement_alias: String,
    interface_selector: String,
    interface_identity: TypeRef,
    instance: BindingInstanceRef,
}

enum BindingInstanceRef {
    LocalRoot { module_path: String, instance_name: String },
    ServiceDependency { dependency_ref: String, public_instance: String },
}
```

Service Unit 应保存 enough information 让 runtime link fail closed：

- package requirement identity。
- resolved instance reference。
- interface method set used by the package.
- 对远端 instance，callee public instance conformance expectation、service dependency operation、target、mode、
  protocol identity。

远端 binding resolution 最终应进入现有 service dependency lock，而不是形成第三种 runtime dependency。

## RemoteLlm Example

`skiff.run/llm` 定义 managed capability interface：

```skiff
interface ManagedLlmService {
  function sendChat(self: Self, input: ManagedLlmChatRequest) -> Stream<LlmStreamEvent>
}
```

`remoteLlm` service 暴露 public instance：

```skiff
type ManagedLlmServiceImpl implements llm.ManagedLlmService {}

const managedLlmService: ManagedLlmServiceImpl = ManagedLlmServiceImpl {}

impl ManagedLlmServiceImpl {
  function sendChat(self: ManagedLlmServiceImpl, input: llm.ManagedLlmChatRequest) -> Stream<llm.LlmStreamEvent> {
    // 读取 remoteLlm 自己的 config、quota、provider catalog，并转发到 provider。
  }
}
```

业务 service 绑定 server-side-agent package：

```yaml
services:
  - id: skiff.run/remotellm
    version: 0.1.0
    alias: remoteLlm

packages:
  - id: skiff.run/server-side-agent
    version: 0.1.0
    alias: agent
    bindings:
      - interface: llm.ManagedLlmService
        instance: remoteLlm.managedLlmService
```

`remoteLlm` 负责 API key、额度、provider routing 和计费侧状态。调用方仍然负责决定是否统计自己的业务维度。

## Non-Goals

本机制不解决：

- 长时间后台 drain、durable workflow、resume/cursor 或 WebSocket actor 生命周期。
- first-class interface value。
- runtime service locator。
- 动态按字符串选择 service/provider。
- function/callback value 传入 package。
- service alias 后穿透 callee private source module。
