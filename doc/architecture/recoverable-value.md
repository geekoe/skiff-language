# 可恢复值架构

本文定义 Skiff **可恢复值**的长期内部架构契约。文件名使用英文 `recoverable-value` 只是路径约定；本文主术语是
**可恢复值**。

用户可见语义后续应落到 `../reference/static-semantics.md`、`../reference/spawn.md`、`../reference/db.md`
和 `../reference/any-interface.md`；本文只规定 compiler、artifact、runtime、DB、spawn/queue payload
如何统一承载“值离开当前 request 后还能恢复”的机制。

Skiff 尚未发布。本文目标态不要求兼容旧 DB schema、旧 spawn payload、旧 `any I` boundary 禁令或旧
ToolProvider key-registry 方案。

## Scope

本文负责：

- 定义“可恢复值”这一跨 request / 持久边界的统一属性。
- 说明普通数据、nominal object、native handle、`any I`（`carrier = Local` 或正向 `carrier = Remote` public-instance
  引用）如何进入同一恢复机制；以及跨 service 反向 local callback 进恢复边界**第一版 fail-closed**，及其目标态（service
  callback transport 落地后）的 sealed 直传回拨机制。
- 规定恢复后的等价语义：类型、接口投影和可恢复状态保持一致；heap 地址不保持。
- 给出 compiler/runtime 的边界分工：静态闭包检查 + encode 时动态 carrier 查询 + owner-internal local behavior
  decode 时按当前 execution context 与稳定 restore key 恢复。
- 说明 ToolProvider 不需要 provider key registry；`any ToolProvider` 能否跨边界由其可恢复性决定（本地可恢复，正向远程
  public-instance carrier 在 owner-internal recoverable lane 可恢复，跨 service 反向 local callback 第一版 fail-closed）。

本文不负责：

- 具体 Rust 模块拆分和迁移步骤。
- 定义某个公开用户接口名，例如 `Recoverable` trait。本文的“可恢复”是语义属性，具体实现可以是 compiler fact、
  runtime codec、native adapter 或 artifact metadata。
- 保证对象 identity。恢复值不是同一个 heap object。
- owner-internal local behavior 的历史 artifact/build 读取或保留策略；这类 durable bytes 只保存稳定 `LocalConcrete`
  restore key，不依赖历史 artifact/build。native adapter 若声明 artifact-owned adapter，仍由对应 adapter owner 规则处理。

## Position

Skiff 中所有会让值离开当前 request-local heap 的边界，本质都需要同一个条件：

```text
encode(value, expected_type_plan, boundary_context) -> recoverable envelope
decode(envelope, expected_type_plan, boundary_context) -> restored value
```

这里的“可恢复”是**所有跨 request / 持久边界的值闭包要求**，不是 DB 专属规则，也不是 interface value 专属规则。
`any I` 只是最容易暴露这个要求的一类值，因为它把 concrete self、interface projection 和 method table 隐藏在 existential
wrapper 后面；普通 record、array、map、nominal object、native handle 只要进入 DB / spawn / queue / persistent payload，同样必须满足
recoverable closure。

`recoverable envelope` 是行为/动态恢复节点的统一载体。语义上所有跨 request / 持久边界的值都必须可恢复，但物理存储不要求
每个 primitive 或 plain record 字段都包一层 envelope；能由边界 schema 和 canonical bytes 直接恢复的 plain data 可以走该边界已有的
canonical codec。只有需要额外恢复身份（code identity、interface carrier、native adapter、custom restore state 等）的值，才需要显式
envelope lane。DB 在这条底线之上还叠加查询能力要求，见 §DB storage shape。

边界包括：

- DB object field / DB row。
- spawn payload。
- queue / persistent work item payload。
- 跨 request runtime binary payload。
- 普通 JSON/materialization 中被标记为可恢复 envelope 的值。
- service/public API payload 中明确允许可恢复 envelope 的位置。

后两条指的是**显式 envelope** 形态，不放开 `any-interface-value.md`“`any I` 不得编码成 ordinary JSON、不进
public API schema closure”这条禁令；ordinary（无 envelope）路径仍 fail closed。`any I` 进可恢复边界永远走显式
envelope，没有默认 wire shape。显式 envelope 还必须受信任边界约束：第一版只有 owner service 内部的
DB/spawn/queue/runtime lane 可以承载行为节点；public API、导出 materialization 或跨 service 这类离开 service trust
domain 的 envelope 第一版只允许 plain data，除非未来引入 sealed opaque payload。

本文的表述不再围绕“`any I` 是否跨 spawn”。正确表述是：

```text
所有跨 request / 持久边界的值都必须是可恢复值。
any I 只是其中一种值；判据是进入了哪类边界：
package public 入口传参（同 service / 同 runtime）= request-scope 本地值，不需要恢复；
DB/spawn/queue/persistent payload = 可恢复边界，按 carrier 与 self payload 判定；
跨 service 反向 local callback / 明文 local behavior / native adapter 节点进恢复边界 = 第一版 fail-closed（卡 service
callback transport 与 sealed payload）；
跨 service / public / 导出 materialization 的 plain data 显式 envelope 可传输，但不改变 public API schema closure 规则。
```

## Definition

一个值是可恢复值，当且仅当 runtime 能为它生成一份 envelope，并能在后续 request 中用该 envelope 恢复出等价值。

恢复后的等价要求：

- concrete type identity 一致，或满足该 type 声明的兼容恢复规则；owner-internal local behavior 用 stable
  `LocalConcrete` restore key 表达这个身份。
- 对 `any I`，当前 expected type plan 给出的 interface identity 与 method slot projection 一致。
- 可恢复 state 一致。
- 方法调用语义仍可用。
- heap 地址、native handle 地址、in-memory object identity 不要求一致。
- 第一版不保留对象图 aliasing / cycle。遇到需要保持共享引用或循环才能表达语义的 object graph，encode fail closed；可按值复制的
  acyclic tree 才进入 recoverable envelope。

不可恢复的典型值：

- 当前 request 的 stream cursor。
- HTTP/WebSocket live connection。
- file descriptor。
- DB transaction / claim lease guard。
- runtime task handle / timer handle。
- 未声明恢复语义的 native value。

如果某个 native 概念提供 durable handle 或配置，例如 `HttpClientConfig`、`credentialRef`、`endpointConfig`，
它可以作为普通 durable 配置值参与恢复（与 carrier 无关，是 plain state）。是否 native 不是判断标准；是否有恢复
语义才是判断标准。

### 当前 execution context 恢复

owner-internal local behavior 不按写入时 artifact/build 恢复。durable bytes 只保存稳定的
`LocalConcreteRestoreKey { owner, concrete_type_identity }`；decode 使用当前 request 的 execution context、当前 linked
program、当前 method table registry 和当前 expected type plan 来解释这个 key。

这不是“任意新版本都能读旧行为值”的宽松迁移。恢复时必须同时满足：

