# Recoverable 当前 artifact 恢复实现计划

本文是实现计划，不是长期架构契约。对应长期契约需要在 B-F 实现合入前同步修订
`doc/architecture/recoverable-value.md`、`doc/reference/spawn.md` 和
`doc/reference/any-interface-value.md` 中“按写入时 artifact/build 恢复”的旧表述。

## 背景

当前 owner-internal recoverable behavior payload 把本地实现身份写进 durable
bytes。典型路径是 `carrier = Local` 的 `any I`：

- `runtime/eval/src/recoverable_behavior.rs` 写入 `self_node.code_identity =
  LocalCode { artifact_identity, build_id, concrete_type_identity, package }`。
- 同一个 `self_node` 使用 `NominalObjectState::Custom { restore_schema_version,
  durable_state }`，其中 `restore_schema_version` 当前是固定字符串
  `skiff.runtime.interfaceSelf.v1`。
- `restore_local_interface_self` 在恢复时要求 stored `artifact_identity` 和
  `build_id` 与当前 request hook 完全相等，否则报
  `recoverable_artifact_unavailable`。
- DB 写入时还会递归收集 artifact refs，并通过
  `CurrentRequestRecoverableArtifactStore` 校验“只能加载当前 request artifact/build”。

这导致稳定 service DB 中的 recoverable field 被旧 build 写入后，新 build 即使
schema 和 concrete 类型完全兼容，也没有机会进入应用或 schema 兼容逻辑。错误会在
DB field decode 阶段提前失败。

`spawn` 的情况不同。`spawn` 的 service version、build id、activation identity 是发给
router/worker 的控制面路由信息，默认队列也是本机内存态；它们可以存在于 submit/queue/claim
元数据中，但不属于 recoverable args payload，更不应写进每个 local behavior 节点。

## 目标

1. owner-internal local behavior 的恢复使用**当前执行上下文 artifact/build**，而不是
   durable payload 中保存的 artifact/build。
2. durable local behavior payload 中不再写 `artifact_identity`、`build_id`、service
   version、package version、activation identity、activation-local id 或其它版本/构建身份作为恢复 hard gate。
3. typed recoverable boundary 的 expected type plan 提供 interface/projection；
   `InterfaceValueState` 不再把 interface/projection 当 durable truth 重复保存。若 expected
   type 是 union，必须唯一解析到一个 any-interface 分支；多分支可匹配时 fail closed。
4. local behavior self 只保存恢复当前值必需的稳定 concrete restore key 和 durable
   state。
5. 移除 local interface self wrapper 的 `restore_schema_version`。未来如需应用级迁移，
   必须由 concrete type 或 DB schema migration 显式定义，不能复用 runtime wrapper
   版本字符串。
6. DB recoverable-envelope lane 采用 durable decode 策略：对本方案上线后写入的 v2
   recoverable-envelope 记录，artifact/build mismatch 不再早于 schema 检查失败；不同
   build 写入的 v2 记录可以进入当前 expected type 的兼容判断。
7. `spawn` 保持 strict same-build 语义。service version/build id 只作为控制面路由和队列
   claim 信息存在，不进入 recoverable payload。

## 非目标

- 不开放 cross-service 或 external-untrusted behavior envelope。非 owner-internal 边界
  仍然拒绝 `InterfaceValue`、`NominalObject`、`LocalCode`、`NativeAdapter` 等
  behavior-bearing node。
- 不实现完整 DB schema migration、backfill、dual-write 或 read-repair 工作流。
- 不兼容旧 recoverable v1 bytes。Skiff 尚未发布，本地 dev/stable DB 里已由旧 build
  写下的 recoverable-envelope 字段可以删除或重建。
- 不让 `carrier = Remote` 的 `any I` 变成 durable value。
- 不设计 native adapter 的长期 schema compatibility。`NativeAdapter.adapter_schema_version`
  是另一条线，不属于本轮 local concrete restore key 规则；本文只处理 local
  artifact/build gate。
- 不保证新写入的 recoverable v2 bytes 能被旧 runtime 读取。Skiff 尚未发布，旧 runtime
  回滚需要清理或重写对应本地数据。

## 目标数据模型

新增 recoverable envelope v2。encoder 只写 v2；decoder 只接受 v2。旧 v1 bytes 一律以
unsupported recoverable schema/version 或 state invalid fail closed，不做 compatibility shim。
v2 schema 常量必须是 recoverable 容器格式常量，不包含 service/package/build/activation
信息；v1、unknown schema、state invalid、current concrete missing、projection missing 和
durable state mismatch 需要映射到稳定 recoverable error code 分类，测试不得只匹配文案。

