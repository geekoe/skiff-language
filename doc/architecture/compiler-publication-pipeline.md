# Compiler Publication Pipeline Contracts

本文定义 `compiler` 的长期内部 pipeline contract。它面向维护者，
不是用户可见语言语义，也不是一次迁移计划。临时实现步骤仍放在
`../implementation/`。

本文里的 Rust 类型是**目标态契约草图**，描述阶段边界和事实归属，不是 public
API，字段名也不保证和当前 `compiler` 实现一致。某些类型（如
`ConfigRequirementSet`、`ServiceActivationInput`、`PublishedArtifacts`）目前尚未
在代码里成型；已存在的类型（如 `SourceCompileModel`、`LoweredPublication`、
`CompiledPublication`）字段可能与此处不同。当前实现状态以
`../implementation/compiler-architecture-audit-2026-06-03.md` 记录的 open items
为准；本文描述的是该 audit 指向的终点。

## Scope

本文负责：

- package/service publication compile 的阶段边界。
- 每个阶段的输入输出和禁止事项。
- `package.yml`、`service.yml`、`config.yml` 的内部归属。
- package dependency config requirements 的合并和诊断 provenance。
- HTTP/WebSocket ingress adapter 的长期内部位置。

本文不负责：

- 完整 YAML schema。
- registry、release、dev sync 和 runtime activation 操作流程。
- 当前代码迁移 checklist。
- 具体 Rust 模块拆分方案。

## Pipeline

长期 pipeline 只有一条 publication compile 线：

```text
PublicationInput
  -> SourceCompileModel ─┐
                         ├─> CompiledPublication
  -> LoweredPublication ─┘
       -> ProjectionBundle
       -> PublishedArtifacts
```

`CompiledPublication` 的核心是 `SourceCompileModel + LoweredPublication` 的组合
（代码里已接近这个形状），下游 projection 阶段消费的就是它。

每个阶段只能消费前一阶段的 typed output。任何阶段如果需要 AST、源码文本、
配置原文、artifact JSON 或 path/string 协议中的事实，必须先把该事实提升到
前一阶段的 typed output。

`config.yml` 不在 publication compile 线上。它属于 service activation 输入：

```text
ServiceActivationInput
  -> ResolvedRuntimeConfig
  -> runtime request config view
```

## Source Inputs

`package.yml` 和 `service.yml` 是平级的 root manifest。一个 source root 作为
publication 编译时只能是 package 或 service；同时存在时应报 ambiguous root。

```rust
enum PublicationInput {
    Package(PackagePublicationInput),
    Service(ServicePublicationInput),
}
```

二者共享 publication core：

```rust
struct PublicationCoreInput {
    root: SourceRoot,
    sources: Vec<SourceInput>,
    manifest: PublicationManifestInput,
    api: PublicationApiSpec,
    package_dependencies: PackageDependencyInputs,
}

struct PublicationManifestInput {
    id: PublicationId,
    version: String,
    packages: Vec<PackageDependencyDecl>,
}
```

Package 没有 service runtime spec：

```rust
struct PackagePublicationInput {
    core: PublicationCoreInput,
}
```

Service 在 publication core 之外叠加 service definition：

```rust
struct ServicePublicationInput {
    core: PublicationCoreInput,
    service: ServiceDefinitionInput,
}

struct ServiceDefinitionInput {
    access: ServiceAccessSpec,
    runtime: ServiceRuntimeSpec,
    service_dependencies: Vec<ServiceDependencyDecl>,
    ingress: ServiceIngressSpec,
    timeout: TimeoutSpec,
    components: ComponentSpec,
}
```

`ServiceDefinitionInput` 来自 `service.yml` 和 service profile overlay。它是
service publication 的编译输入。`config.yml`、`config.<profile>.yml` 和
`config.<profile>.secret.yml` 不是它的一部分。

## Package Dependency Inputs

Package dependencies 是 compile input，但 dependency source text 不是当前
publication 的 source set。当前 publication 应消费 dependency 的 typed
artifact/ABI information。

