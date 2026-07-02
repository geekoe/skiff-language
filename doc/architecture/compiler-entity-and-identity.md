# Compiler Entity And Identity Architecture

本文定义 Skiff compiler / artifact 中 `Entity`、名字解析、declaration anchor、public path、ABI
nominal identity 和 contract revision 的长期内部分层。Runtime 只作为下游链接消费方出现在边界说明里，
不拥有或决定 entity、ABI nominal identity 或 contract revision。本文是 architecture contract，不是用户可见
语言规范，也不是实现迁移计划。

## Scope

本文负责：

- compiler 如何把源码名字和 manifest/public API 路径解析为编译期 entity。
- publication 顶层 entity、函数局部 entity、type parameter 和 external entity 的 id 空间。
- `root`、`std`、package alias、service alias 这类 resolver root 与 entity 的区别。
- public path、source selector、declaration anchor、ABI nominal identity 和 contract revision 的归属。
- closure-only ABI type 如何不可命名但有 identity。
- compiler / artifact / runtime 各阶段可以使用哪些 key。

本文不负责：

- public API metadata 的文件格式。
- 用户可见 import、shadowing、visibility 或 service dependency 语法细节。
- 具体 hash bytes、artifact JSON 字段名或 Rust 模块拆分。
- registry compatibility policy、release pointer 或 runtime routing。
- JSON / DB / HTTP schema 字段名和外部协议名字。
- 普通 runtime value layout。

## Core Invariant

Skiff 必须区分这些东西：

```text
Name              -> source token / manifest token 中的名字字符串
NamePath          -> a.b.c 这类分段路径
ResolverRoot      -> root/std/package alias/service alias 等 lookup 起点
PathPrefix        -> qualified lookup 的中间前缀
Entity            -> 可作为完整引用最终结果的编译期实体
EntityName        -> 某个 scope/path 中指向 Entity 的名字入口
EntityRef         -> 某个 use site 解析后的 EntityId
DeclarationAnchor -> 某个 nominal/API declaration 的稳定身份锚点
PublicPath        -> 外部源码可写 API 名字
Descriptor        -> 类型或 callable 的 canonical shape / signature / schema fact
ABI nominal id    -> 类型相等和 ABI graph/link identity 的 artifact key
Contract revision -> descriptor/schema/signature compatibility key
Runtime address   -> 某次 activation/linking 后的执行地址或 slot
```

任何阶段不得把其中一个当成另一个：

- `root`、`std`、package alias、service alias 是 resolver root，不是 entity。
- `root.user`、`pkg.http`、`svc.user` 这类路径前缀不是 entity，除非语言显式允许它们作为完整引用结果。
- 字面量是 value，但不是 entity；它没有名字、定义点或引用一致性问题。
- public path 是 lookup / export surface，不是 entity id，也不是 nominal type identity。
- source module path 不是 public source path。
- source selector 不是 export。
- 裸 source selector 不是 ABI nominal identity；`PublicationIdentity + SourceSelector + kind` 可以作为第一版 source
  declaration anchor。
- File IR `type_index` 不是跨 file / package 稳定 identity。
- runtime `TypeAddr` / `ExecutableAddr` 不是 artifact 或 ABI nominal identity。
- display string 只用于诊断，不作为 canonical map key。
- type/schema metadata 负责外部协议编解码，不由 entity 负责。
- runtime 不通过源码名字字符串查找 ordinary symbol。

## Entity Model

`Entity` 是 compiler 的编译期命名实体。它由源码声明、参数、局部绑定、type parameter、
manifest binding 或 dependency metadata 引入，拥有 compiler 内部 id，用于 name resolution、
type checking、lowering 和诊断。

`Entity` 不是 runtime value。多数 entity 在 lowering/linking 后会消失，或被投影成 slot、
local index、ABI nominal id、contract revision、link target、runtime address、type descriptor 等执行所需事实。

Entity kind 表示 compiler 可解析到的实体类别。kind 不是 runtime tag。