v2 的 local behavior 身份不再叫 `LocalCode`，避免误导为“定位某个历史 artifact 的代码”：

```rust
enum RecoverableCodeIdentity {
    None,
    LocalConcrete {
        owner: LocalConcreteOwner,
        concrete_type_identity: String,
    },
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

struct InterfaceValueState {
    self_node: Box<RecoverableNode>,
}

enum NominalObjectState {
    DefaultFields {
        fields: Vec<RecoverableField>,
    },
    Custom {
        durable_state: Box<RecoverableNode>,
    },
}
```

说明：

- `LocalConcrete.concrete_type_identity` 是 stable concrete restore key，格式固定为
  `abi-type:` + lowercase hex of `AbiTypeId::key_bytes()`，其中 `AbiTypeId` 必须由
  artifact-model 的 `abi_type_id_from_source_anchor` 生成。它仍然需要存：恢复 `any I` 时
  expected type 只说明“这里需要一个 `I`”，不知道历史值是 `A implements I` 还是
  `B implements I`。
- `LocalConcrete.owner` 参与 lookup。`Service` 表示当前 service artifact 内的 concrete
  type；`Package { package_id }` 表示 concrete type 来自当前 linked program 中该 package
  id 对应的 package unit。owner 只允许语义身份，不允许 service version、package version、
  package build id、package slot、source hash 或本地路径。lookup key 是
  `(owner, concrete_type_identity)`；若当前 linked program 找不到该 owner/concrete pair，
  decode fail closed。
- `artifact_identity` / `build_id` / service version / package version / activation identity /
  activation-local id
  从 local behavior durable state 中移除。当前 request 的 linked program 和 method table
  是唯一解释上下文。
- `InterfaceValueState.interface_identity` 和 `method_projection_identity` 从 durable state
  中移除。decode 时由 `RuntimeRecoverableExpectedTypeNode::AnyInterface` 提供，并用当前
  linked program 校验 `LocalConcrete.concrete_type_identity` 是否仍 conform。
- `any I` 的 `I` 不进入 durable payload。单一 expected `any I` 由代码类型直接提供；union
  expected 必须在 decode 时解析到唯一可匹配 any-interface 分支，若 `A` 同时实现 `I` 和
  `J` 且 expected 是 `any I | any J`，decode fail closed，而不是按分支顺序猜测。
- `restore_schema_version` 从 local interface self wrapper 中移除。envelope 自身的
  `schema_version` 如仍保留，只描述 recoverable 容器/二进制格式，不得编码 service/package/
  build/activation，也不得参与 local concrete 的兼容判断。
- 新写入绝不产生 `LocalCode`。旧 v1 `LocalCode { artifact_identity, build_id,
  concrete_type_identity, ... }` 不被解释为 `LocalConcrete`；旧 DB 数据由清理策略处理。

## Stable Restore Key 设计

删除 artifact/build gate 之前，必须先新增 stable restore key。现有
`linked_type_ref_runtime_key(concrete_type)` 不能继续用于 v2 durable payload：linker 会把
concrete type 解析成 `Address { TypeAddr }`，其中 `LoadedFileIndex`、package slot 和
`type_index` 都是当前 linked runtime image 的局部地址，不能跨 activation 或 rebuild 比较。

v2 必须新增显式 restore key：

```rust
struct LocalConcreteRestoreKey {
    owner: LocalConcreteOwner,
    concrete_type_identity: String,
}
```

### Key wire format

`concrete_type_identity` 的唯一合法格式是：

```text
abi-type:<lowercase-hex-encoded AbiTypeId key bytes>
```

其中 `AbiTypeId` 必须由 artifact-model 的
`abi_type_id_from_source_anchor(anchor, type_args)` 生成，`anchor` 来自 compiler/source
`SourceDeclarationAnchor` 投影到 `AbiSourceAnchorInput`：

- `publication_id`：service id 或 package id 的 canonical publication id。
- `abi_epoch`：`PublicationIdentity.abi_epoch`。这是显式 ABI owner epoch，不是 service/package
  version、build id 或 artifact identity；改变它是有意打断 ABI identity 的 breaking 操作。
- `module_path`：source anchor 的 module path segments。
- `symbol`：type declaration symbol。
- `kind`：必须是 `AbiDeclarationKind::Type`。
- `type_args`：按声明顺序递归编码，每个 type arg 必须先投影为 `AbiTypeId`，再把它的
  `key_bytes()` 作为 child key。type arg 里出现 alias 时按 compiler ABI 规则展开到 target；
  出现 builtin/record/union/nullable/function 等非 named source type 时，A0 必须先定义其
  ABI type-id 投影或拒绝该 generic concrete 的 recoverable encode，不能把
  `LinkedTypeRef::Address`、JSON descriptor 或 runtime type shape 写入 durable key。

