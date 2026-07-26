# Compiler Package Pipeline Contracts

本文定义 compiler 的长期内部阶段边界。它面向 compiler、artifact 与 deployment 维护者，不是用户可见
语言规范，也不是迁移计划。本文里的类型名是职责草图，不冻结 Rust public API 或最终字段拼写。

本文服从 [`package-service-contract-deployment.md`](package-service-contract-deployment.md) 的四对象模型：
Package 是唯一源码编译单元；ServiceContract、ServiceDeployment 与 RuntimeAssembly 是独立 projection，
不存在 Package/Service 的共同 `Publication` 输入、产物或流水线。

## Scope

本文负责：

- Package source compile 的阶段边界与事实 owner。
- `package.yml`、`api.yml`、可选 `service.yml` 和 `config.*.yml` 的读取边界。
- Package dependency 与 service dependency 的 typed inputs。
- config requirement 的合并与诊断 provenance。
- ServiceContract、external ingress 和 ServiceDeployment projection 的输入输出。
- typed model 到 artifact emission 的单向数据流。

本文不负责完整 YAML schema、registry/release 操作、runtime linking、迁移 checklist 或具体 Rust 模块布局。

## Pipeline

目标态有一条 Package 编译线和三条消费其 typed facts 的 projection：

```text
PackageCompileInput
  -> PackageSourceModel
  -> LoweredPackage
  -> CompiledPackage
  -> PackageArtifactProjection
  -> PackageArtifact + FileIrUnit[]

CompiledPackage + PackageArtifact + service id
  -> ServiceContractProjection
  -> ServiceContract

CompiledPackage + PackageArtifact + typed service.yml ingress
  -> GatewayEntryProjection
  -> typed gateway entries

PackageArtifact + ServiceContract + typed gateway entries + selected config profile
  -> ServiceDeploymentProjection
  -> ServiceDeployment
```

每个阶段只消费前一阶段显式提供的 typed facts。下游若需要源码、AST、配置原文或 path/string
协议中的语义，必须把所需事实提升到上游 typed output；不得在 projection 或 emission 中重新解析。

## Compiler Support Owners

跨阶段 pure support 可以放在 compiler core，但 core 不拥有 Package API、artifact identity 或 IO。

共享 artifact DTO 与 canonical identity 分开：

- artifact model 拥有 PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly 及其共享叶子
  DTO；
- artifact identity owner 唯一负责 canonical bytes、hash、prefix、normalization 和 identity validation；
- compiler projection input 只承担 typed DTO handoff，不解析 source、不推导 type/ABI、不执行 IO；
- source、lowering、projection 和 emission 不复制 identity builder。

旧的 `PublicationAbiUnit`、`PackageUnit`、`ServiceUnit`、`CompiledPublication`、
`LoweredPublication` 或共同 projection bundle 不是目标态 owner。迁移期代码只能作为待删除 legacy
consumer，不能被新入口或新 artifact 继续依赖。

## Package Compile Input

Package root 的语言输入包括：

```text
PackageCompileInput
  root
  .skiff production sources
  package.yml
  api.yml metadata
  declared static resources
  resolved PackageArtifact dependencies
  resolved code-free ServiceContract dependencies
  optional service.yml authoring metadata
```

职责固定为：

- `package.yml`：Package id/version、package dependencies、service dependencies，以及
  config/state/resource/runtime capability requirements 的声明入口；
- `api.yml`：Package public source graph，以及 service package 可投影的 service-to-service API roots；
- `service.yml`：service id、HTTP/WebSocket external ingress、handler/pre/guard selector、adapter source
  与外部协议 metadata；
- `config.*.yml`：部署时为已声明 requirement 提供或选择值与 owner，不进入 Package compile。

Service 首先是 Package。存在 `service.yml` 不会把 root 切换成另一种 source input，也不会让
`package.yml` 与 `service.yml` 互斥。

Dependency source text 不属于当前 Package source set。Package compile 只消费精确、已验证的 typed artifact：

```text
ResolvedPackageDependency
  alias
  exact package coordinate
  expected PackageLocalAbiIdentity
  PackageArtifact

ResolvedServiceDependency
  alias
  exact service coordinate
  expected ServiceProtocolIdentity
  ServiceContract
```