```rust
enum EntityKind {
    Type,
    Alias,
    Interface,
    Function,
    ImplMethod,
    Const,
    DbObject,
    Local,
    Parameter,
    PatternBinding,
    TypeParameter,
    PackageCapability,
    ExternalPackageSymbol,
    ExternalServiceOperation,
    ExternalServiceInstance,
    BuiltinSymbol,
}
```

分类规则：

- 顶层 `type`、`alias`、`interface`、`function`、`const`、DB object 是 publication source entity。
- `impl` method 是 receiver method namespace 中的 entity，不进入普通顶层 source selector。
- 函数参数、局部变量、pattern binding、catch binding 等是 local entity。
- type parameter 是 type namespace 中的 local entity。
- package capability alias 是受控 receiver root entity，可以在 package source 中被调用，但不是普通 first-class runtime value。
- external package symbol 是 dependency package 的 public symbol 在当前 compilation 中的引用实体。
- external service operation / instance 是 remote linkage entity，不能和 package local symbol 合并。
- std / prelude / compiler-known built-in symbol 解析为 `BuiltinEntityId`，不是 `root` source entity，也不是
  package dependency entity。

## Entity Names

一个 entity 可以有多个名字入口。名字入口不是 entity 本身。

例如同一函数可以同时被这些路径找到：

```text
root.internal.user.getUser
pkg.user.get
```

第一条是 producer source set 内的 source lookup 路径，第二条是 consumer 通过 package dependency
public path 访问的路径。它们是不同 lookup context 下的 `EntityName` 或 external reference entry，
不能复制出两个 source entity。

```rust
struct EntityName {
    owner: NameOwner,
    path: NamePath,
    target: EntityId,
    namespace: EntityNamespace,
}

enum EntityNamespace {
    Value,
    Type,
}
```

`EntityName` 只用于 lookup。解析完成后，AST / HIR / source facts 中的引用点应持有
`EntityRef`，而不是继续保存 display path 作为语义 key。

```rust
struct EntityRef {
    target: EntityId,
    use_site: SourceSpan,
}
```

## Entity Id Spaces

Entity id 必须有明确 owner context，不能使用单个全局 `u32` 表示所有实体。

```rust
enum EntityId {
    TopLevel(TopLevelEntityId),
    ImplMethod(ImplMethodEntityId),
    Local(LocalEntityId),
    TypeParameter(TypeParameterEntityId),
    PackageCapability(PackageCapabilityEntityId),
    ExternalPackage(ExternalPackageEntityId),
    ExternalService(ExternalServiceEntityId),
    Builtin(BuiltinEntityId),
}
```

`TopLevelEntityId` 属于当前 publication 的 entity table：

```rust
struct PublicationEntityTable {
    publication: PublicationId,
    entities: Vec<TopLevelEntity>,
}

struct TopLevelEntityId {
    publication_local_index: u32,
}
```

`PublicationEntityTable.publication` 是 source compile owner id；ABI owner 使用下文的
`PublicationIdentity`。`publication_local_index` 只是当前 compiler table 的 owner-local handle，不是 ABI
nominal identity 输入，也不是 source declaration anchor。顶层 declaration 如果进入 ABI / schema
closure，必须另外投影到结构化 source declaration anchor；不能把 declaration 在文件、module 或
publication table 中的序号当作稳定身份。

`LocalEntityId` 属于一个 executable / const initializer / callback / pattern owner：

```rust
struct LocalEntityTable {
    owner: LocalEntityOwner,
    entities: Vec<LocalEntity>,
    scopes: Vec<LexicalScope>,
}

struct LocalEntityId {
    owner: LocalEntityOwnerId,
    local_index: u32,
}
```

`TypeParameterEntityId` 属于 type/function/interface/method generic owner：

```rust
struct TypeParameterEntityId {
    owner: GenericOwnerId,
    param_index: u32,
}
```

`BuiltinEntityId` 属于 compiler-known platform symbol registry：

```rust
struct BuiltinEntityId {
    registry: BuiltinRegistryId,
    symbol: BuiltinSymbolId,
    namespace: EntityNamespace,
}
```