`AbiTypeId::key_bytes()` 当前由 artifact-model 使用 length-framed binary bytes 表示。v2
recoverable wire format 固定使用这些 bytes 的 lowercase hex 字符串；后续即使 artifact-model
内部改为 hash，也必须保留该 recoverable wire encoding 的读写兼容，或在新的 recoverable
format 中显式迁移。禁止 worker 自行选择 JSON、debug string、`serde_json`、base64、source
path 或其它 encoding。

### Owner 与 lookup 唯一性

要求：

- `owner = Service` 时，key 只表示当前 service 的 source/ABI owner，不存 service version、
  service build id 或 artifact identity。service id 已包含在 `AbiTypeId` 的 `publication_id`
  中；decode 时必须校验 `AbiTypeId.publication_id == current service_id`，且只能在当前
  owner-internal service context 的 current linked program index 内查找，不能跨 service
  registry 或跨 service DB 查找。
- `owner = Package { package_id }` 时，只存 package id，不存 package version、package build
  id、package slot、package store path 或 dependency alias。当前 linked program 负责把
  package id 解析到本次执行所加载的 package unit。linked program 中同一 package id 必须唯一；
  如果发现 0 个或多于 1 个 candidate package unit，decode fail closed。decode 还必须校验
  `AbiTypeId.publication_id == package_id`，不一致时 fail closed。
- `concrete_type_identity` 不得包含 descriptor hash、schema hash、file IR identity、source
  hash、file path、`TypeAddr`、`LoadedFileIndex`、package slot、type table index、build id、
  service/package version 或 activation identity。
- artifact load/link 阶段需要建立 `(owner, concrete_type_identity) -> current TypeAddr /
  restore expected plan / method table entries` 的索引，eval hook 只从这个索引编码和恢复。索引
  构建时若同一 `(owner, concrete_type_identity)` 对应多个不同 concrete declaration、不同
  restore expected plan 或互不等价的 method table set，必须 fail closed 并拒绝加载/编码；decode
  时 0 个或多于 1 个 match 都 fail closed。
- 增加 identity fixture/unit test：同一 service/package concrete type 在两个不同 build id 下，
  stable restore key 相同；不同 owner、不同 concrete type 或不同 stable type args 不碰撞；
  package id 重复、service owner 与当前 context 不匹配、同 key 多 concrete candidate 都
  fail closed。

没有实现并验证该 key 时，不得进入 runtime/model、eval hook 和 DB 集成实现。

## 旧数据处理策略

旧 v1 recoverable-envelope DB 数据不迁移、不兼容读取。处理方式：

- 本地 dev/stable DB 中受影响的 recoverable-envelope 字段或集合可以删除、清空或由业务重新
  创建。
- 本方案验证前必须先清理现有 v1 recoverable-envelope 数据，或者只在新写入 v2 数据的
  collection/chat/thread 上验证。现有 v1 数据读取失败是预期清理信号。
- v2 runtime 读取旧 v1 bytes 时应 fail closed；这是清理信号，不是 runtime regression。
- 实现不增加 v1 legacy 中间表示，不增加 `allow_legacy_v1_local_self`，不把旧
  `InterfaceValueState.interface_identity` / `method_projection_identity` 或
  `restore_schema_version` 带入 v2 语义。
- 需要保留业务数据时，由应用层导出 plain business facts 后重建，不在 recoverable codec
  中做兼容。

## 恢复流程

### Encode

1. boundary codec 根据 expected type plan 编码值。
2. 遇到 local `InterfaceValue` 时，要求 boundary 是 owner-internal，且 expected plan 经
   alias/nullable/union 解析后唯一选中一个 `AnyInterface`。unresolved expected plan 不允许
   编码 behavior node；union 中多个 any-interface 分支可匹配同一 concrete 时，encode
   fail closed。
3. `EvalRecoverableBehaviorHooks::encode_local_interface_self` 根据当前 linked program 找到
   `(interface, projection, concrete_type)` 的 method table entry。
4. 使用当前 artifact 中 `LocalConcreteRestoreKey { owner, concrete_type_identity }`
   对应的 recoverable expected plan 编码 self payload。取不到 concrete self expected plan
   时，encode fail closed；不允许继续使用 `unresolved("local interface self")` 作为
   production 路径。
5. 写出 `self_node`：