- 当前 linked program 能在当前 service context 中唯一找到 `(owner, concrete_type_identity)`。
- 找到的 concrete type 仍能按当前 expected type plan 恢复 durable state。
- 对 `any I`，当前 expected type plan 唯一给出 interface/projection；若 expected union 中多个 any-interface 分支都可匹配同一
  concrete，decode fail closed，不能按分支顺序猜测。
- durable state 与当前 restore plan 不兼容时 fail closed。这是当前 schema/类型不接受旧值，不是 artifact/build 不可用。

这条与 DB 普通字段的 schema 演化仍是两条正交的线：

- owner-internal local behavior 的恢复靠当前 execution context + stable `LocalConcrete` key；payload 不保存
  `artifact_identity`、`build_id`、service version、package version、activation identity、activation-local id、package slot、
  type table index、source hash、本地路径或 `TypeAddr`。
- DB recoverable-envelope lane 可以为 v2 历史记录使用显式 durable read policy：未知 record field 忽略，缺失 nullable field
  materialize 为 `Null`，缺失 required field 失败。默认 decode、spawn payload、runtime transient payload 仍是 strict policy。
- DB 普通数据字段的类型变更、rename、backfill 属于 **DB schema migration**（`../reference/db.md §11` 当前未定），
  与 recoverable codec 的 local behavior restore key 不是同一层。

旧 v1 recoverable-envelope bytes 和旧 local self wrapper 不进入长期兼容 contract。Skiff 尚未发布，decoder 只接受当前 v2
recoverable envelope；v1 或未知 schema/version fail closed，由本地数据清理或应用层重建处理。

### Stable LocalConcrete Restore Key

local behavior 的 durable code identity 是 `LocalConcrete`：

- `owner = Service` 表示 concrete type 来自当前 service artifact；key lookup 只能在当前 owner-internal service context 内进行。
- `owner = Package { package_id }` 表示 concrete type 来自当前 linked program 中该 package id 对应的 package unit；同一
  package id 必须唯一，0 个或多个 candidate 都 fail closed。
- `concrete_type_identity` 的 wire format 固定为 `abi-type:` + lowercase hex of `AbiTypeId::key_bytes()`。
  `AbiTypeId` 由 artifact-model 的 `abi_type_id_from_source_anchor(anchor, type_args)` 生成；worker 不得改用 JSON descriptor、
  debug string、base64、source path、runtime type shape 或局部 runtime address。`anchor` 是 source
  `SourceDeclarationAnchor` 投影到 ABI source anchor：`publication_id` 是 service id 或 package id，`abi_epoch` 是
  publication ABI owner epoch，`module_path` 和 `symbol` 来自源声明，`kind = Type`。`abi_epoch` 不是 service/package
  version、build id 或 artifact identity；改变它是显式打断 ABI identity 的 breaking 操作。
- 泛型 concrete 只有在所有 type args 都能按声明顺序递归投影成 `AbiTypeId` child key 时，才可以编码为 durable
  `LocalConcrete`；alias 按 compiler ABI 规则展开到 target。如果实现不能证明某个 type arg 有稳定 ABI type id，必须整体
  fail closed，不能把 `LinkedTypeRef::Address`、JSON descriptor、runtime type shape、package slot 或 `TypeAddr` 写进
  durable key。
- lookup key 是 `(owner, concrete_type_identity)`。当前 linked program 中同 key 多 concrete declaration、owner 与
  `AbiTypeId.publication_id` 不一致、package id 重复或无法稳定投影 generic type args，都必须 fail closed。
- `owner = Service` 时，`AbiTypeId.publication_id` 必须等于当前 service id，并且 lookup 只在当前 owner-internal service
  context 的 linked program 内进行，不能跨 service registry 或 service DB 查找。`owner = Package { package_id }` 时，
  `AbiTypeId.publication_id` 必须等于该 package id，当前 linked program 负责把 package id 解析到本次执行加载的唯一 package
  unit。
- artifact load/link 阶段需要建立 `(owner, concrete_type_identity) -> current TypeAddr / restore expected plan / method table`
  索引。索引构建时若同 key 对应多个不同 concrete declaration、不同 restore expected plan 或互不等价的 method table set，
  必须 fail closed；decode 时 0 个或多于 1 个 match 也必须 fail closed。
- `LocalConcrete` 不产生 artifact retention root。仍需要 historical artifact/adapter owner 的 `NativeAdapter` 节点继续按 native
  adapter contract 校验和保留。

## Envelope

可恢复 envelope 是边界里的稳定载体。目标态结构是“一个 envelope 包一个递归 recoverable node”。**code identity
属于每个递归节点，不是只属于顶层 envelope**；否则 record/array/nominal object 中嵌套多个行为值时，无法为每个行为值
分别提供恢复所需的 local concrete key 或 native adapter identity。

```rust
struct RecoverableEnvelope {
    schema_version: String,
    root: RecoverableNode,
}

struct RecoverableNode {
    value_kind: RecoverableValueKind,
    variant_identity: RecoverableVariantIdentity,
    code_identity: RecoverableCodeIdentity,
    state: RecoverableState,
}

enum RecoverableValueKind {
    Null,
    Bool,
    Number,
    String,
    Bytes,
    Date,
    Array,
    Map,
    Record,
    NominalObject,
    InterfaceValue,
    NativeHandle,
}
```

`variant_identity` 记录 expected type plan 下不能仅靠 payload shape 推断的分支身份：

```rust
enum RecoverableVariantIdentity {
    None,
    UnionBranch {
        union_identity: String,
        branch_identity: String,
    },
}
```

`code_identity` 不是“当前进程指针”，而是“decode 时到底要定位什么”，**不是平铺所有可能字段再用 `Option` 表达
存在与否**：

```rust
enum RecoverableCodeIdentity {
    // A. 纯数据：Null/Bool/Number/String/Bytes/Date/Array/Map/Record。
    //    结构递归即可恢复，不定位任何代码。InterfaceValue wrapper 本身也用 None；
    //    它的 concrete self 身份由 InterfaceValueState.self_node 携带。
    None,

    // B. owner-internal local concrete：NominalObject，以及 InterfaceValue.self_node 中的 concrete self。
    //    存稳定 restore key；decode 用当前 execution context 查当前 concrete restore plan。
    LocalConcrete {
        owner: LocalConcreteOwner,
        concrete_type_identity: String,
    },

    // C. durable native handle：按版本化 adapter 恢复，不把 adapter identity 重复放进 state。
    NativeAdapter {
        adapter_identity: String,
        adapter_schema_version: String,
        owner: NativeAdapterOwner,
        native_type_identity: String,
    },
}

enum LocalConcreteOwner {
    Service,
    Package {
        package_id: String,
    },
}

struct NativeAdapterPackageCoordinate {
    package_id: String,
    package_version: String,
}

enum NativeAdapterOwner {
    Builtin,
    Artifact {
        artifact_identity: String,
        build_id: String,
        package: Option<NativeAdapterPackageCoordinate>,
    },
}

enum RecoverableState {
    Null,
    Bool(bool),
    Number(/* canonical number */),
    String(String),
    Bytes(Vec<u8>),
    Date(/* canonical date */),
    Array(Vec<RecoverableNode>),
    Map(Vec<(RecoverableMapKey, RecoverableNode)>),
    Record(Vec<RecoverableField>),
    NominalObject(NominalObjectState),
    InterfaceValue(InterfaceValueState),
    NativeHandle(NativeHandleState),
}

struct RecoverableField {
    field_identity: String,
    value: RecoverableNode,
}

enum InterfaceValueState {
    Local {
        self_node: Box<RecoverableNode>,
    },
    Remote {
        carrier: RecoverableRemoteInterfaceCarrier,
    },
}

struct RecoverableRemoteInterfaceCarrier {
    dependency_ref: String,
    public_instance_key: String,
    operations: RecoverableRemoteOperationTable,
}

struct RecoverableRemoteOperationTable {
    id: String,
    interface_abi_id: String,
    slots: Vec<RecoverableRemoteOperationSlot>,
}

struct RecoverableRemoteOperationSlot {
    slot: u32,
    method_abi_id: String,
    operation_abi_id: String,
}

enum NominalObjectState {
    DefaultFields {
        fields: Vec<RecoverableField>,
    },
    Custom {
        durable_state: Box<RecoverableNode>,
    },
}

struct NativeHandleState {
    durable_state: Box<RecoverableNode>,
}

enum RecoverableMapKey {
    String(String),
    NominalRepresentation {
        representation_identity: String,
        value: Box<RecoverableMapKey>,
    },
}
```