`EntityId` 是跨 source compile model 传递的 typed id，因此每个 variant 必须包含 owner 或指向一个
owner-local table。`TopLevelEntityId { publication_local_index }` 只能在它所属的
`PublicationEntityTable` owner 上下文内解释；如果顶层 entity id 需要脱离该 table 跨 artifact /
package 边界流动，必须先投影为 declaration anchor、ABI nominal id 或 contract revision，不能裸传 local
index。

Lowering 可以把这些 id 投影成 File IR local indexes、slot indexes、type parameter indexes 或 link
targets，但投影后的数字仍必须有 owner context。裸 `3` 不是 stable identity。

## Publication Entity Table

一个 package 或 service 在 source compile 阶段共享同一种 publication entity table。package 和
service 的差异不在 entity table，而在 projection / linkage policy。

```rust
enum PublicationKind {
    Package,
    Service,
}

struct PublicationEntityModel {
    kind: PublicationKind,
    top_level: PublicationEntityTable,
    module_index: ModulePathIndex,
    resolver_roots: ResolverRootTable,
    local_tables: Vec<LocalEntityTable>,
}
```

`PublicationEntityTable` 覆盖当前 production source set 中所有顶层声明。文件和 module path 只是 lookup
组织结构，不是 runtime 地址，也不是 public path。

Source selector 是 source-layer lookup key，也是第一版顶层 source declaration anchor 的名字部分。典型
形态是：

```text
module.path.Symbol
```

Source selector 解析应先从 resolver root 进入 `ModulePathIndex`，再落到 `TopLevelEntityId`：

```text
root.<modulePath>.<symbol> -> TopLevelEntityId
```

Source selector 不携带 publication id，不是外部源码可写名字，也不是 ABI nominal identity。字符串可以作为
source metadata surface 或诊断显示，但 compiler 内部 index 应使用结构化 key。作为 declaration anchor
时，它必须保留完整 module path、source symbol name 和 declaration kind；不能替换成 declaration
ordinal。

```rust
struct SourceSelector {
    module_path: ModulePath,
    symbol: SymbolName,
    kind_hint: Option<SourceDeclarationKind>,
}

enum SourceDeclarationKind {
    Type,
    Alias,
    Interface,
    Function,
    Const,
    DbObject,
}
```

Public API graph 记录 public path 到当前 publication source entity 的关系，但 public path 不替代
`EntityId`。外部 dependency ABI symbols 可以进入 schema closure、signature 或 dependency facts，不作为
当前 publication public path 的 source target。

第一版中，当前 publication 的 source declaration anchor 使用结构化 source selector 加 declaration
kind：

```rust
struct SourceDeclarationAnchor {
    publication: PublicationIdentity,
    selector: SourceSelector,
    kind: SourceDeclarationKind,
}
```

因此：

- 同一 module / file 内重排 declaration，不改变 source declaration anchor。
- 添加或删除无关 sibling declaration，不改变既有 source declaration anchor。
- 把一个 nominal type 从一个 module / file 移到另一个 module / file，即使 public path 和 descriptor
  不变，也会得到不同 declaration anchor，视为另一个 nominal/API declaration。

这个规则保守但可程序化；未来如果需要支持“移动文件但保持同一 declaration identity”，必须显式引入
stable declaration id，不能靠 compiler 猜测开发者意图。

`impl` method 不进入顶层 source selector（见 Entity Model），因此它不独立持有 source declaration anchor。
进入 ABI 的 impl method（interface method implementation、public receiver method）其 ABI 身份由 owning
type / interface 的 declaration anchor 加 method name 在该 owner 的 descriptor 内承载，而不是单独申请一个
顶层 declaration anchor。重排同一 owner 内的 method、或在不改签名的前提下移动 method 所在文件，不改变
owner 的 declaration anchor；method 的签名变化体现在 owner 的 contract revision 中。

## Local Entity Tables

局部 entity table 由函数、method、const initializer、callback、match arm 等 owner 拥有。局部 entity
不进入 publication 顶层表，不进入 public API graph，不参与 ABI nominal identity 或 contract revision。