```text
value_kind = NominalObject
code_identity = LocalConcrete { owner, concrete_type_identity }
state = Custom { durable_state }
```

6. DB 写入不再为 `LocalConcrete` 生成 artifact retention root。若 tree 中仍存在
   `NativeAdapter` 且 owner 需要 artifact，则继续走对应 native adapter availability/retention
   机制。

### Decode

1. canonical decoder 读取 envelope。只接受 v2；v1 或未知 schema version fail closed。
2. `trust_boundary != OwnerInternal` 时仍先扫描并拒绝 behavior-bearing node。
3. 按 expected type plan 做预检。预检策略由 boundary 决定：
   - spawn/runtime owner-internal transient payload：strict。
   - DB recoverable-envelope durable read：durable DB policy，见下一节。
4. 遇到 `InterfaceValue` 时，先校验 `self_node` 是 `NominalObject + LocalConcrete` 并提取
   `LocalConcreteRestoreKey`；`LocalCode`、旧 wrapper 或旧
   `restore_schema_version` 一律不是 v2 合法 local self。
5. 从 expected type 唯一取得 interface/projection。payload 中不再读取 wrapper
   interface/projection。若 expected 是 union，用第 4 步提取出的 `LocalConcreteRestoreKey`
   对当前 linked program 做 conformance 检查；没有分支匹配或多个 any-interface 分支匹配都
   fail closed，不能按 union 分支顺序猜测。
6. 用当前 linked program 查找 `(owner, concrete_type_identity)` 的 restore plan，按当前
   decode policy decode durable state 得到 concrete self。DB durable read 的 policy 必须传入
   这个嵌套 decode；不能在 local self 内部回落到 strict 默认。
7. 用当前 linked program 校验 concrete type 仍实现 expected interface/projection，重建
   method table，返回 `InterfaceValue { carrier = Local { concrete_type, method_table,
   payload } }`。
8. 如果当前 artifact 中找不到 concrete type、concrete type 不再 conform、method table
   projection 不存在或 durable state 与当前 restore plan 不匹配，按稳定 recoverable error
   fail closed。这时失败原因是当前 schema/类型不接受旧值，不是 artifact unavailable。

## DB schema 不一致策略

这次实现不解决完整 DB migration，但必须避免 v2 local artifact/build gate 抢先失败。
下面的“历史 envelope”只指本方案上线后由较早 schema/build 写入的 v2 envelope，不包括
旧 v1 bytes。DB recoverable-envelope lane 的 read policy 如下：

- 新增 nullable 字段：历史 v2 envelope 缺字段时通过，decode 时 materialize 为 `null`，
  而不是让 runtime object 缺少该字段。
- 新增 required 字段：历史 v2 envelope 缺字段时失败。当前没有默认值/field initializer
  migration 机制，不能猜。
- 删除字段：历史 v2 envelope 多出的字段在 DB durable read 中忽略；strict transient
  payload 仍拒绝未知字段。
- 字段改名：等价于“旧字段多出 + 新字段缺失”。如果新字段 required，则失败；如果新字段
  nullable，则读出 `null`，旧字段值不会自动迁移。
- 字段类型改变：递归按 expected type 检查；只有 nullable/union 等明确拓宽能接受旧 shape，
  其它类型变化 fail closed。
- projection 没选中 recoverable-envelope 字段时，不触发该字段 decode。
- missing nullable materialization 只发生在“已经选中并正在 decode 的 recoverable-envelope
  record 内部”。如果 DB projection 没选择某个 top-level recoverable-envelope 字段，不能
  因字段缺席而 materialize 为 `null`。

实现上不要全局放宽 `precheck_record_fields`。应引入显式策略参数，例如：

```rust
enum RecoverableRecordUnknownFieldPolicy {
    Reject,
    Ignore,
}

struct RecoverableDecodePolicy {
    unknown_record_fields: RecoverableRecordUnknownFieldPolicy,
    materialize_missing_nullable_fields: bool,
}
```

`RecoverableBoundaryCodec::decode*` 默认使用 strict policy。service-db 的
`runtime_value_from_recoverable_envelope_bson` 和
`business_value_from_recoverable_envelope_bson` 在 DB read 场景传入 durable DB policy。
encode 路径保持 strict，避免当前代码写出 schema 外字段。

policy 必须随 decode context 贯穿所有递归路径，包括 behavior hook 内部对 local interface
self `durable_state` 的嵌套 decode。实现可以把 policy 放入 `RuntimeRecoverableBoundaryContext`
或新增到 `RecoverableLocalInterfaceRestoreRequest`，但不能只作为顶层
`RecoverableBoundaryCodec::decode*` 的临时参数。否则 `A implements I` 的 self record 新增
nullable 字段时，顶层 DB read 会走 durable policy，local self 内部却 strict 失败，核心场景
仍无法跨版本恢复。

