# 编译器 / 运行时共享 Artifact 类型契约

本文定义编译器（compiler）、运行时（runtime）和路由器（router）围绕 artifact
类型、identity 投影和 linked runtime overlay 的长期内部契约。它是目标态架构文档，
不是用户可见语言规范，也不是迁移 checklist。当前实现偏差仍以
`../implementation/` 下的实施记录为准。

文中的 Rust-ish 类型只表达归属和阶段边界，不是 public API。

## 范围

本文负责：

- 编译器产出的 canonical artifact DTO 应由谁拥有。
- 运行时加载 artifact 后，哪些类型继续共享，哪些必须转成 linked/runtime-only
  overlay。
- 编译器/运行时可共享的 identity hashing 逻辑放在哪里。
- 路由器因为是 TypeScript 无法共享 Rust 类型时，如何用 parity fixtures 绑定行为。
- `serde_json::Value` 可以出现在哪些边界，以及哪些场景不能再把它当内部类型副本。

本文不负责：

- 用户语言语义、source syntax、config YAML schema。
- 运行时 request execution、DB/HTTP/file/native host 细节。
- 普通 runtime value layout、request-scope memory 和 type erasure 细节。该契约见
  `runtime-value-layout-and-type-erasure.md`。
- registry、release pointer、dev sync 和部署操作流程。
- 具体模块重排步骤。

## 归属

长期归属只有四层：

```text
compiler typed publication model
  -> artifact-model canonical DTOs
  -> artifact-identity shared identity projections
  -> runtime linked program image
```

路由器位于 Rust 共享代码之外：

```text
artifact JSON on disk
  -> router TS artifact reader
  -> skiff-artifact-identity CLI boundary
  -> parity fixture with artifact-identity/runtime/compiler
```

### `skiff-artifact-model`

`skiff-artifact-model` 拥有 canonical artifact DTO：也就是编译器写入磁盘、运行时
反序列化读取的 wire/disk shape。

典型类型：

- `FileIrUnit`
- `FileIrRef`
- `PackageUnit`
- `PackageExportIndex`
- `PackageDependencyConstraint`
- `ServiceUnit`
- `ServiceMeta`
- `ServiceOperation`
- `GatewayConfig`
- `ServiceConfigMetadata`
- shared type refs、executable signatures、DB/actor metadata，以及作为 artifact DTO
  存在的 native signature descriptors。
- recoverable boundary plan、expected type plan、custom restore plan 和 native adapter plan。

规则：

- 编译器在最终 artifact projection 阶段产出这些 DTO。
- 运行时在 artifact loader 边界直接反序列化这些 DTO。
- artifact type refs、type descriptors、runtime descriptors 和 executable signatures 是
  schema/ABI/linking facts，不定义普通 `RuntimeValue` 的物理布局，也不能要求普通值携带
  source type name 或 `__skiffType`。
- recoverable metadata 描述边界和恢复 ABI，不是普通 heap object 的隐式 tag。
- 这个 crate 不拥有 compiler lowering、runtime linking、runtime activation、router
  routing、部署逻辑或 identity hashing。
- canonical DTO 的 unknown fields 默认 fail closed，除非某个字段被明确记录为开放
  payload。

### `skiff-artifact-identity`

`skiff-artifact-identity` 拥有需要在多个 Rust 子系统之间逐字节一致的 identity
投影。

典型内容：

- runtime program service-unit identity projection。
- runtime program dynamic build id 的 hash framing。
- 这些 identity projection 使用的 canonical JSON normalization。

规则：

- 编译器 publish 和运行时必须调用同一份 Rust 实现来计算共享 identity
  projection。
- projection 名称必须对应它实际计算的 identity。runtime program build identity 不能
  悄悄复用 service unit artifact identity。
- 长期 source of truth 是 canonical typed DTO。任何 raw JSON 入口要么先 parse 成
  canonical DTO，要么必须用测试证明它和 typed 入口应用了完全一致的 default 和字段
  normalization。
- identity code 不放进 `skiff-artifact-model`；DTO 归属和 hash 归属必须分开。

### 运行时程序类型

运行时拥有 linked/executable state 和 activation view。下面这些不是 artifact DTO：

- `LinkedProgramImage`
- `RuntimeActivation`
- `RuntimeProgramIdentity`
- `LinkedFileUnit`
- `LinkedTypeRef`
- `LinkedCallTarget`
- `LinkOverlay`
- `FileAddr`、`TypeAddr`、`ExecutableAddr`
- route maps 和 dispatch indexes
- package slot layout 与 runtime package config vector
- linked package export overlay