```rust
struct LexicalScope {
    parent: Option<LexicalScopeId>,
    names: Map<(EntityNamespace, Name), EntityId>,
}
```

实现也可以把 `names` 拆成 `value_names` / `type_names` 等分表；contract 是 lookup key 必须包含
namespace。不能用 `Map<Name, EntityId>` 表示长期模型，否则同一 spelling 在 value/type namespace
并存时会丢信息。

参数、局部变量、pattern binding 等都创建 local entity。字面量、临时表达式和匿名 record literal 不创建
entity。编译器可以为临时值创建 IR temp 或 slot，但这些不是 source entity。

解析完成后，局部 value 引用应指向 `LocalEntityId`。Lowering 再把 `LocalEntityId` 映射到 frame slot：

```text
LocalEntityId -> SlotIndex
```

slot index 是执行布局，不是 name resolution identity。

## Resolver Roots

Resolver root 是 name/path lookup 起点，不是 entity。它不能作为普通表达式或类型引用的最终结果。

```rust
enum ResolverRoot {
    CurrentPublicationRoot,
    StdRoot,
    PackageDependency { alias: PackageAlias, dependency_slot: u32 },
    ServiceDependency { alias: ServiceAlias, dependency_slot: u32 },
    ConfigRoot,
}
```

规则：

- `root` 指向当前 publication source set 的 module/top-level lookup。
- `std` 指向 compiler-provided platform namespace。
- package alias 指向 dependency package public surface，后续调用使用 local linkage。
- service alias 指向 service dependency public operation / public instance surface，后续调用使用 remote linkage。
- `config` 指向 publication config requirement API，不是普通 runtime object。

Resolver root 不创建 `EntityId`。如果某个 language feature 未来允许把 package、service、config 或
namespace 当作 first-class value，它必须显式引入新的 entity kind 和 runtime value layout；不能复用
resolver root。

`std.foo.Bar` 这类路径从 `StdRoot` 开始 lookup；完整路径若落到 compiler-known type、callable 或 const，
最终结果是 `EntityId::Builtin(BuiltinEntityId)`。`std` 本身和中间 prefix 仍不是 entity。

## NamePath Resolution

NamePath resolution 按 context 区分 value position、type position、call position、constructor position、
receiver-method position 和 manifest selector position。

解析过程可以经过多个 path prefix，但只有完整路径的最终结果可以成为 entity ref。

```text
pkg.user.get
```

`pkg` 是 package resolver root，`user` 是 public path prefix，`get` 是最终 public callable。`pkg.user`
不是 entity，也不要求唯一对应某个实体；它只在继续 lookup 时有意义。

同理：

```text
root.internal.User
```

`root` 是 resolver root，`internal` 是 module path prefix，`User` 才可能解析到 type entity。

解析结果必须是 typed result：

```rust
enum ResolvedPath {
    Entity(EntityId),
    PathPrefix(PathPrefixId),
    ResolverRoot(ResolverRootId),
}
```

表达式、类型、constructor、manifest selector 等 consumer 只能接受它们所在 context 允许的最终结果。
不能把 `PathPrefix` 或 `ResolverRoot` 当作 entity fallback。

## Package And Service References

Package dependency 和 service dependency 都从 resolver root 开始，但它们产生不同 entity kind 和不同
linkage。

Package reference：

```text
pkg.some.path
  -> ExternalPackageEntityId
  -> package public symbol / ABI expectation
  -> local linkage
```

Service reference：

```text
svc.someOperation
  -> ExternalServiceEntityId
  -> service dependency operation / public instance metadata / protocol expectation
  -> remote linkage
```

二者不能合并成同一种 external symbol：

- package callable 可以在当前 runtime program 中 local link 到 executable target。
- service operation 是 remote call contract，必须携带 service dependency、operation target、mode、protocol revision 和 boundary schema expectation。
- service public instance 是 remote receiver root metadata，不是 dependency package object。
- package public path 和 service operation path 即使 display string 相同，也必须产生不同 entity kind。

Lowering 必须消费 name resolution 的 typed result，生成对应 call target：