## Spawn 策略

`spawn` 不跨版本。same-build 指 spawned request 的 target executable 必须在 submitting
runtime 的同一个 service/version/build 上执行。约束属于 router/runtime 控制面，不属于
recoverable payload。当前 `spawn.submit` 路径没有 delayed execution 语义；router 默认使用
内存队列，queue item 有 `maxQueueWaitMs`、lease TTL 和 `createdAt`，但没有“未来某时执行”的
durable schedule。本文只覆盖当前 same-service spawn 路径；如果未来开放跨 service spawn，
必须重新定义控制面 routing/ownership，不得通过 recoverable payload 存版本来解决。

控制面规则：

- submit header 必须携带 service id、service version、service protocol identity 和 target，
  这些字段用于 router compatibility key、queue item 和 claim 过滤。
- build id / activation identity 可以由 runtime submit header 显式提供，也可以由 router 从
  submitting runtime 的 registered source metadata 兜底写入 queue item。无论来源如何，
  queue item 中的 build id 必须等于 submitting runtime 的 build id。
- worker claim 必须按自己的 current build id 过滤 queue item。若目标 build 当前没有 loaded
  runtime，claim/dispatch 在 payload decode 前失败。claimed worker 的 build id 必须等于
  queue item build id，也就是 submitting runtime/source metadata 写入的 build id。
- claim response 构造 `RequestEnvelope` 时使用 queue item / claim descriptor 的 service
  version、build id、activation identity 作为执行上下文；payload decode 不参与 build 校验，
  也不能作为 fallback。
- service version、build id、activation identity 是 submit/queue/claim 元数据，可以短期存于
  router 队列；它们不得写入 spawn args recoverable bytes，也不得写入 local behavior
  `LocalConcrete`。
- spawn args payload 写 v2 recoverable bytes，不含 `artifact_identity`、`build_id`、service
  version、package version 或 activation identity。
- spawn decode 使用 target executable 的当前 expected plan，policy 仍 strict。payload
  schema 不一致说明 control plane 路由到了错误 build 或 payload 损坏，应 fail closed。

需要补测试：构造 spawn args 中含 local `any I`，断言 canonical envelope 中没有
artifact/build/version/activation 字符串；同 build decode 成功；普通 runtime binary decode
仍拒绝 recoverable magic；cross-service/external trust 仍拒绝 behavior-bearing envelope；
submit header build id 为 `None` 时，router 用 source build id 写入 queue item，并且只有同
build worker 能 claim。

## 实现 DAG

### A0. Stable restore key 设计与索引

依赖：无。

改动：

- 新增 `LocalConcreteOwner` 和 `LocalConcreteRestoreKey`，明确 service/package owner 都不含
  version/build/activation。
- compiler/source 把每个 recoverable concrete type 的 `SourceDeclarationAnchor` 投影为
  `AbiSourceAnchorInput`，调用 artifact-model `abi_type_id_from_source_anchor`，并把
  `abi-type:` + lowercase hex(`AbiTypeId::key_bytes()`) 写入 `concrete_type_identity`。
- 泛型 type args 按 artifact-model `AbiTypeId` 递归编码；无法投影为 named ABI type id 的 type
  arg 在 v2 local behavior encode 时 fail closed。本轮只要求支持所有 type args 都能投影为
  `AbiTypeId` 的 generic concrete；非 named source type args 的 ABI 投影若未在 A0 明确定义，
  不得编码为 durable local behavior。
- 在 linked program / eval hook 可访问的位置建立
  `(owner, concrete_type_identity) -> TypeAddr / restore expected plan / method table entries`
  索引。
- 索引构建和 decode lookup 都必须执行唯一性检查：当前 service context 内 0 个或多于 1 个
  owner/concrete candidate 都 fail closed；package id 重复或同 key 多 concrete declaration 也
  fail closed。
- 禁止 v2 encoder 使用 `linked_type_ref_runtime_key`、`TypeAddr`、package slot、type table
  index、JSON descriptor、debug string、base64 或 `PackageCoordinate { name, version }` 作为
  durable restore key。
- 增加不同 build id 下同一 concrete/interface schema key 相同、不同 owner/package/concrete
  或不同 stable type args 不碰撞的测试；增加固定 golden fixture，断言同一
  `AbiSourceAnchorInput + type_args` 得到完全相同的 `abi-type:<hex>` 字符串。