两个 alias 集合共享 namespace。Compiler 不得从调用拼写、当前 deployment、display name 或源码目录猜
dependency kind。

## PackageSourceModel

`PackageSourceModel` 是 source-level 事实 owner。它至少拥有：

- parsed production source set 与 all-symbol `root.*` index；
- name/type resolution、alias expansion与generic binding；
- expression type、constructor、field、operator、control-flow narrowing facts；
- Package API graph；
- exact executable signature（包含推断的concrete suspension summary）与不含该summary的interface
  requirement/conformance facts；
- package/service dependency resolution；
- callable effect、provenance、escape、write、alias 与 same-heap identity facts；
- config/state/resource/runtime requirement 使用事实；
- `service.yml` source selector 的 typed resolution intent。

Source 阶段可以 parse/resolve/type-check，但：

- 不生成 File IR；
- 不读取 config profile 中的实际值；
- 不读取最终 artifact JSON；
- 不把 external ingress 自动加入 Package API graph；
- 不把 service-only route/policy 混入 Package Local ABI；
- 不从 dependency source text 补事实。

`config.require<T>`、`config.optional<T>` 与 `config.has` 是 Package source feature。Compiler 只收集
path、type、requiredness、presence 与 provenance，不读取 deployment value。

## Type And Expression Facts

Source model 必须保存 lowering 和 projection 所需的精确 typed facts，包括：

- source-local、Package dependency、ServiceContract、prelude/std 与 DB symbol 的类型解析；
- constructor 的 duplicate/missing/unknown field 与 assignability；
- binding、return、call argument/result、field access、operator 和 pattern facts；
- generic callable instantiation；
- receiver call 的 method owner、executable identity 与 generic bindings；
- throw payload 的静态名义类型；
- service call 的 exact requirement slot、ContractOperationId 与 value-plan references。

Source spelling、public path、PackageSchemaTypeId、PackageCallableId 和 runtime address 是不同 identity
domain，不得互换。Lowering 不重新推断类型；projection 不从 AST、display string 或 File IR
execution representation恢复 source facts。

## Config Requirements

有效 config requirement 是当前 Package 与全部 Package dependency requirements 的合并：

```text
effective requirements
  = own requirements
  + direct Package requirements
  + transitive Package requirements
```

Service dependency 的 provider config 不进入 caller requirement；它属于 provider deployment。

合并必须保留声明 Package、source location 与 dependency chain：

- 同 path、同 typed access：合并并保留所有 provenance；
- 同 path、同 type 的 required + optional：effective 为 required；
- 同 path、不同 type：在 deployment/activation 前 fail closed；
- `has` 不让 path 变成 required，但仍进入使用与诊断事实。

诊断必须能指出 path、type、requiredness、声明 Package 与 dependency chain。具体显示格式不是架构契约。

## LoweredPackage

`LoweredPackage` 只消费 `PackageSourceModel`，产出 File IR 与 typed lowering metadata：

```text
LoweredPackage
  FileIrUnit[]
  executable/source mapping
  package call targets
  unresolved ServiceCallRefs
  storage lowering facts
  synthetic runtime adapter facts
```

Lowering 不直接读取 manifest、config value 或 artifact JSON，不重建 name/type/interface conformance。
普通 receiver call 必须按 source facts降低为精确 target；`throw` 必须保留静态/linked nominal identity。

External ingress 可以使用 lowering-owned typed synthetic adapter facts，但不能生成 Skiff source text。
Wrapper 是 compiler/runtime adapter 结构，不是隐藏用户源码。

## PackageArtifact Projection

Package projection 消费 `CompiledPackage = PackageSourceModel + LoweredPackage`，生成：

- FileIrUnit refs；
- PackageLocalAbi、PackageCallableId 与 implementation links；
- PackageSchema type records/index；
- Package/service requirements；
- callable semantic/effect facts；
- `BoundaryCallableProjection`；
- config/state/resource/runtime requirements；
- unresolved ServiceCallRefs。