说明：

- 原平铺草图里的 `type_identity` 收进 `LocalConcrete.concrete_type_identity`，并由 `owner` 一起构成 lookup key。
- `RecoverableNode` 是递归单位。plain data 节点通常 `code_identity = None`；需要代码恢复的本地对象节点使用
  `code_identity = LocalConcrete{...}`；durable native handle 节点使用 `code_identity = NativeAdapter{...}`。数组元素、map
  value、record 字段、nominal object 字段和 native durable state 内部的每个子值都是 `RecoverableNode`，因此嵌套行为值
  可以各自携带自己的 code identity。
- `RecoverableVariantIdentity::UnionBranch` 只在 expected type plan 是 union 且 branch identity 不能从 payload shape 唯一推断时写入。
  nullable 的 `null` / non-null 由 expected type plan 与 `value_kind = Null` 区分；若 nullable 被实现为 union lowering，仍按
  union branch identity 规则写入。
- `RecoverableMapKey` 第一版只支持 `string` 或单一名义 representation over string，和当前 boundary map key 规则一致。
  该 representation 必须是不依赖 behavior code 执行的 canonical string representation；若某种 key 规范化需要
  `LocalConcrete` behavior 或 native adapter，第一版不能作为 recoverable map key。其它 primitive key domain 若未来进入语言，需要先扩展
  这里的 canonical key 编码和 DB/index policy。
- `RecoverableState` 是按 `value_kind` 递归承载 plain data / nominal 字段 / interface self payload 的状态体。
  具体编码（结构递归 + size/depth 限制）见实现计划 `../implementation/recoverable-value-implementation.md` P2/P3。
- `NominalObjectState::DefaultFields` 表示默认结构恢复；`NominalObjectState::Custom` 表示 concrete type 自定义恢复。入口由
  `LocalConcreteRestoreKey` 在当前 linked program 中定位；runtime wrapper 不保存 `restore_schema_version`。自定义 durable
  state 仍是 `RecoverableNode`，可递归包含 plain data 或其它可恢复值。
- **`any I` 的恢复机制不在 encode 点重新判，而是取自 `any I` 自己的 `InterfaceCarrier` 分支**
  （`any-interface-value.md §Runtime Value`，`Local` / `Remote`）。carrier 在**装箱点 `as I` 就焦死**了：装箱源
  是局部 concrete 值 → `carrier = Local`（带 payload）；装箱源是已发布 public instance（如 `remoteLlm/llmInstance`）→
  `carrier = Remote`（带寻址坐标，不带 payload）。encode 点早已 type-erased、看不到装箱源，只读 carrier 已填好的
  分支。所以：
  - `carrier = Local` → `InterfaceValue` wrapper 节点 `code_identity = None`，`InterfaceValueState` 只保存
    `self_node`；interface/projection 来自 encode/decode 的 expected type plan。concrete self 的身份与状态写入
    `self_node`（通常是 `NominalObject + LocalConcrete`）。local interface 不能把 runtime carrier 或 payload 直接写进
    DB/spawn/queue；只能写入显式 envelope 中的 `Local{ self_node }`。
  - `carrier = Remote` → 是正向远程引用（consumer 主动调一个已发布 public instance）。在 owner-internal
    recoverable lane 中，它写成 `Remote{ carrier }`，只保存 `dependency_ref`、`public_instance_key` 和
    `operation table`，不保存远端 self payload；恢复时用当前 linked program 重建并校验 operation table。把本地 local
    carrier 作为跨 service 反向 callback payload 传出仍不属于这个分支，见 §Cross-Service Interface Value。
- `value_kind` 仍区分 `NominalObject` 与 `InterfaceValue`：它决定恢复出的静态形态，与 code identity 的定位职责分离。
- decode 阶段必须能用每个节点自己的 `code_identity` 定位当前 `LocalConcrete` restore plan 或 native adapter；找不到、找到
  多个候选或当前 plan 不接受 durable state 时 fail closed（见 §Definition“当前 execution context 恢复”）。
- `InterfaceValueState::Local` 不携带 interface identity 或 method projection identity。`self_node` decode 后必须得到 concrete
  nominal object；native/custom resource 若要作为 interface self，必须被某个 nominal object 或该 nominal type 的自定义恢复封装。
  decode 恢复 concrete self 后，必须用 expected type plan 提供的 interface/projection 重新校验 concrete type 仍 implements
  该 interface，并按 projection 重建 method table。projection identity 不从 source method name 临时推导，也不从 durable
  wrapper 读取。
- `InterfaceValueState::Remote` 不保存 local self。decode 先用 expected type plan 校验 operation table 的
  `interface_abi_id`，再要求当前 linked program / behavior hooks 能按 `dependency_ref + public_instance_key + persisted
  operation table` 重建等价 `RemoteOperationTable`；找不到、接口不匹配或 table 不等价都 fail closed。
- `NativeHandle` 节点的 adapter 身份只来自 `code_identity = NativeAdapter{...}`。`NativeHandleState` 只保存 adapter 的
  durable state。decode 先加载并校验 adapter，再把 durable state 交给该 adapter；adapter 缺失、schema version 不兼容或
  native type 不匹配都以 `recoverable_native_missing_adapter` / `recoverable_state_invalid` fail closed。

### DB storage shape

DB 的规则建立在 recoverable 之上，并额外要求查询/投影/索引语义。可以把 DB stored field 的能力拆成两级：

- **可存储底线**：写入 DB 的值必须可恢复；不可恢复的 request-local resource、未声明恢复语义的 native value、跨 service
  反向 local callback carrier 等，写入时 fail closed。正向 `carrier = Remote` public-instance 引用在 owner-internal
  recoverable-envelope lane 中可持久化为远程坐标与 operation table。