验收：identity/key 测试先于 B-F 通过；文档记录该 key 不含 version/build/source/path/runtime
address，也不含 package slot/type table index；同 key 多 candidate、package id 重复和无法稳定
投影的 generic type args 都 fail closed。

### A. 文档契约同步

依赖：A0。

改动：

- 更新 `doc/architecture/recoverable-value.md`：删除“行为值按写入时 artifact 恢复”的长期
  结论，改为 owner-internal local behavior 按当前 execution context 恢复。
- 更新 `doc/reference/spawn.md`：说明 spawn payload 不承载 artifact/build，same-build 是控制面
  约束。
- 更新 `doc/reference/any-interface-value.md`：说明 typed recoverable boundary 的
  interface/projection 来自 expected type，不来自 durable wrapper truth；union 中多个
  any-interface 分支可匹配时 fail closed。

验收：文档不再同时声明旧模型和新模型。B-F 可以并行准备实现分支，但不能在 A 合入前
合入或作为完成状态验收。

### B. Recoverable model 与 canonical codec

依赖：A0。A 可以并行开始，但 B 的最终命名应与 A 一致。

改动：

- 在 `runtime/model/src/recoverable.rs` 增加 v2 schema 常量和 `LocalConcrete`。
- v2 encoder 写 `LocalConcrete { owner, concrete_type_identity }`，不写 artifact/build/version/
  activation。
- v2 `InterfaceValueState` 只写 `self_node`。
- v2 `NominalObjectState::Custom` 不写 `restore_schema_version`。
- decoder 只接受 v2；v1 或未知 envelope schema/version fail closed。
- `collect_artifact_refs` 不收集 `LocalConcrete`。

验收：model/boundary 单测覆盖 v2 roundtrip、v1/unknown schema decode 失败、新写入不含
artifact/build/version/activation。

### C. Boundary codec 与 decode policy

依赖：B。

改动：

- `runtime/boundary/src/recoverable.rs` 的 `RecoverableBoundaryCodec::decode*` 增加
  policy-aware 内部入口，公开 strict 默认入口保持现有调用语义。
- `precheck_record_fields` 按 policy 处理 unknown fields。
- record decode 在 expected record 下 materialize missing nullable field 为 `Null`。
- plain/unresolved expected plan 不允许 decode behavior-bearing `InterfaceValue`。
- any-interface expected selection 必须能从 expected plan 唯一得到 interface/projection；
  union 中多个 any-interface 分支对同一 `LocalConcrete` 可匹配时 fail closed。
- policy 默认值为 strict：`unknown_record_fields = Reject`、
  `materialize_missing_nullable_fields = false`。
- policy 必须传给 behavior hook 的 local self 嵌套 decode；可以通过 context 或 restore
  request 字段传递，但所有递归 decode 使用同一个 policy。
- untrusted behavior scan 继续在 expected precheck 之前执行。
- `carrier = Remote` 的 `InterfaceValue` 在所有 recoverable encode/decode 边界都 fail closed；
  owner-internal 也只允许 local carrier。

验收：strict payload 仍拒绝 extra field；DB durable policy 忽略 extra field；missing
nullable field decode 为 `Null`；missing required field 失败；union any-interface 多匹配失败；
DB durable policy 下 local self 内部 missing nullable 字段也 decode 为 `Null`；remote carrier
不能编码成 durable `any I`。

### D. Eval behavior hooks

依赖：B、C。

改动：

- `runtime/eval/src/recoverable_behavior.rs` 停止写
  `INTERFACE_SELF_RESTORE_SCHEMA_VERSION`。
- encode hook 从 A0 索引写 `LocalConcrete { owner, concrete_type_identity }`。
- restore hook 只接受 v2 `LocalConcrete`；不比较 artifact/build。
- restore hook 从当前 linked program 的 stable key 索引查 concrete restore expected plan，并用
  当前 method table registry 校验 conformance/projection。
- restore hook 的 durable self decode 使用传入的 `RecoverableDecodePolicy`；DB read 场景不得
  回落到 strict。
- 错误文案从 “written by a different artifact/build” 改为当前 concrete type/projection 不可用
  或 durable state 不匹配。

验收：两个不同 build id 的 hook 对同一 concrete/interface schema roundtrip 成功；当前
program 中移除 concrete 或 projection 时稳定失败；同一 package id 的 compatible package
version 变化不因版本字符串提前失败，而是进入当前 schema/conformance 判断。

### E. Service DB integration

依赖：B、C、D。

改动：

- `runtime/service-db/src/mapping.rs` 的 recoverable-envelope read 传 DB durable decode
  policy。