```rust
enum ResolvedCallable {
    LocalFunction(TopLevelEntityId),
    LocalImplMethod(ImplMethodEntityId),
    PackageFunction(ExternalPackageEntityId),
    ServiceOperation(ExternalServiceEntityId),
    Builtin(BuiltinCallableId),
}
```

Runtime 不再通过字符串判断一次 call 是 package call 还是 service call。

`BuiltinCallableId` 是 `BuiltinEntityId` 在 call position 的投影（call-target 视角的 builtin 句柄），不是另一个
独立 id space；二者必须能互相对应。builtin 不进入 `AbiSymbolId`，因为它不是 package / service boundary 上
可观察的 published symbol。

## Type And Value Namespaces

Entity 可以属于不同 namespace。Skiff compiler 至少需要 value namespace 和 type namespace。

Value namespace 包含：

- local / parameter / pattern binding。
- top-level const。
- function and callable references where the language permits named callable use。
- package capability receiver root。
- external package callable / const。
- external service operation / public instance access。
- compiler-known std/prelude callable / const.

Type namespace 包含：

- type。
- alias。
- interface。
- type parameter。
- external package type / alias / interface。
- compiler-known std/prelude type.

同一个 source spelling 可以在不同 namespace 中解析为不同 entity，前提是用户语义允许。解析结果必须携带
namespace，后续阶段不得只按短名匹配。

## Public Path

Public path 是外部源码可写 API 名字，例如：

```text
getUser
user.get
http.Request
```

public path 的职责只有一个：定义 package / service API surface 中的外部名字。

```rust
struct PublicApiBinding {
    public_path: PublicPath,
    target: PublicApiTarget,
    kind: PublicSymbolKind,
}

enum PublicApiTarget {
    Source(PublicSourceEntityId),
}

enum PublicSymbolKind {
    Type,
    Alias,
    Interface,
    Callable,
    Const,
    PublicInstance,
}

struct PublicSourceEntityId {
    entity: TopLevelEntityId,
    kind: PublicSourceEntityKind,
}

enum PublicSourceEntityKind {
    Type,
    Alias,
    Interface,
    Function,
    Const,
    PublicInstance,
}
```

Public API target 必须是当前 publication 中可公开的顶层 source entity。它不能是 `LocalEntityId`、
`TypeParameterEntityId`、`PathPrefix`、`ResolverRoot` 或普通 external dependency symbol。外部 package /
service / std ABI symbols 可以出现在 schema closure、signature、binding requirement 或 dependency lock
中，但不是 public path 的 source target。

public path 改变是 source API surface 改变，但 public path 不是 nominal type identity。显式 public
type 同时有 public path 和 `AbiTypeId`；closure-only type 只有 `AbiTypeId`，没有 public path。

lookup 过程可以通过 public path 找到一个 ABI symbol，但解析完成后，type checking、schema closure 和
artifact linking 必须消费 `AbiSymbolId` / `AbiTypeId`，不能继续把 public path 字符串当语义 key。

## Declaration Anchor And Descriptor

Nominal identity 和 descriptor / schema revision 是两类事实：

```text
DeclarationAnchor -> 这是哪个 declaration / symbol
Descriptor        -> 这个 declaration 的结构、签名或 schema 形状
```

第一版 source declaration anchor 使用 `PublicationIdentity + SourceSelector + kind`。`PublicationIdentity`
是 ABI owner identity，不是 build id。它固定由 stable publication id 和显式 ABI epoch 组成；`abi_epoch`
默认值为 `0`，只有开发者或 registry policy 明确要求切断 nominal lineage 时才递增。普通 publication
version、source hash 和 build id 不进入 nominal declaration anchor。

```rust
struct PublicationIdentity {
    id: PublicationId,
    abi_epoch: AbiEpoch,
}
```

这意味着：

- source module path 或 source symbol 改变，会改变 declaration anchor。
- 同一个 declaration anchor 下，descriptor 可以变化。
- descriptor 变化必须改变 contract / schema revision，即使 declaration anchor 和 public path 没变。
- declaration anchor 变化必须改变 nominal identity，即使 descriptor 完全相同。
- 同一 `PublicationIdentity` 下，同一 source declaration anchor 跨 publication versions 保持 nominal
  identity；compatibility 由 contract / schema revision 判断。