- **可查询能力**：只有具有稳定 schema storage shape、可比较/可排序/可投影语义的值，才能参与 nested `fields`、`where`、
  `order` 和 index。可恢复本身不自动赋予这些 DB 查询能力。

因此，“所有跨 request / 持久边界的值都必须可恢复”是语义底线，不等于 DB 每个 primitive 字段都必须物理包一层
`RecoverableEnvelope`。DB 有两种存储 lane：

- **schema-projectable lane**：`string`、`number`、`bool`、`Date`、`Bytes`、以及静态类型图内不含行为/动态恢复节点的普通
  record/array/map 等 plain data，仍按现有 DB canonical storage shape 存储。它们在语义上是可恢复值，但不需要 per-field
  envelope。
- **recoverable-envelope lane**：静态类型图可能需要携带 code identity / carrier state / durable adapter state 才能恢复的
  **顶层 stored field**，例如字段类型就是 `any I`、字段是 nominal object value（默认结构恢复或自定义恢复，都需要
  `LocalConcrete` 定位 concrete restore key）、字段内嵌 `any I` / behavior object / durable `NativeHandle`。该顶层 stored field
  整体存为显式 envelope。

这里的 lane 判定只针对 DB schema 里的 stored field。DB object / row 本身仍按 DB schema 管理，不因为“row 是 nominal
service object”而整体包 envelope；但如果某个 stored field 的静态类型就是 nominal object value，第一版按
recoverable-envelope lane 处理。普通结构化 `record` / array / map 若不含 nominal behavior、`any I`、custom restore 或
native adapter，仍属于 schema-projectable lane。

第一版不做“schema-projectable record 里局部子节点包 envelope、父 record 仍可被 DB projection/index 穿透”的混合形态。
lane 是 DB schema / compiler 的静态属性，不允许同一字段逐行在 projectable storage 与 envelope storage 之间切换。一旦某个
top-level DB stored field 的静态类型图可能需要 recoverable-envelope lane，该字段整体变成 opaque envelope。
例如 `settings: { provider: any ToolProvider, label: string }` 中只有 `provider` 需要 code identity，但 DB 中 `settings` 整体是
recoverable-envelope 字段；`fields { settings.label }`、`where settings.label == ...` 和对 `settings.label` 建索引第一版都不支持。
反过来，schema-projectable lane 写入运行时才出现的 behavior/dynamic value 时，不改变 storage shape，必须 fail closed。

DB projection 和 index policy 只看 DB 可投影的 storage shape：

- schema-projectable lane 可以按现有规则做 nested projection、predicate、order 和 index。
- recoverable-envelope lane 是一个不可穿透的字段值；可以 full field 读写，但第一版不支持 nested projection、predicate、order
  或 index。未来若要让某个 concrete adapter 声明稳定 index projection，必须另行定义，不属于第一版。
- DB full read decode envelope 失败时，不构造半对象；projection 没选中该字段时不触发它的 decode。

DB recoverable-envelope read 可使用 durable DB policy：已选中的 envelope 内部 record 多出的历史字段被忽略，缺失 nullable
字段 materialize 为 `Null`，缺失 required 字段仍失败。这个 policy 只作用于正在 decode 的 v2 envelope 及其递归
`LocalConcrete` self durable state；projection 没选中某个 top-level envelope 字段时，不触发 decode，也不 materialize 字段。
写入、spawn payload 和 runtime transient payload 默认仍使用 strict policy。

## Decode Against Expected Type

recoverable decode 也不能只拿 envelope。它必须带当前边界的 expected type/schema plan：

```rust
decode(envelope, expected_type_plan, boundary_context) -> restored value
```

decode 顺序：

1. canonical decoder 先校验 envelope schema/version。当前长期 contract 只接受 v2；v1 或未知 schema/version fail closed。
2. `trust_boundary != OwnerInternal` 时，先扫描并拒绝明文 behavior-bearing node；不能为了 expected type 预检而先加载本地
   behavior。
3. 按 `expected_type_plan` 和 decode policy 做 envelope-level 预检：校验 `RecoverableNode.value_kind`、`variant_identity`、
   plain data shape、map key domain、nullable null/non-null shape 和 union branch identity 是否兼容。record unknown/missing
   field 行为由当前 decode policy 决定。
4. 对 `value_kind = InterfaceValue` 节点，expected type 必须唯一解析到一个 `any I` expected plan。单一 expected `any I`
   直接提供 interface/projection；expected union 对 `Local` carrier 必须用 `self_node` 的 `LocalConcreteRestoreKey` 对当前
   linked program 做 conformance 检查，对 `Remote` carrier 必须用当前 linked program 按 persisted carrier 重建等价
   remote operation table，并唯一选中一个 any-interface 分支。没有分支或多个分支可匹配都 fail closed。
5. `InterfaceValueState::Local` 只提供 `self_node`。`self_node` 必须是 `NominalObject + LocalConcrete`；decode 用当前 linked
   program 按 `(owner, concrete_type_identity)` 查找 concrete restore plan，用当前 decode policy 递归恢复 durable state，
   再校验 concrete type 仍 implements expected interface/projection 并重建 method table。
6. `InterfaceValueState::Remote` 只提供正向 public-instance carrier：`dependency_ref`、`public_instance_key` 和持久化
   operation table。decode 用 expected type plan 校验 interface identity，并要求当前 linked program 能为同一 dependency /
   public instance 重建等价 operation table；否则 fail closed。它不读取 local self，也不产生 artifact retention root。
7. 对 `LocalConcrete` 节点，按当前 execution context lookup restore plan，再校验恢复出的 concrete type 可以赋给当前
   expected type。local behavior decode 不读取 durable interface/projection，也不读取 artifact/build。
8. 对 nominal object，先按当前 concrete restore plan 和 durable state 恢复等价值，再按当前 expected nominal/interface/union
   plan 做可赋值检查。当前 schema 不接受该 concrete key 或 durable state 时 fail closed；这不是自动 schema migration。
9. 对 DB 普通 schema-projectable 字段的跨版本变化，仍由 DB schema migration 负责；recoverable decode 只负责“当前 expected
   type 能否接收这个 envelope”。

不兼容必须以稳定错误 fail closed，不得把旧类型值交给当前代码后再在任意调用点失败。

### Compatibility Contract

第一版 recoverable decode 的兼容性采用保守矩阵。没有列在这里的兼容都不存在，不能靠名字相同、字段顺序相近或 runtime
duck typing 放行。

- `LocalConcrete.concrete_type_identity` 是 stable restore key，不是 runtime address。唯一 wire format 是 `abi-type:` +
  lowercase hex of `AbiTypeId::key_bytes()`；它不得包含 artifact/build/version/activation/source path/package slot/type table
  index/`TypeAddr`。
- plain data：`value_kind`、nullable shape、primitive canonical domain 必须与 expected type plan 匹配；不做 string/number 等
  宽松 coercion。
- record/default nominal fields：field identity 必须逐项匹配当前 concrete restore plan；字段 rename、删除、拆分、合并属于
  DB schema migration 或 concrete custom restore，不由 recoverable decode 自动猜测。