- write path 仍 strict。
- `CurrentRequestRecoverableArtifactStore` 不再用于 `LocalConcrete`。如保留 artifact store，
  只服务仍需要历史 artifact/native adapter owner 的节点。
- 更新 `runtime/service-db/src/tests.rs`：
  - extra historical v2 record field under DB durable policy is ignored。
  - new required field still fails。
  - projection omitted envelope field does not decode it。
  - v1 recoverable envelope decode fails with stable unsupported schema/state error。
  - new v2 bytes written by two different build ids decode by current expected type and no longer
    produce `recoverable_artifact_unavailable`。
  - local `any I` self durable state 内部缺少新增 nullable 字段时，DB durable policy 可恢复为
    `Null`；新增 required 字段仍失败。
  - v2 local behavior bytes 中不包含 service version、package version、build id 或 activation
    identity。
  - current concrete missing、projection missing、state mismatch、unknown schema/v1 schema 使用
    稳定 recoverable error code 分类，测试不只匹配易变文案。

验收：focused service-db tests 通过。

### F. Spawn integration

依赖：B、C、D。

改动：

- `runtime/eval/src/spawn_ops.rs` 和 `runtime/eval/src/recoverable_spawn_payload.rs` 使用
  v2 behavior-aware recoverable payload。
- 保持 spawn decode strict policy。
- router/control-plane 保持 serviceVersion/buildId/activationIdentity 为 queue/claim 元数据；
  args recoverable payload 不写这些字段。
- 在 `runtime/host/src/host/spawn_worker.rs`、`runtime/host/src/host/route_registry.rs`
  或对应 host tests 中覆盖 wrong-build claim/dispatch：错误 build 必须在 request route
  lookup 前后、payload decode 前失败。
- 增加 eval/router/host tests，确保 build id 是 spawn 控制面字段或 router source metadata
  兜底字段，payload 中不含 local artifact/build/version/activation。

验收：现有 spawn tests 通过；新增 local `any I` spawn payload 测试通过；wrong-build
claim/dispatch 不进入 payload decode。测试应使用会在 decode 时失败的 payload、panic
decode stub 或 decode counter 证明该路径没有调用 payload decode；submit build id 为 `None`
时 queue item 使用 source build id 且只被同 build worker claim。

### G. 全量验证与收尾

依赖：A0-F。

验证命令：

```bash
cargo test --manifest-path runtime/Cargo.toml -p skiff-runtime-model -p skiff-runtime-boundary --no-fail-fast
cargo test --manifest-path runtime/Cargo.toml -p skiff-runtime-eval -p skiff-runtime-service-db --no-fail-fast
pnpm test
```

如改动 artifact schema 或 runtime protocol 后需要端到端验证：

```bash
node scripts/skiff.mjs instance build .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
```

验收：清理现有 v1 recoverable-envelope 数据后，由旧 v2 build 写入但当前 schema 兼容的
DB recoverable field 不再因为 artifact/build mismatch 失败；如果 schema 真不兼容，错误
指向 expected type/state mismatch。

## Worktree 与多 agent 分工

建议实现阶段使用 3 个并行 worker worktree，最后合入 Skiff `main`：

- Worker 1：B + C，负责 `runtime/model`、`runtime/boundary`、decode policy 和对应单测。
- Worker 2：D + F，负责 `runtime/eval` 的 behavior hook、stable key 使用和 spawn tests。
- Worker 3：E，负责 `runtime/service-db` integration 和 DB tests。E 的 mapping/policy wiring
  可在 C 后先做；涉及 local self roundtrip 的验收测试必须等 D 的 hook 改动合入后完成。

主 agent 负责 A0、A、跨 worker 冲突裁决、最终全量验证和 merge。每个 worker 只在自己的
worktree 提交，验收通过后由主 agent 合并回 `main` 并删除临时 worktree/branch。

## 风险与缓解

- **错误 build 执行 spawn payload**：payload 不再自带 build guard，必须依赖 spawn 控制面。
  缓解：把 build check 放在 claim/dispatch/request route lookup，增加 router/host 测试，
  claim 到错误 build 时在 payload decode 前失败。
- **DB unknown field 被忽略导致数据丢失**：只有 DB durable read 忽略未知字段；write 仍 strict。
  删除字段后的 read-modify-write 会按当前 schema 重写并丢弃旧字段，这是删除字段的预期结果。
  改名/迁移旧值仍需要 DB migration。
- **projection 缺字段与 envelope 内部缺字段混淆**：projection 未选择 top-level envelope
  字段时不得 materialize nullable 字段。缓解：DB durable decode policy 只作用于已选中并
  正在 decode 的 envelope 内部 record。