```rust
struct PackageDependencyInputs {
    direct: Vec<ResolvedPackageDependency>,
    transitive: Vec<ResolvedPackageDependency>,
}

struct ResolvedPackageDependency {
    alias: Option<String>,
    package_id: PublicationId,
    version: String,
    manifest: PackageManifestInput,
    unit: PackageUnitInput,
    config_requirements: ConfigRequirementSet,
}
```

`direct` 表示当前 manifest 声明的 dependencies；`transitive` 来自 dependency
package units 或 equivalent lock metadata。实现可以使用不同存储结构，但必须
能回答：

- 当前 source import alias 绑定到哪个 direct dependency。
- 当前 publication 的 effective config requirements 包含哪些 dependency
  requirements。
- 每条 dependency requirement 的来源 package 和 dependency chain。

## SourceCompileModel

`SourceCompileModel` 是中心 source-of-truth。它从 `PublicationInput` 得到所有
source/config/dependency 派生事实，但不产出 File IR、artifact JSON 或最终
runtime manifest。

```rust
struct SourceCompileModel {
    kind: PublicationKind,
    sources: ParsedSourceSet,
    name_resolution: NameResolutionModel,
    type_resolution: TypeResolutionModel,
    expression_types: ExpressionTypeModel,
    publication_api: PublicationApiModel,
    package_bindings: PackageBindingModel,
    service_definition: Option<ServiceDefinitionModel>,
    own_config_requirements: ConfigRequirementSet,
    dependency_config_requirements: ConfigRequirementSet,
    effective_config_requirements: ConfigRequirementSet,
    ingress: Option<ServiceIngressModel>,
}
```

阶段规则：

- 可以 parse source、build AST、resolve names、validate imports、构建 public API graph、
  resolve source-level types、type-check expressions、collect config usage 和
  derive typed service ingress intent。
- 不生成 Skiff source string。
- 不产出 File IR。
- 不读取 `config.yml` 的实际值。
- 不读取 final artifact JSON。
- 不把 service-only runtime facts 混入 package model。

`config.require<T>(path)`、`config.optional<T>(path)` 和 `config.has(path)` 是
publication-level source feature。Package 和 service 都可以声明 config
requirements；compiler 只收集 path/type/requiredness/presence requirements，
不读取 runtime config values。

### TypeResolutionModel / ExpressionTypeModel

`TypeResolutionModel` 和 `ExpressionTypeModel` 是 `SourceCompileModel` 的一部分，
不是 `LoweredPublication` 或 projection 的派生缓存。它们保存 source 级 typed
facts，让 lowering、contract/runtime/package projection 和 source rule validation
消费同一份事实。详细 type checking contract 见
`compiler-type-checking.md`；本节只记录 publication pipeline 中的归属边界。

```rust
struct TypeResolutionModel {
    declarations: TypeDeclarationIndex,
    aliases: TypeAliasExpansionIndex,
    package_types: PackageTypeResolutionIndex,
    db_objects: DbObjectTypeIndex,
    external_symbols: ExternalTypeSymbolIndex,
}

struct ExpressionTypeModel {
    expressions: ExpressionFactIndex,
    constructors: ConstructorValidationIndex,
    flows: ExpressionFlowIndex,
}

struct ExpressionTypeFact {
    key: ExpressionKey,
    ty: ResolvedTypeRef,
    kind: ExpressionKindFact,
    diagnostics: Vec<SourceDiagnostic>,
}
```

这些类型仍是目标态契约草图。实现可以选择不同的 index 结构，但必须保证事实的
owner 是 source compile 阶段。

`TypeResolutionModel` 负责：

- source-local type lookup、alias expansion、generic argument binding。
- package alias / dependency type lookup。
- prelude/std、DB object 和 external service type symbol lookup。
- 源级 type expression 到 compiler-owned `ResolvedTypeRef` 的规范化。

`ResolvedTypeRef` 必须遵守 compiler entity/identity 分层契约：source spelling、public path、ABI type id 和
runtime address 不能互相替代。closure-only ABI types 必须能作为不可命名但可推断的类型进入
`ExpressionTypeModel`。长期 identity contract 见 `compiler-entity-and-identity.md`。