- nominal expected type：恢复出的 `LocalConcrete.concrete_type_identity` 必须被当前 expected nominal identity 接受；若 expected type 是
  interface，则按 interface 规则检查；若 expected type 是 union，则先按 union branch 规则选中分支。
- interface expected type：interface identity 和 projection identity 来自当前 expected type plan；local
  `InterfaceValueState` 不保存这两项。恢复出的 concrete type 必须在当前 linked program 中实现该 interface/projection。
  remote `InterfaceValueState` 必须能在当前 linked program 中按 persisted carrier 重建等价 remote operation table。否则
  `recoverable_interface_conformance_missing` 或 remote carrier rebuild 失败。
- union expected type：若 envelope 带 `UnionBranch{ union_identity, branch_identity }`，两者必须与当前 expected union 和 branch
  identity 精确匹配。没有 branch identity 时，只允许 compiler 证明 payload shape 在当前 union 中唯一；对 any-interface union，
  当前 `LocalConcrete` 若能匹配多个 any-interface 分支，必须 `recoverable_expected_type_mismatch` fail closed。
- custom restore：runtime wrapper 不保存 `restore_schema_version`。custom restore plan 属于当前 `LocalConcrete` restore entry；
  durable state 是否可读由当前 plan 和 decode policy 决定。应用级迁移必须由 concrete type 或 DB schema migration 显式定义。
- native adapter：`adapter_identity`、`adapter_schema_version`、`native_type_identity` 必须被当前可加载 adapter plan 接受。
  第一版 adapter schema version 默认精确匹配；显式版本兼容必须由 adapter metadata 声明。

错误优先级：

1. `trust_boundary != OwnerInternal` 的明文行为节点先失败，不做 local concrete lookup。
2. envelope schema/value kind/variant/shape 不匹配报 `recoverable_expected_type_mismatch` 或 `recoverable_state_invalid`。
3. local concrete 缺失/歧义或 native adapter 缺失报对应 stable recoverable code。
4. 当前 linked program 中 interface/projection/impl 不成立报 `recoverable_interface_conformance_missing`。

## Plain Data

普通数据按结构递归恢复：

- `Null` / `Bool` / `Number` / `String` / `Bytes` / `Date` 直接编码。
- `Array<T>` 要求所有元素可恢复。
- `Map<K,V>` 要求 key/value 都满足该边界的 key/value 规则；map key 仍不能需要 object identity。
- record 要求字段可恢复，并保留足够 schema/type 信息让接收方无歧义恢复。

这部分不是新能力，而是现有 boundary codec 的统一命名。

## Nominal Object

nominal object 可以自动结构恢复，也可以声明自定义恢复。

默认规则：

- concrete type identity 进入当前 `RecoverableNode.code_identity`（需要代码恢复时为 `LocalConcrete.concrete_type_identity`）。
- 字段按声明顺序或 canonical field order 编码。
- 每个字段必须可恢复。
- decode 时用同一 concrete type identity 做 restore-mode allocation，填入恢复后的字段，再执行 compiler/runtime 已知的
  representation/invariant 校验；默认结构恢复不调用 source-level constructor、不执行外部副作用。

自定义规则：

- 类型可以声明某些 runtime-only 字段不进入 state。
- 类型可以用 durable state 替代 runtime state，例如用 `connectionId` / `credentialRef` 替代 live connection。
- 类型可以拒绝恢复，即使字段看起来可编码。
- 类型需要非字段重建逻辑时，必须声明自定义恢复；自定义恢复仍不得在 decode 阶段执行不可回滚的外部副作用。

自定义规则属于 concrete type，不属于 interface 使用方。使用方只能在 encode 时得到“可恢复/不可恢复”的结果。

### Custom Restore ABI

自定义恢复第一版只定义内部 artifact ABI，不定义公开语法。compiler 可以先用内部 annotation / metadata 产出同一结构。

当前 linked program 的 restore metadata 必须为每个支持 custom restore 的 concrete type 写入：

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

约束：

- encode hook 输入当前 object self 和 `RestoreCapability::PureRecoverableRestore`，输出 `durable_state_type_plan` 下的
  recoverable value；runtime-only raw fields 不被 recoverable closure 遍历，除非 hook 自己把它们投影成 durable state。
- encode/decode hook 在该 capability 下都不允许网络、DB write、spawn、文件写入、外部 clock/random 等不可回滚副作用；
  第一版以 runtime capability guard 为准，lint 只能作为辅助，违反时 fail closed。
- hook 由当前 `LocalConcreteRestoreKey` 定位。找不到 plan、durable state 不符合当前 plan、hook 输出对象 concrete type
  不匹配时 fail closed。
- runtime wrapper 不保存 `restore_schema_version`。若 concrete type 需要应用级状态迁移，必须由该 concrete type 或 DB schema
  migration 显式定义，不能复用 recoverable wrapper 版本字符串。

### Native Adapter ABI

durable native handle 也只定义内部 ABI。native adapter 必须在 builtin registry 或 artifact metadata 中注册：

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

约束：

- encode 只允许 adapter 把 native handle 投影成 durable state；没有 adapter plan 的 native/request-local value fail closed。
- decode 先按 `code_identity = NativeAdapter{...}` 查找 builtin 或 artifact-owned adapter，校验 adapter identity、
  native type identity 和 schema version；不兼容则 fail closed。
- adapter decode 与 custom restore 使用同一 restore capability 限制，不得执行不可回滚外部副作用。
- adapter durable state 自身仍是 `RecoverableNode`，可递归包含 plain data 或其它可恢复值，但必须遵守当前 boundary context。

## any I

`any I` 不特殊。它的静态类型只说明“当前值实现了 I”，不说明它是否可恢复。是否可恢复、走哪条路径，取自它的
`InterfaceCarrier` 分支（`any-interface-value.md §Runtime Value`，装箱点 `as I` 已焦死）。envelope 字段以 §Envelope
为唯一真相源，本节只描述动作流。

**`carrier = Local`**（装箱源是本进程 concrete 值）→ 走 §Envelope 的 `InterfaceValueState`，concrete self 进
`self_node`：

- encode：`value_kind = InterfaceValue`；`code_identity = None`；
  `state = InterfaceValueState{ self_node = recover(payload) }`。interface/projection 来自当前 expected type plan；若 expected
  type 是 union 且多个 any-interface 分支都可匹配同一 local concrete，encode fail closed。`self_node` 是 concrete self 的
  唯一身份源，通常是 `value_kind = NominalObject` 且 `code_identity = LocalConcrete{ owner, concrete_type_identity }`。
  `recover(payload)` 若 self 闭包含不可恢复物（live connection / stream / transaction /
  fd / 无 adapter 的 native，见 §Definition），**encode 即 fail-closed**（用 `recoverable_*` 码），按 §Nominal Object
  “使用方只能在 encode 时得到可恢复/不可恢复结果”。绝不把重建不出的字节交出去。