- **LocalConcreteRestoreKey 不够稳定**：如果 v2 误用 `linked_type_ref_runtime_key`、
  `TypeAddr`、package slot、type table index、service/package version 或 build id，DB 仍无法
  跨版本恢复。缓解：A0 作为前置实现任务，未通过 key 稳定性测试不得进入 runtime/model
  实现。
- **worker 自行选择 key encoding**：如果不同模块分别用 JSON、debug string、base64 或不同
  ABI projection，跨 build 恢复会不稳定。缓解：唯一合法 wire format 是
  `abi-type:` + lowercase hex(`AbiTypeId::key_bytes()`)，并用 golden fixture 锁定。
- **owner lookup 歧义**：如果当前 linked program 中同 package id 多 unit 或同 key 多 concrete
  declaration，可能恢复到错误 concrete。缓解：package id/current service owner 必须唯一；
  0 个或多于 1 个 owner/concrete match 都 fail closed，并测试覆盖。
- **union any-interface 恢复错误分支**：移除 durable interface/projection 后，`any I | any J`
  这类 expected union 可能对同一 concrete 出现多匹配。缓解：多匹配 fail closed，不能按
  union 分支顺序猜测，也不把 `I` 写回 payload。
- **DB durable policy 没有进入 local self**：如果 policy 只传到顶层 decode，local
  interface self 内部仍会 strict 失败。缓解：policy 放入 context 或 restore request，所有
  behavior hook 嵌套 decode 共享同一 policy，并用 self 内部 missing nullable 字段测试覆盖。
- **package version 重新成为 hard gate**：如果复用 `PackageCoordinate { name, version }`，
  package compatible version 升级会在 schema/conformance 前失败。缓解：local restore owner
  只存 `Package { package_id }`，当前 linked program 决定实际 loaded package。
- **旧 DB 数据读取失败**：v2 不兼容 v1，旧 DB 数据会 fail closed。缓解：本地清理受影响
  recoverable-envelope 字段/集合；需要保留业务事实时由应用层重建，不在 codec 中迁移。
- **放宽 record precheck 影响安全边界**：policy 必须显式传入；默认 strict；cross-service 和
  external-untrusted 仍先拒绝 behavior-bearing payload。
- **旧 runtime 回滚**：新 runtime 写 v2 后旧 runtime 不能读。Skiff 未发布，回滚策略是 revert
  code 后清理或重写本地受影响 DB field，不提供双写。

## 完成标准

- 新写入的 owner-internal local behavior recoverable bytes 不包含 artifact/build/version/
  activation identity。
- `LocalConcreteRestoreKey { owner, concrete_type_identity }` 经测试证明跨 build 稳定，且不包含
  service/package version、build id、source hash、file path、runtime address、package slot、
  type table index 或 `TypeAddr`。
- `concrete_type_identity` 的唯一 wire format 是 `abi-type:` + lowercase
  hex(`AbiTypeId::key_bytes()`)，有 golden fixture；generic type args 递归使用 `AbiTypeId`
  child key，无法稳定投影时 encode fail closed；本轮只要求支持 type args 均可投影为
  `AbiTypeId` 的 generic concrete。
- owner lookup 在当前 service context 内唯一；package id 重复、同 key 多 concrete candidate、
  service owner 不匹配、owner 与 `AbiTypeId.publication_id` 不一致都 fail closed。
- typed interface wrapper 的 interface/projection 来自 expected type plan，不来自 durable payload。
- union any-interface 多匹配 fail closed，不按分支顺序猜测，也不把 `I` 写入 payload。
- local interface self 不再写 runtime wrapper `restore_schema_version`。
- DB read 不再因 stored artifact/build 与当前 build 不同而失败。
- v1/旧 local self 不被任何 boundary 接受；旧 DB recoverable-envelope 数据按清理策略处理。
- spawn payload 不含 local artifact/build/version/activation identity，same-build 约束由 spawn
  控制面测试覆盖，wrong-build dispatch 在 payload decode 前失败，并由测试证明 decode 未被
  调用。
- current concrete missing、projection missing、state mismatch、v1/unknown schema 等失败路径有
  稳定 recoverable error code 分类。
- DB schema mismatch 行为符合本文 v2 矩阵：新增 nullable 可读为 null，新增 required
  失败，删除字段可读，类型不兼容失败；该 policy 同样作用于 local interface self 的嵌套
  durable state。
- 非 owner-internal behavior envelope 仍 fail closed；`carrier = Remote` 不会被编码为 durable
  `any I`。