例如：

```skiff
type Id = string
```

改为：

```skiff
type Id = number
```

如果它仍在同一个 source selector 和 `PublicationIdentity` 下，`AbiTypeId` 仍表示“同一个 nominal
declaration”，但 descriptor / schema revision 必须变化，package ABI contract revision 或 service
protocol revision 也必须变化。Compatibility 检查据此判断这是同一类型的不兼容演进，而不是把它误认为
完全无关的新类型。

Contract revision 应基于 canonical ABI graph，而不是源码声明顺序：

- map / set 按 canonical key 排序。
- 参数、tuple、泛型实参、union variants 等语义有序位置保留顺序。
- source span、声明顺序、File IR local index 和 runtime address 不进入 contract hash。
- 顶层 source declaration 的 ABI nominal identity 使用 declaration anchor，不使用序号。
- 序号只允许用于语言本身定义为 positional 的事实，或带 owner context 的 compiler / IR / runtime 局部布局。
- source selector 只作为结构化 declaration anchor 的组成部分进入相关 nominal identity；不作为 display
  string 或 export path 使用。

## ABI Nominal Identity And Contract Revision

ABI nominal identity 是 package / service boundary 上可观察 declaration 的稳定身份。它用于 type equality、
schema closure graph references、operation projection 和 artifact linking。

```rust
enum AbiSymbolId {
    Type(AbiTypeId),
    Alias(AbiAliasId),
    Interface(AbiInterfaceId),
    Callable(AbiCallableId),
    Const(AbiConstId),
    Instance(AbiInstanceId),
}
```

具体 bytes / string encoding 由 artifact identity 层定义，但 nominal identity 的语义输入只能包含：

- owning publication identity。
- declaration anchor，包括 external declaration anchor。
- symbol kind。
- 对泛型实例化而言，完整 type arguments 的 ABI nominal ids。

ABI nominal identity 可以引用 source declaration anchor 作为输入，但不能暴露 source selector 作为外部源码
路径。`AbiTypeId` 不吞入 descriptor bytes、schema hash、publication version 或 build id；无关 public API
改动和同 declaration 的 descriptor 演进不应导致 nominal type identity 整体 churn。

Descriptor / schema / signature 变化由 contract revision 表示：

```rust
struct AbiContractRevision {
    descriptor_hash: DescriptorHash,
    schema_revision: SchemaRevision,
}
```

`AbiContractRevision` 不自带 `AbiSymbolId`。它的归属由持有它的外层 fact 决定（例如 `AbiTypeFact.type_id`
或某个 callable / const fact 的 nominal id）；同一个 nominal symbol 在一份 fact 里只出现一次，避免 nominal
id 与 contract revision 各存一份导致不一致。

Package ABI expectation、service protocol revision 和 compatibility checking 必须同时消费 nominal
`AbiSymbolId` 和对应 `AbiContractRevision`。Type equality 只使用 `AbiTypeId`；wire compatibility 和
protocol matching 使用 contract revision。

## ABI Type Identity

`AbiTypeId` 是判断“是否同一类型”的 canonical key。它适用于：

- 显式 public type。
- public callable 参数 / 返回中引用的 named type。
- public type body、alias target、interface method signature 引用的 named type。
- public const declared type。
- public instance receiver type 和 interface identities。
- schema closure 中的 closure-only named type。

```rust
struct AbiTypeFact {
    type_id: AbiTypeId,
    declaration_anchor: AbiDeclarationAnchor,
    source_entity: Option<TopLevelEntityId>,
    public_path: Option<PublicPath>,
    nameability: TypeNameability,
    descriptor: CanonicalTypeDescriptor,
    contract_revision: AbiContractRevision,
}

enum AbiDeclarationAnchor {
    Source(SourceDeclarationAnchor),
    External(ExternalDeclarationAnchor),
    Std {
        symbol: StdSymbolId,
    },
}

struct ExternalDeclarationAnchor {
    owner_publication: PublicationIdentity,
    declaration: PublishedDeclarationId,
    kind: AbiDeclarationKind,
}

struct PublishedDeclarationId {
    stable_id: String,
}

enum AbiDeclarationKind {
    Type,
    Alias,
    Interface,
    Callable,
    Const,
    Instance,
}

enum TypeNameability {
    PublicNameable,
    ClosureOnly,
}
```