- decode：从 expected type plan 唯一取得 interface/projection，先 decode `self_node` 得到 concrete nominal object → 校验它仍
  implements I / projection → 重建 method table → 返回 `any I`。失败 fail closed（见 §Definition“当前 execution context 恢复”）。

**`carrier = Remote`**（装箱源是已发布 public instance，如 `remoteLlm/llmInstance`）→ 是 consumer **主动调**一个远程
公开实例的正向引用。owner-internal DB/spawn/queue/persistent payload 或显式 recoverable envelope slot 可以持久化它，
但只保存：

- `dependency_ref`：当前 service dependency 指向的远端 service/public contract。
- `public_instance_key`：远端已发布 public instance 的稳定 key。
- `operation table`：当前 linked program 下用于 dispatch 的 remote operation table。

decode 必须用当前 expected type plan 校验 interface identity，并要求当前 linked program 能按同一 dependency/public instance
重建等价 operation table；dependency 不再存在、public instance 不再发布、operation table 不等价或 interface 不匹配时 fail
closed。该路径不调用 local encode/restore hook，不保存 remote self payload，也不新增 artifact retention root。

同 service 内（跨 package 同 runtime）的 `any I` 在 package public 入口之间传参时是 request-scope 本地值
（`any-interface-value.md §Boundary Contract`），这类同 request / 同 runtime 的流动不需要 envelope。若同一个值进入
DB/spawn/queue/persistent payload，则仍按本文的可恢复边界处理：`carrier = Local` 且 self payload 全可恢复时允许；
`carrier = Remote` 为正向 public-instance carrier 且当前 linked program 可重建 operation table 时允许；self 不可恢复或
local carrier 被当作跨 service 反向 callback payload 时 fail closed。

## Cross-Service Interface Value

唯一的边界是 **service**（service 内部不跨进程，没有“跨进程”这回事）。`any I` 跨 service 有两种**方向**，第一版处境
不同——关键看“**之后由谁、朝哪个方向调它的方法**”：

- **正向：consumer 主动调远程公开实例。** `remoteLlm/llmInstance as I`——装箱出 `carrier = Remote`，consumer 持有它、
  **主动** consumer→callee 调用，复用现状 service dependency dispatch。这是 request-scope 引用，consumer **本地主动调**
  时不需要 envelope。若它进入 owner-internal DB/spawn/queue/persistent payload，则按上节的
  `Remote{ dependency_ref, public_instance_key, operation table }` 形式持久化并恢复。这个能力已经落地，恢复时校验的是当前
  linked program 是否仍能解释该正向 public-instance carrier。
- **反向：值被传去对端，对端之后回拨构造侧。** consumer 把一个 `any I`（装箱源是本进程**局部** concrete 值）作为
  payload 传给对端 service，对端之后调它的方法 = 反向打回构造侧。**这才是恢复机制要管的**——值离开了构造侧 request，
  对端要据可恢复信息回拨。第一版仍 fail closed；未来需要 sealed local value 与 service callback transport。

反向恢复信息**直传、不落 DB**。直传的是对端不可解释的 opaque sealed payload，而不是明文 state：

```text
构造侧                                          对端 service
const i = localImpl as I  ──sealed 可恢复字节随 wire 传──►   持有 opaque 字节（把对端当存储）
                                                       │ 回拨时把字节带回构造侧
  ◄──────回拨：带回原字节──────────────────────────────┘
  按 sealed payload 内各节点的 code identity + state 重建等价 carrier，执行
```

关键性质：

- **构造侧无状态、无 GC**：encode 完即撒手，不留待回拨记录。字节由对端持有，回拨时随请求带回。既满足“构造侧不长期
  持有内存活实例”，又不引入构造侧持久句柄的生命周期/GC 负担。
- **carrier 可执行性靠构造侧 service 级恢复上下文**，不靠某个活实例：回拨到达时按字节里的 stable local concrete key
  与 native adapter identity，在构造侧当前 execution context 中重建。故任意一个可接受该 key/state 的构造侧 service
  实例都能处理回拨，构造侧重启、扩缩容不影响——这正是 §Definition“heap 地址不保持、按 envelope 恢复等价值”的直接应用。
- **字节可重建性由 encode 侧负责**：self payload 必须在构造侧 encode 时就全可恢复，含不可恢复物当场 fail（见上面
  §any I encode）。对端只负责存取字节、不负责发现字节坏。否则坏字节跨 service、跨时间后回拨才炸，无处归因。
- DB 留 id 是直传的退化变体：把同一份字节落构造侧 DB、对端只拿 id，**落库字节与直传字节内容相同**，但要求构造侧
  持久持有一行 + 管 GC。直传把存储推给对端、构造侧零持有，更贴合本文恢复模型。DB 留 id 仅作为“对端不便携带大
  payload”时的可选优化，不是主路径。
- **跨 service 直传 payload 必须 sealed**：payload 由构造侧生成并验证，至少绑定构造侧 service id、整个 recoverable tree
  中所有节点的 code identity（LocalConcrete stable key / native adapter identity 等）、expected interface、目标 service id（或明确的 audience）、
  schema version 和过期/重放策略。对端只能保存和回传，不能读取、篡改或自行构造。未实现 sealed envelope 前，跨 service
  直传保持 fail-closed；不能退化为“把明文 recoverable state 交给对端”。
- “构造侧无状态”与 replay protection 的组合只允许两种实现：无状态可验证策略（例如短 TTL + audience/nonce/单调时间窗口）
  或平台级 replay 状态。不能为了 replay 防护退回到应用级 callback handle / 活实例注册表。

**第一版跨 service 行为值 fail-closed，卡点是 service callback transport 与 sealed payload。** 反向回拨需要 callee→构造侧的**反向入站通道**，而
现状 outbound dispatch 单向（runtime 主动连 router，业务流量只 consumer→callee）。该通道落地前，跨 service `any I`
进恢复边界在 encode 时以稳定 code 失败（`cross_service_interface_callback_unavailable`）。直传**不**需要“运行期实例级
句柄 + 注册表/GC”（构造侧无状态根本不需要它——见 `any-interface-value.md §Evolution` 的否定注）；真正必备的是反向通道
加 sealed payload，而不是实例级注册表。

### 坐标方案与“为什么第一版不区分顶层/局部”

直觉上，装箱源若是**顶层符号单例**（不可复制），似乎应该“传坐标、不带 self”，而非直传字节——单例不该被复制重建。
这个直觉对，但第一版落不了地，原因在寻址层而非语义：

- **坐标 = `(service, 版本, 内部 root 路径)`**，概念上不依赖“发布”这个仪式——一个顶层符号天然有内部路径。
- 但 skiff 第一版的**跨 service 寻址单元只有 `api.yml` 显式发布的 public instance**（`publication.md`：“public
  instance 只能来自 api.yml 显式公开的 top-level const + interfaces leaf；普通 public const 不自动成为 receiver
  root”，跨 service 调用按 `operation_abi_id` 寻址、只对已发布 public instance method 存在，且“**未进 public API
  graph 的 symbol 不进 service remote contract**”）。**未发布的内部 root 路径，跨 service 寻址层第一版根本不在
  contract 里、寻址不到。**