`ExpressionTypeModel` 负责表达式和语句边界的源级类型事实，包括但不限于：

- struct / nominal constructor 的 shape validation：duplicate、missing、unknown
  field、field value type mismatch、generic argument binding。
- `let` binding initializer 和显式类型的 assignability。
- `return` expression 和 executable return type 的 assignability。
- call callee/argument/return facts，包括 generic call instantiation。
- field access 的 receiver type、field existence 和 field type。
- operator operand/result type。
- `if` 等 condition 的 boolean/nullability 规则。
- pattern match / nominal pattern / record pattern 的 target shape 和 binding type。
- DB、stream、suspend、config 等 source rule 需要共享的 expression type facts。

类型擦除要求 `ExpressionTypeModel` 也保存 receiver call 的 resolved method fact：
receiver expression type、method owner、method executable identity、generic bindings，以及
该 call 是 user impl method、built-in receiver method、actor method 还是未来显式
interface/vtable method。Lowering 不得因为缺少这些 facts，把普通 `user.method()` 留给
runtime object type lookup。

`LoweredPublication` 只能消费这些 facts 来生成 File IR 和 lowering metadata。它不得
为了某个 IR 节点重新推断表达式类型，也不得从 AST、display string、package ABI
helper 或 artifact projection 里重建 name/type resolution。

Projection 阶段同样只能消费 `CompiledPublication` 里的 typed facts。Projection 可以
把 `ResolvedTypeRef`、constructor facts 或 expression-derived operation facts 投影成
contract/runtime/package 输出，但不得从 display string、AST declaration、AST
expression 或 lowering helper 反向恢复 expression/type facts。

## Config Requirements

Effective config shape 是当前 publication 自身 requirements 和所有 package
dependency requirements 的合并：

```text
effective_config_requirements =
  own_config_requirements
  + direct package dependency config_requirements
  + transitive package dependency config_requirements
```

合并后必须保留 provenance。否则 activation 失败时用户无法知道哪个 package
或哪条 dependency chain 要求了缺失配置。

```rust
struct ConfigRequirementSet {
    requirements: Vec<ConfigRequirement>,
}

struct ConfigRequirement {
    path: String,
    access: ConfigAccess,
    ty: Option<ConfigType>,
    declared_by: PublicationRef,
    source_path: Option<String>,
    source_span: Option<SourceSpan>,
    dependency_path: Vec<PublicationRef>,
}

enum ConfigAccess {
    Require,
    Optional,
    Has,
}
```

`Require` 和 `Optional` 携带 `ty`；`Has` 只携带 path 的 presence 用法。

合并规则：

- 同一 path、同一 typed access shape：合并成一条 effective entry，保留全部
  provenance entry。
- 同一 path、同一 type，required 和 optional 同时出现：effective typed entry 取
  required；provenance 必须能说明是哪个来源把它变成 required 的。
- 同一 path 但类型不兼容：compile/link 必须在 activation 之前失败，并报告所有
  冲突的 provenance entry。
- 仅 presence 的 `has` 不会让 path 变成 required，但它必须对 activation metadata
  和诊断保持可见。

缺失/非法 activation config 的诊断必须能表达：

```text
Missing required config path packages.mongo.uri: string
Required by skiff.run/mongo@1.0.0 db.skiff:12
Dependency path example.com/app@0.1.0 -> skiff.run/mongo@1.0.0
```

具体显示格式可以变。契约要求的是：package id、version、dependency chain、path、
type、requiredness 和 source location 都对诊断可用。

## Service Activation Input

Runtime config values 是 activation 输入，不是 compiler 输入：

```rust
struct ServiceActivationInput {
    service_id: PublicationId,
    service_version: String,
    profile: Option<String>,
    service_unit: ServiceUnitInput,
    package_units: Vec<PackageUnitInput>,
    config_sources: Vec<RuntimeConfigSource>,
}

struct RuntimeConfigSource {
    path: String,
    source_class: ConfigSourceClass,
    value: JsonObject,
}

enum ConfigSourceClass {
    Bundle,
    Secret,
}
```