`RuntimeProgram` 这个旧名字不属于目标态 architecture contract。任何同时包含 linked
image、activation metadata 和 identity bits 的 `RuntimeProgram` 结构都不是规范性模型。
目标态 production path 使用 `LinkedProgramImage` + `RuntimeActivation` + explicit identity。

运行时可以为 canonical DTO 保留很小的 extension traits，例如
`OperationTargetRef` 上的 helper method。但运行时不应通过 re-export 把 artifact
DTO 包装得像 runtime 自己拥有的 wire struct。artifact 边界上的 import 应让
`skiff_artifact_model` 这个来源可见。

linked type refs、linked call targets、runtime type plans 和 package export overlay 是
执行计划，不是普通 value tag。运行时可以在 linked state 中保留 source nominal
identity 用于校验、边界 decode/encode、exception catch、interface adapter 或 method
linking，但这些 identity 不得被复制进每个 object/map key/representation heap node。
需要 runtime nominal identity 的机制必须显式建模为 exception envelope、tagged value
或 interface/vtable value。

## Artifact DTO 流向

编译器 publication 把 typed source/lowering facts 投影为 canonical artifact DTO：

```rust
struct PublishedArtifacts {
    file_ir_units: Vec<FileIrUnit>,
    package_unit: Option<PackageUnit>,
    service_unit: Option<ServiceUnit>,
}
```

运行时 artifact loading 消费同一批 DTO：

```rust
struct LoadedArtifactGraph {
    service_unit: Arc<artifact_model::ServiceUnit>,
    service_files: Vec<Arc<artifact_model::FileIrUnit>>,
    package_units: Vec<Arc<artifact_model::PackageUnit>>,
    package_files: Vec<Vec<Arc<artifact_model::FileIrUnit>>>,
    identities: ArtifactGraphIdentities,
}
```

linking 是 runtime-only 类型出现的第一个阶段：

```rust
fn link_runtime_program_image(
    service: Arc<artifact_model::ServiceUnit>,
    service_files: Vec<Arc<artifact_model::FileIrUnit>>,
    packages: Vec<Arc<artifact_model::PackageUnit>>,
    package_files: Vec<Vec<Arc<artifact_model::FileIrUnit>>>,
) -> LinkedProgramImage;
```

linking 之后，运行时不应通过修改 canonical DTO 来追加 linked 字段。linked facts 应
存放在运行时拥有的 overlay 和 index 里。

## 包导出 Overlay

`PackageExportIndex` 是 canonical artifact data。运行时 package export lookup 是
linked data。

canonical shape：

```rust
struct PackageExportIndex {
    types: Map<String, TypeExport>,
    constants: Map<String, ConstExport>,
    functions: Map<String, ExecutableExport>,
    impl_methods: Map<String, ExecutableExport>,
}
```

runtime overlay：

```rust
struct LinkedPackageExportIndex {
    types: Map<String, LinkedTypeExport>,       // FileIrRef -> FileAddr
    constants: Map<String, LinkedConstExport>,  // TypeRefIr -> LinkedTypeRef
    functions: Map<String, LinkedExecutableExport>,
    impl_methods: Map<String, LinkedExecutableExport>,
}
```

规则：

- overlay 在 runtime linking 阶段派生。
- overlay 不写回 artifact JSON。
- overlay 可以丢弃运行时不需要的 compiler-only descriptor 信息，但这个丢弃必须发生
  在 overlay conversion 里，不能通过弱化 canonical artifact DTO 来实现。
- `impl_methods` overlay 用于静态链接 user impl method call。普通 `user.method()` 不应
  依赖 runtime object 上的 source type metadata 做 dynamic receiver lookup。

## Service Unit 的 Runtime 视图

`ServiceUnit` 在 loading 和 linking input 阶段保持 canonical。运行时可以为了执行
便利把 linked/executable facts 投影进 `LinkedProgramImage`：

```rust
struct LinkedProgramImage {
    routes: HashMap<String, ExecutableAddr>,
    operations: HashMap<String, ExecutableAddr>,
    spawn_routes: HashMap<String, ExecutableAddr>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
}
```

本机 runtime config 和 service activation facts 投影进 `RuntimeActivation`：

```rust
struct RuntimeActivation {
    service: ServiceMeta,
    version: String,
    package_configs: Vec<OpaqueConfigValue>,
    service_dependencies: Vec<ServiceDependencyConstraint>,
    timeout: ServiceTimeoutConfig,
    operation_route_bindings: Vec<OperationRouteBinding>,
    db: Vec<DbMetadataIr>,
    actors: Vec<ActorMetadataIr>,
    gateway: GatewayConfig,
}
```

规则：

- `ServiceMeta`、`GatewayConfig`、`ServiceConfigMetadata`、`ServiceOperation` 都是
  canonical artifact-model DTO。