于是顶层符号落在两种情形，**都不构成“顶层符号坐标进恢复机制且第一版可做”**：

1. 顶层符号**已发布成 public instance** → 它是正向 `Remote` carrier（consumer 主动调），request-scope、不进恢复机制。
2. 顶层符号是**私有 const**（未发布）→ 没有 `operation_abi_id`，跨 service 寻址层不认，第一版无法被回拨寻址。

加上“反向回拨无论坐标还是直传都卡 callback transport，且直传还必须 sealed”，结论收敛为：**跨 service `any I`
进恢复边界，第一版一律 fail-closed，不区分顶层/局部。** 坐标方案（含未发布内部 root 路径寻址）是正确的演进方向，但它依赖
尚未建的能力：**反向 callback transport**、**跨 service 按未发布内部 root 路径寻址**；若走直传，还必须有
**sealed opaque payload**。这些落地后，顶层符号才可走坐标、免于直传复制单例。这属演进，不在第一版。

字节结构和构造侧按字节重建的内部 codec 可以复用 owner-internal `LocalConcrete` 恢复路径；但跨 service encode 第一版不开放。缺的不只是
state 结构，还有“对端真的发起回拨”那条反向通道，以及对端只能保存/回传、不能读取/篡改的 sealed payload。recoverable codec
能定义本地恢复字节，不能序列化出尚不存在的反向通道、寻址能力和 sealed transport，故这些能力落地前跨 service 行为值
统一 fail closed。`spawn` 现状不支持跨 service callable / callback（`../reference/spawn.md`）——若回拨要在 callee spawn
的 worker 里发起，另受 spawn 自身限制约束，那是 spawn 的范围。

## Recoverable Boundary Context

recoverable encode/decode 不能只拿一个 value 和 type plan。它必须带边界上下文，供 runtime 区分同 service 持久边界、
跨 service 显式 envelope slot、普通正向 remote call 和 package 同 runtime 传参。

目标态上下文：

```rust
struct RecoverableBoundaryContext {
    boundary_kind: RecoverableBoundaryKind,
    origin_service: ServiceIdentity,
    target_service: Option<ServiceIdentity>,
    trust_boundary: RecoverableTrustBoundary,
    explicit_recoverable_slot: bool,
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
    // owner service 内部、平台生成且不接受外部伪造字节的 lane。
    OwnerInternal,
    // service A -> service B，或未来 B 持有 A 生成的 sealed payload 后回拨 A。
    CrossService,
    // public API client、导出的 materialized artifact、或任何外部可构造/篡改 envelope 的来源。
    ExternalUntrusted,
}
```

规则：

- `DbPayload` / `SpawnPayload` / `QueuePayload` 是 owner service 内部的跨 request / 持久边界；`target_service = None`。
  `trust_boundary = OwnerInternal`。`carrier = Local` 且 self payload 全可恢复时允许；正向 `carrier = Remote`
  public-instance 引用可按 dependency/publicInstance/operation table 持久化并恢复。`explicit_recoverable_slot` 对这类
  owner-internal lane 不是 public ABI 开关；DB schema / spawn target / queue payload plan 本身就是可恢复边界。
- package public 入口传参不调用 recoverable codec；它是同 runtime request-scope 值传递。
- service/public API 的 ordinary schema payload 不允许 `any I` 默认 wire shape，也不允许隐式 recoverable envelope。
- service/public API 只有在 schema/ABI 明确标记 `explicit_recoverable_slot = true` 的位置才能调用 recoverable codec。
- 第一版若 `trust_boundary != OwnerInternal` 且 recoverable tree 中出现 `value_kind = InterfaceValue`、`NominalObject`、
  `NativeHandle`，或任何 `LocalConcrete` / `NativeAdapter` 节点，encode 必须 fail closed；decode 收到这类明文行为 envelope
  或畸形行为节点也必须以稳定 code fail closed，除非未来 sealed opaque payload 生效且校验通过。`CrossService` 下
  `InterfaceValue` local carrier 的失败码是
  `cross_service_interface_callback_unavailable`；普通 `LocalConcrete` nominal object 或 `NativeAdapter` 节点的失败码是
  `cross_service_recoverable_behavior_unavailable`。`ExternalUntrusted` 下行为节点的失败码是
  `recoverable_untrusted_behavior_payload`。`carrier = Remote` 只有 owner-internal 正向 public-instance carrier 可恢复；作为
  cross-service 反向 callback payload 或不可信明文行为 envelope 时仍 fail closed。
  plain data 显式 envelope 可作为普通数据 envelope 传输，但不改变 public API schema closure 规则。
- 目标态跨 service 行为值解除 fail-closed 前，必须同时具备 sealed opaque payload 与 service callback transport；否则不得把
  `LocalConcrete` state 作为明文发给对端 service。
- `PublicApiPayload` 默认 `trust_boundary = ExternalUntrusted`。第一版即使有 `explicit_recoverable_slot = true`，也只允许
  plain data envelope；行为节点、native adapter 和 `InterfaceValue` 不得从 public API 明文解码。未来若要开放，必须使用
  sealed opaque payload，并校验 issuer service、audience、expected type/interface、整个 recoverable tree 的 code identity、
  schema version、expiry/replay policy。
- `RuntimeWirePayload` 是 runtime 内部跨 request payload，按 owner service 内部边界处理；若带 `target_service`，适用上面的
  cross-service fail-closed 规则。
- `Materialization` 只允许显式 envelope 形态；ordinary JSON/materialization 仍不允许 `any I` 默认 wire shape。第一版若需要
  导出到 service trust domain 外，`trust_boundary = ExternalUntrusted`，只允许 plain data envelope；owner-internal
  materialization 必须由平台标记 `trust_boundary = OwnerInternal` 才能承载行为节点。

## Dynamic Recoverability Query

`any I` 的 carrier（`Local` / `Remote`）在装箱点 `as I` 已定死，但 `carrier = Local` 的 **self payload 是否全可恢复**
擦除后只有运行时才知。故采用两层检查：

1. 静态 type closure：排除明显不可恢复的位置和类型（function callback、stream、transaction handle）。carrier 类别
   不在此判——它已随 `any I` 值携带（见 §any I）。
2. 动态 self query：对 `carrier = Local` 的行为值，encode 实际值时询问其 self payload 是否全可恢复，取得 envelope；
   不可恢复则当场 fail。

动态查询失败时，边界操作失败：

- DB write 不写半截 row。
- spawn submit 不提交 work item。
- queue enqueue 不提交 work item。
- service payload encode 不发送请求。

错误必须带稳定 code。完整错误码清单是实现细节，以实现计划
`../implementation/recoverable-value-implementation.md` 为真相源；本文只约束错误必须能区分 code identity 缺失、
local concrete key 缺失或歧义、native adapter 缺失、interface conformance 失效、remote carrier 不可持久化、cross-service
callback 缺失、cross-service behavior transport 缺失、不可信明文行为 envelope、sealed payload 校验失败等故障类别。

## Boundary Policy

所有跨 request / 持久边界共享“值必须可恢复”这条底线。不同边界仍可叠加自己的 public contract 规则：