Activation 合并 `config.yml`、profile config 和 secret config，然后用 service/package
unit 携带的 effective config requirements 校验解析后的 config view。required 的
缺失/null 值导致 activation 失败；optional 的缺失/null 值允许；类型不匹配导致
activation 失败。

改 config 值不应要求重建 service artifact——除非 source 级别的 config
requirements 变了。

## LoweredPublication

`LoweredPublication` 是 lowering 的唯一输出。它包含从 `SourceCompileModel` 派生的
可执行 IR 和 lowering metadata。

```rust
struct LoweredPublication {
    files: Vec<FileIrUnit>,
    sources: Vec<CompiledSource>,
    operations: OperationLoweringIndex,
    storage: StorageLoweringModel,
    synthetic_operations: SyntheticOperationIndex,
}
```

阶段规则：

- 只消费 `SourceCompileModel`。
- 消费 `TypeResolutionModel` / `ExpressionTypeModel` 中已经校验的 source-level
  typed facts。
- 产出 File IR 和 typed lowering metadata。
- 用户 `impl` receiver call 必须在本阶段静态降为 executable call，例如
  `user.displayName()` 降为 `User.displayName(user)`。`DynamicReceiver` 只允许表示
  built-in physical shape receiver method、actor ref method，或未来显式
  interface/vtable value method。
- `throw` lowering 必须携带 payload static type 或 linked type identity；runtime 不得
  从普通 payload object 反查 source nominal type。
- 不直接读取 `service.yml` 或 package manifest。
- 不 parse 或生成 Skiff source text。
- 不查看 artifact JSON。
- 不通过另一条路径重新计算 name resolution、type resolution、expression type 或
  package ABI 事实。

HTTP/WebSocket wrapper 应在这里、或在本阶段拥有的专门 typed lowering submodel 里，
成为 typed synthetic operation。长期架构里它们不应表示为 generated Skiff source
text。

## ProjectionBundle

`ProjectionBundle` 包含 typed projection 输出，不是最终 JSON 文件。

```rust
struct ProjectionBundle {
    config: ConfigProjection,
    contract: Option<ContractProjection>,
    runtime_manifest: Option<RuntimeManifestProjection>,
    package_unit: Option<PackageUnitProjection>,
    service_unit: Option<ServiceUnitProjection>,
    artifact_index: ArtifactIndexProjection,
}
```

阶段规则：

- 消费 `CompiledPublication`（`SourceCompileModel + LoweredPublication`）和显式的
  typed projection context。
- 读取 typed config requirements、typed public API graph、typed File IR、typed
  package ABI、typed expression/type facts 和 typed service ingress/lowering
  metadata。
- 不查看 AST。
- 不调用 lowering helper。
- 不从 display string 重建类型事实。
- 不从 AST expression、AST declaration 或 lowering helper 重建 expression/type
  facts。
- 不把 `serde_json::Value` 当作内部协议来读写。

contract/runtime/package projection 可以把 typed model 序列化，但只能在一个显式
边界上、用于 identity 或最终 emission。如果 identity 要排除某些字段，这个
excluded-field 策略必须用 typed identity payload 表达，而不是序列化整个 DTO 再
删 JSON 字段。

## PublishedArtifacts

`PublishedArtifacts` 是最终 emission 边界。JSON 渲染、artifact path、hash 和
identity 都归这里。

```rust
struct PublishedArtifacts {
    file_ir_units: Vec<PublishedFileIrArtifact>,
    package_unit: Option<PublishedJsonArtifact>,
    service_unit: Option<PublishedJsonArtifact>,
    contract_schema: Option<PublishedJsonArtifact>,
    runtime_manifest: Option<PublishedJsonArtifact>,
    bundle: PublishedJsonArtifact,
    index: PublishedJsonArtifact,
}
```

阶段规则：

- 只消费 `ProjectionBundle` 和显式 emission context。
- 可以为最终 artifact 文件调用 `serde_json::to_value`。
- 可以通过 typed identity projection 计算 path、hash 和 identity。
- 不得做 semantic extraction、source parsing、lowering 或 config 值 activation。