- 运行时不维护降级成 `raw: Value` 的本地 canonical 副本。
- Package DB metadata 不是 runtime package-unit input。package DB metadata 必须在
  runtime linking 之前投影进 service unit DB metadata。
- routes、executable addresses 和 linked type context 是 linked image facts，不属于
  `ServiceUnit`。
- package config values、timeout、DB/actor/gateway activation view 是 runtime activation
  facts，不属于 `ServiceUnit` 或 `LinkedProgramImage`。

## Recoverable Boundary Metadata

Recoverable value 使用 compiler 产出的 artifact metadata 闭合边界语义。第一版不要求公开 source 语法，但 artifact
metadata 必须足够让 runtime 在 encode/decode 时 fail closed，而不是在业务调用点迟发错误。用户可见语义见
[`../reference/static-semantics.md §18`](../reference/static-semantics.md#18-boundary-policy-and-recoverable-values)，
完整架构见 [`recoverable-value.md`](recoverable-value.md)。

边界 plan 的 canonical 形态应包含：

```rust
struct RecoverableBoundaryPlan {
    boundary_id: String,
    boundary_kind: RecoverableBoundaryKind,
    trust_boundary: RecoverableTrustBoundary,
    expected_type_plan: RuntimeTypePlan,
    explicit_recoverable_slot: bool,
    db_storage_lane: Option<DbRecoverableStorageLane>,
    requires_runtime_carrier_check: bool,
}

enum RecoverableBoundaryKind {
    DbPayload,
    SpawnPayload,
    QueuePayload,
    RuntimeWirePayload,
    ServicePayload,
    PublicApiPayload,
    Materialization,
}

enum RecoverableTrustBoundary {
    OwnerInternal,
    CrossService,
    ExternalUntrusted,
}

enum DbRecoverableStorageLane {
    SchemaProjectable,
    RecoverableEnvelope,
}
```

`DbPayload`、`SpawnPayload`、`QueuePayload` 和 owner-internal `RuntimeWirePayload` 本身就是 recoverable boundary；
`explicit_recoverable_slot` 对它们不是 public ABI 开关。service/public API ordinary payload 不调用 recoverable codec；
只有 ABI/schema 明确标记的 slot 才生成 `explicit_recoverable_slot = true`，且第一版离开 owner service trust domain 时只允许
plain data envelope。

custom restore 的 artifact metadata 必须闭合：

```rust
struct CustomRestorePlan {
    concrete_type_identity: String,
    durable_state_type_plan: RuntimeTypePlan,
    encode_hook_id: String,
    decode_hook_id: String,
    restore_capability: RestoreCapability,
}

enum RestoreCapability {
    PureRecoverableRestore,
}
```

native adapter 的 artifact 或 builtin metadata 必须闭合：

```rust
struct NativeAdapterPlan {
    adapter_identity: String,
    adapter_schema_version: String,
    native_type_identity: String,
    durable_state_type_plan: RuntimeTypePlan,
    encode_hook_id: String,
    decode_hook_id: String,
    owner: NativeAdapterOwner,
    schema_compatibility: AdapterSchemaCompatibility,
}

enum AdapterSchemaCompatibility {
    Exact,
    Accepts(Vec<String>),
}
```

规则：

- `CustomRestorePlan` 属于当前 linked program 中的 `LocalConcrete` restore entry。recoverable payload 只保存 stable
  `LocalConcreteRestoreKey`；decode 必须在当前 service execution context 内唯一定位当前 concrete type、durable state plan
  和 hook。runtime wrapper 不保存 `restore_schema_version`，应用级状态迁移必须由 concrete type 或 DB schema migration
  显式定义。
- `NativeAdapterPlan` 由 builtin registry 或 artifact metadata 提供。decode 必须校验
  `adapter_identity`、`adapter_schema_version` 和 `native_type_identity`，不兼容则 fail closed。
- encode/decode hook 只在 `PureRecoverableRestore` capability 下执行，不得做 DB write、HTTP/WebSocket、spawn、文件写入、
  外部 clock/random 或其它不可回滚副作用。
- `requires_runtime_carrier_check` 标记 `any I` carrier、自定义恢复输出或 external envelope 内容这类必须看 runtime value
  才能判定的场景；静态已知不可恢复类型不得靠该标记延后。

## Identity 契约

这里有两个不同 identity，输入也不同：

```text
service unit artifact identity
  input: canonical ServiceUnit artifact
  purpose: artifact path/content identity

runtime program dynamic build id
  input: runtime-program service-unit identity projection
       + ordered package build identities
  purpose: router/runtime registration and request routing
```

runtime program service-unit identity projection 不等同于 service unit artifact
identity。前者投影的是 linked runtime program 相关字段，并额外折入 package build
identities。

规则：

- Rust 投影实现只放一份，归 `skiff-artifact-identity`。
- 编译器 publish 和运行时都调用这份共享 Rust 实现。
- 路由器不再手写 build-id identity 投影或 hash；它通过 `skiff-artifact-identity`
  CLI boundary 调用同一份 Rust source of truth，并通过 cross-system fixtures 绑定
  stdin/stdout 契约与 fixed expected identity string。
- projection 必须只做一次 optional/default artifact fields normalization。对于同一个
  canonical artifact，typed DTO input、raw JSON input 和 router CLI input 必须产出同一
  组 bytes。
- golden fixtures 必须同时覆盖非空字段和省略/default 字段，尤其是会影响 serde 输出的
  字段：`db`、`processes`、package dependency `config`、operation `effects`、
  operation `params`、optional timeout 和 optional
  `operation.target.executableIndex`。
- fixture 不能只断言“三方相等”。它还必须锁住具体 expected identity string，避免三份
  实现一起漂移但测试仍然通过。

## 路由器边界

路由器不能共享 Rust DTO 类型，但必须共享 artifact contract。

规则：

- 路由器读取 artifact JSON，并校验 routing 所需的 canonical schema fields。
- 路由器的 dynamic build id production path 必须调用 `skiff-artifact-identity` CLI。
  router 侧只能拥有 artifact JSON 读取、schemaVersion 初筛、CLI 定位和 stdin/stdout
  边界校验；不得恢复 TypeScript identity projection 或 hash mirror。
- 任何 identity fields、package dependency traversal、dedupe order 或 hash framing
  变更，都必须先更新 cross-system parity fixtures，再改变生产行为。
- 当 selector 指向未知 service/release，或没有 runtime 注册匹配的 dynamic build id
  时，路由器必须 fail closed。

## `Value` 边界

`serde_json::Value` 只允许出现在明确开放的 payload，或外部边界正在校验 raw artifact
JSON 的地方。

允许场景：

- activation 阶段传入的 runtime config **载体**（部署方注入的 resolved config blob）。
  注意这只指 opaque 载体本身：config 的 typed 字段读取（`config.require<T>` 等对
  `string` / `number` / `bool` 字段的 decode）属 boundary 解释，应走 `RuntimeTypePlan`
  /boundary contract，不在此允许范围内。参见
  `runtime-layered-crate-architecture.md` 的 `serde_json::Value` Policy。
- canonical DTO 中明确开放的 metadata/config payload。
- 反序列化为 `artifact-model` DTO 之前读取到的 raw JSON。
- 临时 identity raw-JSON 入口，但必须用 typed 入口 parity test 约束。

禁止场景：

- 用 `raw: Value` 表示 runtime-local canonical DTO 副本。
- 当上游 typed model 本应已经携带语义事实时，继续从最终 artifact JSON 里读 semantic
  fields。
- 在没有 typed-vs-raw parity test 的情况下，把 `Value` serialization 当作 typed
  canonical DTO 和 identity projection 之间唯一的桥。
- 把 `__skiffType`、representation envelope 或 artifact descriptor 当成普通
  runtime-local object metadata。JSON decode/encode 必须由 expected type descriptor
  驱动；legacy envelope 只能作为显式拒绝边界或迁移期测试 fixture 存在，不能进入
  production path。

## 审计目标

目标态达成后，以下检查应保持干净：

```bash
rg "pub struct PackageUnit|pub struct ServiceUnit|pub struct FileIrUnit" runtime/src
rg "runtime_program_build_identity_value|fn runtime_program_service_unit_identity" compiler/src runtime/src
```

预期：

- 运行时不再有本地 canonical artifact structs。
- 编译器/运行时不再有本地手写 runtime-program service-unit identity projection。
- 运行时 artifact loading 直接反序列化 `artifact_model::ServiceUnit` 和
  `artifact_model::PackageUnit`。
- 运行时 linked overlays 都以 linked/derived 类型命名。
- cross-system dynamic build id fixtures 覆盖 typed Rust、raw Rust 和 router CLI boundary
  路径。

## 最近失败为什么重要

`operation.target.executableIndex` 缺省和默认字段省略导致的验收失败，局部修复并不难，
但它们暴露的是更大的架构缺口：identity projection semantics 还没有脱离单个实现，
成为独立契约。

目标不是“再加一个 golden 就结束”。目标是：

- 共享 identity bytes 只有一个 Rust owner；
- default-field normalization 有一条被写清楚的规则；
- 每个 default-sensitive field 都有 typed/raw/router parity tests；
- canonical artifact DTO 和 runtime linked state 明确分离。