- DB 可以要求 schema/index 可投影。
- service public API 可以要求 public schema 可描述。
- spawn 可以要求 target function 参数可恢复且 target 返回 `void/null`。
- queue 可以要求 work item payload 可恢复且大小受限。

这些是额外 policy，而非 interface value 特例。DB 拒绝某个值，可能是因为 DB schema/index policy，不是因为它本质
不能恢复。

## ToolProvider Consequence

ToolProvider 不需要通用的“provider key 映射到顶层 const”registry。

目标态 agent runtime bindings 可以直接是：

```skiff
type AgentRuntimeBindings {
  llm: any LlmClient,
  events: any AgentEventReceiver,
  providers: Array<any ToolProvider>,
}
```

`any ToolProvider` 是否能写入 thread config、run row（以及仅审计用的 model turn metadata），由具体 provider 的装箱源决定：
同 service 内 provider 走 owner-internal recoverable envelope（`carrier = Local`）；正向引用的远程公开实例 provider（`carrier = Remote`）
在 owner-internal recoverable lane 中以 dependency/publicInstance/operation table 持久化；跨 service 反向 local callback
provider 第一版 fail-closed（见 §Cross-Service Interface Value）。`HostProvider` 怎么恢复是 `HostProvider` 的责任，不是
agent 包的责任。

spawn payload 不保存 provider array。agent drain 类后台任务只能传 `threadId` / `runId` 等稳定 id；worker 进入新 request 后从
  `AgentRun.runtimeBindings` 读取 run 生命周期冻结的 provider array。这样 spawn payload 只是唤醒信号，不成为第二份 runtime
  binding source。所有写入 `AgentRun.runtimeBindings` 的 `llm` / `events` / `providers` interface 值都必须在 owner-internal
  recoverable boundary 中可恢复；正向 `carrier = Remote` public-instance carrier 可恢复，不可恢复 local self 或跨 service
  反向 callback local carrier 在创建 run/config 时稳定拒绝。

`ToolProviderBinding { key, provider }` 不应作为恢复机制。若仍需要 key，只能是 provider 自己暴露的诊断或业务
stable id，不是 agent 包找回 provider 的通用载体。

snapshot 路由的命题（本文只规定这一条，不冻结 agent 包 snapshot schema——具体字段属
`../implementation/recoverable-value-implementation.md` P7 / agent 包）：`AgentRun.runtimeBindings.providers`
保存 run 生命周期冻结的 provider array，并且是 dispatch 的唯一 provider source。snapshot entry 只保存
`providerIndex`、不重复保存 provider；dispatch 时从 `AgentRun.runtimeBindings.providers[providerIndex]` 取。若
`AgentModelTurn` 需要记录 runtime binding 信息，只能保存 digest / audit metadata 或不可用于 dispatch 的副本，不能成为
第二个 provider array source。这样 snapshot 只描述 model turn 的工具路由，runtime binding 仍由 run 生命周期冻结。

## Fixed Constraints

本文采用以下约束：

- 主术语是“可恢复值”；英文仅用于文件名和 Rust-ish 类型草图。
- DB/spawn/queue/persistent payload 使用“值必须可恢复”的统一底线。
- `any I` 是否可恢复、走哪条路径，取自其 `InterfaceCarrier` 分支（装箱点 `as I` 已定，recoverable 不重判）：
  `carrier = Local` 行为值走 `InterfaceValueState + self_node`（self 节点携带 `LocalConcrete` stable restore key）；
  `carrier = Remote`（正向引用已发布远程公开实例）在 owner-internal recoverable lane 以
  dependencyRef/publicInstanceKey/operation table 持久化，并在恢复时校验当前 linked program。跨 package（同 service 内）
  `any I` 在 package public 入口传参时是 request-scope 本地值；若进入 DB/spawn/queue/persistent payload，则仍按本文可恢复边界处理。
  **跨 service 反向 local callback / sealed local value 第一版 fail-closed**（卡 `any-interface-value.md §Evolution` 的
  service callback transport 与 sealed payload）；坐标方案（含未发布内部 root 路径寻址）是演进方向、不在第一版。见
  §Cross-Service Interface Value。
- 普通结构化值默认递归恢复；native/request-local 值默认拒绝；concrete type 可以提供自定义恢复或拒绝恢复。
- owner-internal local behavior envelope 必须携带 stable `LocalConcrete` restore key；decode 在当前 execution context 中找不到唯一
  concrete restore entry 时 fail closed。
- local behavior payload 不携带 artifact/build/version/activation identity；`NativeAdapter` 若使用 artifact-owned adapter，仍按
  adapter owner 的 artifact availability/retention contract 处理。
- DB 普通 schema-projectable 字段保持现有 storage shape；需要 code/carrier/adapter state 的字段整体使用显式 envelope，
  且第一版作为不可穿透字段处理。
- 离开 owner service trust domain 的 envelope 第一版只允许 plain data；跨 service 直传 payload 和未来 public/materialization
  行为 envelope 的目标态必须是 sealed opaque payload。sealed 机制和 callback transport 均未落地前，跨 service `any I`
  进恢复边界保持 fail-closed。
- ToolProvider 不使用 provider key registry 作为通用恢复机制；agent 保存/传递 `any ToolProvider`。
- Tool snapshot 不重复保存 provider array；snapshot entry 保存 `providerIndex`，dispatch 从
  `AgentRun.runtimeBindings.providers` 取对应 provider。
- spawn 不携带 provider array；后台 worker 通过 `runId` 读取 `AgentRun.runtimeBindings`，避免 providerIndex 出现第二个解释源。

## Compatibility With Existing Docs

本文会取代这些旧表述：

- `any-interface.md` / `any-interface-value.md` 中把同 service 内 `any I` 行为值排除在 DB/spawn/persistent payload
  之外的旧绝对规则已被取代；同 service 内行为值（`carrier = Local`）现在走可恢复机制。注意：
  `any-interface-value.md §Runtime Value / §Evolution` 关于跨 service `any I` durable 句柄未提供的定位**部分被取代**：
  本文把它落为“跨 service 进恢复边界第一版统一 fail-closed、待 service callback transport 落地后解除”，并同步否定了原
  “运行期实例级句柄+注册表”这块基建（见 §Cross-Service Interface Value 与 `any-interface-value.md §Evolution` 否定注）。
- `spawn.md` 的旧参数编码判据收敛为“参数必须可恢复”（由实现计划 P0 承接）。
- ToolProvider 实现文档中用 key 代替 interface 值、并在 spawned worker 侧通过 registry 重装箱的通用结论。

保留的正确部分：

- request-local native resource 不能跨边界，除非提供 durable restore state。
- package public entry 的 `any I` 仍是同 service 内值传递，不需要 envelope。
- 跨 service `any I` 的 callback transport 基建依赖、以及正向 `Remote` carrier（已发布公开实例）的 request-scope
  定位，仍以 `any-interface-value.md §Runtime Value / §Evolution` 为准。
- spawn 仍是后台唤醒，不是业务可靠层；业务事实仍需先落 DB。