Package projection不得读取 `service.yml` route 或 config profile，也不得生成ServiceContract operation id、
GatewayEntryIdentity 或 deployment binding。它可以保存后续 projection 所需的精确 implementation callable
signature/link facts，包括非 public top-level handler。

## ServiceContract Projection

存在`service.yml`时，tooling使用service id与Package API中显式`serviceCall: true`且
boundary-available的roots生成ServiceContract：

- operation authoring roots只来自`api.yml`；
- 只有显式`serviceCall: true`的public function或public-instance methods进入；
- marker对应的boundary projection必须Available，否则以结构化原因失败；未标记callable只是Package API；
- 每个 operation引用 canonical boundary descriptor 与 PackageSchemaTypeId closure；
- service call的caller-side suspension由call target种类决定；operation descriptor不复制provider
  callable的`maySuspend`或由它派生的取消类别；
- 不读取 HTTP/WebSocket ingress；
- 不绑定 implementation build、PackageCallableId、config 或 runtime route；
- 不发布 operation-specific throw set。

第一版service dependency graph必须是DAG；compiler在Package body编译前拒绝环，不得在普通单Package入口里
隐式启动全局源码批编译。

## Gateway Entry Projection

External ingress 是单独的 typed projection：

```text
service.yml entry
  -> strict typed authoring DTO
  -> source selector resolution
  -> exact PackageCallableId/signature/link facts
  -> adapter arg/source validation
  -> external codec/schema plan
  -> gateway entry + identity
```

约束：

- handler/pre/guard不要求出现在`api.yml`；
- 不得先制造 public path、ServiceContract operation 或 ContractOperationId；
- runtime业务 codec来自linked callable signature与compiler生成的typed plan；
- 外部JSON schema由compiler从linked callable signature与adapter source确定性生成，是entry-local
  projection，不是PackageSchema或手写runtime codec事实源；
- generic handler在完整 callable generic facts进入共享模型前必须fail closed；
- 旧`operation`字段、public-path fallback与dual-read都非法。

`GatewayEntryIdentity`只覆盖external protocol surface；handler/pre/guard callable、PackageArtifact、
完整execution plan与policy只进入ServiceDeployment revision。Canonical preimage只由共享
artifact-identity owner计算；各consumer不得自行重算另一套identity。

## ServiceDeployment Projection

Deployment projection消费 typed artifacts 与所选 config profile，不拥有 AST、source text、
type/effect inference 或 lowering helper。它负责：

- 验证 ServiceContract operations 与 Package callable bindings一一对应；
- 验证 gateway entries、external selectors 与 exact implementation facts；
- 闭合 package/service dependency bindings；
- 绑定 config/secrets/state/resource/activation policy；
- 生成 immutable ServiceDeployment 与 revision identity。

Profile 只能绑定已声明 requirements，不能增加/删除 Package dependency 或 service dependency。

## Emission Boundary

最终 JSON、artifact path 与 content hash只在显式 emission边界产生。内部阶段结构不得同时保存 typed model
及其预先渲染的`serde_json::Value`副本，也不得通过“序列化整个 DTO 后删除字段”计算 identity。

允许：

- emission按需把typed artifact渲染成JSON；
- identity owner从专用 typed identity projection计算canonical bytes；
- artifact writer写入content-addressed blobs与immutable records。

禁止：

- projection从raw JSON提取语义；
- 内部stage用JSON作为协议；
- emission重新做source/type/effect分析；
- 多个stage各自实现相同hash/preimage。

## Verification Contract

结构与测试必须证明：

- compiler没有Package/Service共同`Publication`输入或产物；
- service root仍通过唯一Package compile入口；
- `service.yml` external ingress不会进入Package API graph或ServiceContract；
- config profile值只在deployment/activation边界读取；
- SourceModel之后不重建name/type/conformance；
- lowering不读取manifest或重新推断expression type；
- projection不读取AST、调用lowering helper或解析raw artifact JSON；
- identity只有一个canonical owner；
- 没有generated Skiff source wrapper；
- 没有legacy Unit/RuntimeProgram/publication ABI被新四对象链重新依赖；
- 每个strict reader拒绝旧字段、unknown nested field、缺失identity与不匹配owner。