`ExternalDeclarationAnchor` 是 dependency artifact 发布的 declaration anchor。它不能包含
`AbiSymbolId`、generic type arguments、descriptor hash、schema revision 或 runtime address；这些分别属于
ABI nominal symbol instantiation、contract revision 和 runtime linking。它也不能退化成 consumer 看到的
public path，因为 public path 是 lookup/export surface，不是 declaration anchor。

`PublishedDeclarationId.stable_id` 是 dependency 发布时固化的不透明 token，由发布方 artifact 生成并冻结。
它不是 display path、public path 或 source selector，consumer 不得反解析或据它重建源码名字；它只作为跨
artifact 引用同一 published declaration 的稳定 key。这与“display string 不作 canonical map key”不冲突：
被禁止的是把诊断/显示用的人类可读字符串当 key，而不是禁止使用稳定不透明 id。

`ClosureOnly` type 是 ABI-visible 但 source-unnameable：compiler / IDE 可以通过 inference 使用它，
runtime / artifact 可以用它做 schema 和 link，但外部源码不能直接书写它的 public name。

Alias 是透明类型缩写，不创建 nominal `AbiTypeId`；alias declaration 可以有 `AbiAliasId`、
ABI metadata 和 public path，但 assignability / schema descriptor 按 target type 展开。

Interface 有 `AbiInterfaceId`。Interface identity 包含 interface symbol identity 和完整 type arguments；
interface 不是普通 schema payload type，不应被伪装成 record descriptor。

## Type Equality

Nominal type equality 使用 `AbiTypeId`，不使用 public path、short name 或 runtime address。Source selector
只在它被结构化封装进 declaration anchor 时参与 source-defined nominal identity；裸字符串 selector 不作为
type equality key。

因此下面判断必须能成立：

```skiff
const user = pkg.getUser()
pkg.updateUser(user)
```

即使 `getUser` 返回的 named type 没有 public path，只要 `updateUser` 参数的 `AbiTypeId` 相同，type
checker 就应接受。

相反，两个 type 即使字段完全相同，只要 `AbiTypeId` 不同，就不是同一 nominal type。

Type checker 和 IDE 必须把“可推断”与“可书写”分开。

对于 closure-only type：

- 可以作为 expression inference 结果。
- 可以用于 field access、match、schema encode/decode 和同 ABI type id 的 API 传参。
- 可以在 tooltip / diagnostics 中显示 diagnostic label。
- 不能作为外部源码中的显式 type annotation、constructor target 或 package namespace lookup。

IDE 展示不应伪造合法源码名。推荐展示：

```text
user: User (ABI-only, returned by pkg.getUser; not nameable)
```

或：

```text
user: <unexported package ABI type internal.user.User>
```

其中 `internal.user.User` 只是 diagnostic source label，不是用户可写 public path。

## Runtime Linkage Boundary

Runtime 不参与 entity identity、declaration anchor、ABI nominal identity 或 contract revision 的生成。
Runtime address 是 linking 之后的执行地址：

```rust
TypeAddr
ExecutableAddr
ConstAddr
FileAddr
SlotIndex
```

这些地址只在某个 linked runtime program image 或 request frame 内有效。runtime 可以建立：

```text
AbiTypeId -> TypeAddr
AbiCallableId -> ExecutableAddr
EntityId -> local lowering/link target -> runtime address
```

但不能把 runtime address 写回 artifact DTO，也不能让 artifact/projection 依赖某次 activation 的 address。
Runtime may keep type descriptors, protocol metadata, operation names, JSON field names or source maps for boundary
handling and diagnostics. Those strings are not entity lookup keys for ordinary execution.

## Artifact Boundary

Artifact DTO 可以继续使用局部 index 和 structured symbol refs，但必须满足：