判据区分（哪种 `serde_json::Value` 合规）：emission 阶段**产物**持有已渲染 JSON
是合规的——它就是 emission 输出本身。违规的是**内部阶段结构**里钉一份从自己已持有
的 typed model 派生出来的 `serde_json::Value` 副本，当作阶段间传递的协议。判断标准
是这份 JSON 是不是"emission 边界渲染一次的输出"，而不是"提前算好、层层携带的副本"。
例如 File IR artifact 只持 typed `unit`、JSON 在写出时按需渲染（合规）；若结构里同时
钉一个预算好的 `value` 字段，则违规。

## Ingress Adapter Contract

Service ingress（`http`、`websocket`）是 service-only 的 compile 输入。它应该作为
typed 事实流过 pipeline：

```text
service.yml ingress spec
  -> SourceCompileModel.ingress
  -> LoweredPublication.synthetic_operations
  -> ProjectionBundle.runtime_manifest / service_unit
  -> PublishedArtifacts
```

长期契约里没有 C 风格预处理器，也没有 generated Skiff source 阶段。历史上的
generated source ingress adapter 已移除；临时实现也不应重新引入这条 pipeline
阶段。

## Audit Targets

pipeline 要保护以下不变量：

- 任何 publication compile 阶段都不读 service root `config.yml`。
- `SourceCompileModel` 之后的阶段都不重建 source/resolution view。
- `LoweredPublication` 不重新推断 expression type，也不重建 name/type resolution。
- 任何 projection 阶段都不为生产逻辑 import AST declaration 类型。
- 任何 projection 阶段都不为生产逻辑 import AST expression 类型。
- 任何 projection 阶段都不调用 lowering helper。
- 任何 projection 阶段都不从 display string、AST 或 lowering helper 重建
  expression/type facts。
- 没有 generated Skiff source text 被当作长期 wrapper 协议。
- 没有 compiler 内部阶段产物是 raw artifact JSON value。
- 没有 identity projection 依赖「从序列化后的 artifact JSON 删字段」。

### 怎么保证这些不变量

主要靠**阶段接口的输入类型**保证，而不是事后扫描源码。

每个阶段是一个函数（或一组函数），它的输入类型就是它**能拿到的全部事实**。
projection 阶段的入口签名只收 `SourceCompileModel`、`LoweredPublication`、
`ContractProjection` 和 typed projection context——这些都是上游已经规整好的 typed
model，里面没有 AST 实例、没有 `config.yml` 原文、没有 raw artifact JSON。只要接口
这样定，下游阶段在正常数据流里就**摸不到**这些事实：要在 projection 里弄出一个 AST
节点或一份 config 原文，必须自己重新 parse 源码或重新读文件，那是蓄意另起炉灶，不是
顺着接口顺手拿到的。

这是这条 pipeline 的核心保证手段，也是判断违规的标准：看一个阶段是否只消费它入口
类型里携带的事实。`mod` 之间的可见性是否开着不是关键——**能 `import` 一个类型 ≠ 能
拿到它的实例**，数据来源由函数签名这条线索决定，不由 `pub` 可见性决定。

在此之上，可以用 crate 边界 / 模块可见性做**纵深防御**，把"蓄意绕过"也一并堵死：

- 把 AST declaration / expression 类型限制在前端 / lowering 边界内（不向 projection
  层 `pub use`），让 projection 连 `import` 都做不到，而不只是拿不到实例。
- 把 lowering helper 设为 lowering 阶段私有（`pub(crate)` 或更窄），projection
  调不到它们。
- identity 用 typed identity payload 表达，使"序列化 DTO 再删字段"这条路在类型上
  不存在。

纵深防御是加分项，不是这些不变量成立的前提：接口输入类型定对了，不变量就成立；
crate 边界只是把绕过它的代价从"自觉"提高到"编不过"。

不要用「禁止子串扫描某个文件里不得出现某个符号」这类测试来把关结构性不变量。它锁的
是字面符号不是语义，换个字段名或写法就失效，且容易给人"已达标"的错觉。结构是否达标
以本文契约和 `../implementation/compiler-architecture-audit-2026-06-03.md` 的 open
items 为准。

Implementation 文档可以追踪更细的迁移步骤，但不得削弱这些契约。