- File-local references 可以用 `LocalType { type_index }`，但跨 file/package boundary 必须带 owner context
  或能恢复 `AbiTypeId`。
- Package / service symbol refs 必须是结构化 key，不是 display string。
- Package exports table 只列 public path -> declaration / link target；closure-only types 不应混入 public
  exports table。
- ABI/schema closure table 必须能表达 explicit public type 与 closure-only type 的区别。
- service operation refs 必须保留 remote linkage metadata，不能和 package local export refs 合并。

长期目标是 artifact projection 明确产出：

```rust
struct AbiIdentityProjection {
    public_symbols: Map<PublicPath, AbiSymbolId>,
    types: Map<AbiTypeId, AbiTypeFact>,
    aliases: Map<AbiAliasId, AbiAliasFact>,
    interfaces: Map<AbiInterfaceId, AbiInterfaceFact>,
    callables: Map<AbiCallableId, AbiCallableFact>,
    consts: Map<AbiConstId, AbiConstFact>,
}
```

具体 wire shape 可以不同，但必须保留这些事实。

## Pipeline Ownership

`SourceCompileModel` owns:

- resolver roots。
- source selector resolution。
- publication and local entity tables。
- entity refs for source use sites。
- public API bindings。
- resolved type facts。
- ABI type facts needed by publication API graph。

`LoweredPublication` owns:

- File IR local tables and local indexes。
- slot layout derived from local entity refs。
- link target closure derived from ABI/public roots。
- lowering metadata needed by typed projections。

Artifact projection owns:

- public export table。
- ABI / schema closure table。
- package ABI expectation and service protocol projection。
- structured package and service dependency refs。

Runtime linking owns:

- artifact refs / ABI ids to runtime addresses。
- linked package export overlay。
- remote service operation dispatch metadata。
- runtime dispatch indexes。

No downstream stage should rediscover identity by parsing display type names, AST text, artifact JSON paths or public path
strings.

## Diagnostics

Diagnostics may use source names and display paths, but diagnostics must be derived from typed resolution facts.

Each unresolved or ambiguous path diagnostic should report:

- original `NamePath` and source span。
- lookup context。
- resolver root if one was selected。
- candidate entity names if resolution found multiple final candidates。
- reason the final candidate was rejected, such as wrong namespace or wrong linkage kind。

Diagnostic display strings are not canonical map keys.

## Verification Contract

Architecture-level tests should cover:

- `root` resolves as `ResolverRoot::CurrentPublicationRoot` and cannot be used as an entity final result。
- `root.<module>.<symbol>` resolves to a top-level entity id。
- path prefix such as `pkg.user` is not treated as an entity unless a full path resolves to a valid final result。
- local variable and parameter references resolve to local entity ids, then lower to slots。
- type parameter references resolve to type-parameter entity ids, not short strings。
- package callable reference resolves to `ExternalPackageEntityId` and lowers to local package linkage。
- service operation reference resolves to `ExternalServiceEntityId` and lowers to remote service linkage。
- same display path under package alias and service alias does not produce the same entity kind。
- literals and anonymous temporaries do not create source entities。
- closure-only return type can be inferred and passed to another API requiring the same `AbiTypeId`。
- closure-only type cannot be written as a package public name or explicit type annotation。
- two same-shape nominal types with different ABI type ids do not compare equal。
- two public paths cannot collapse distinct source entities into one type identity。
- changing a source type descriptor changes ABI contract revision even when declaration anchor and public path stay the same。
- moving a source type to a different source selector changes declaration anchor and nominal identity, even when descriptor
  and public path stay the same。
- reordering top-level declarations in the same source selector space does not change declaration anchors or ABI nominal
  identity。
- adding an unrelated sibling top-level declaration does not renumber existing declaration anchors or change their nominal
  identity。
- changing public path changes export surface without making public path the only type identity。
- File IR local `type_index` cannot be used outside its owning file without owner context。
- runtime `TypeAddr` equality is not used as artifact/ABI equality across activations。
- runtime call execution does not parse source display paths to find symbols。
