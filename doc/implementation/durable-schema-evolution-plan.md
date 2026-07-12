# Durable Schema Evolution Implementation Plan

日期：2026-07-10

本文是实现方案，不是长期架构契约。目标是把 DB object、DB recoverable-envelope
field、spawn / queue durable payload 等持久化数据的历史 shape 升级收敛成一套可实现、可测试、有明确回滚门禁的机制，避免应用为了兼容旧数据把当前 required 字段错误建模成 nullable。

长期语义最终应并入：

- `doc/architecture/recoverable-value.md`
- `doc/reference/db.md`
- `doc/reference/static-semantics.md`
- 后续正式的 migration reference

## Background

当前 recoverable durable DB read policy 只有两类兼容行为：

- 历史 record 多出的字段可忽略。
- 历史 record 缺失 nullable 字段时 materialize 为 `Null`。

缺失 required 字段会 fail closed。这个行为在 recoverable codec 和 service DB adapter 中已经固化：

- `runtime/boundary/src/recoverable.rs` 的 `RecoverableDecodePolicy::durable_db()` 只包含 `ignore_unknown_record_fields` 和 `materialize_missing_nullable_record_fields`。
- `runtime/boundary/src/recoverable.rs` 的 `precheck_record_fields` 对 missing required field 直接失败。
- `runtime/service-db/src/mapping.rs` 在 recoverable-envelope DB field runtime read 中使用 durable DB policy。
- `doc/architecture/recoverable-value.md` 明确写出“缺失 required field 失败”。

这会把历史兼容压力泄漏到业务类型设计中。典型例子是 Agine `HostProviderMount.currentDirectory`：当前语义希望它是 required string，但为了从 DB 中恢复旧 `any ToolProvider` self payload，应用被迫把它写成 `string?`。

这个问题不只属于 recoverable envelope。DB object 本身也会遇到同类演进：

- 新增 required stored field。
- 字段 rename / split / merge。
- record / union durable shape 改变。
- 可查询字段或索引字段变更。

因此需要统一 durable schema evolution，而不是只给 recoverable codec 增加一个 missing-field default。

## Goals

- 保持当前业务类型正确：required 字段仍然是 required，nullable 只表示当前业务值域允许 `null`。
- 支持开发者为历史 durable shape 提供显式 migration 逻辑，把旧数据升级为当前 durable shape。
- 支持两种语言内读路径：
  - readonly migration：读到旧 shape 时在内存中升级，返回当前值，不写回。
  - read-repair：读到旧 shape 时在 storage 层通过 CAS / transaction 写回当前 shape，再返回当前值。
- 确保所有 current durable write 都写入 current revision metadata，禁止新数据继续落成 absent-revision legacy shape。
- 明确 `recoverable` 与 `repairable` 的边界。
- 为 DB object/document 与 recoverable envelope 复用同一套 migration runner，允许不同 storage adapter 接入。
- 保证 DB query / order / index 语义不被 read-time migration 模糊化。
- 让没有 migration plan 的历史 required 字段缺失成为部署/迁移配置错误，而不是诱导应用把字段改 nullable。
- 区分 code rollback 和 data rollback：readonly migration 必须支持代码回滚；普通 current write 必须按 `CurrentRevisionWritePolicy` 通过显式 gate 或 privileged operator assertion proof；read-repair 必须通过显式 repair gate。

## Non-Goals

- 不提供语言内批量修复工具。批量 backfill / offline repair 属于语言外运维工具。
- 不在 decode 阶段读取外部状态，例如 DB、文件系统、网络、当前进程 cwd、clock/random 或 live host。
- 不自动根据类型生成默认值，例如 string -> `""`、number -> `0`、bool -> `false`。
- 不从字段名相似、字段顺序、record shape、union branch 顺序推断迁移。
- 不让 read migration 修复 DB `where` / `order` / index 已经发生的漏查、错排或索引缺失。
- 不复用 `RecoverableEnvelope.schema_version`、service version、build id 或 artifact identity 作为业务 durable shape revision。
- 不要求所有 recoverable value 都支持 repair。
- 不在第一版支持自动 reverse migration。需要回滚已 repair 数据时，必须依赖 forward-compatible reader、外部 repair 工具，或部署系统禁止回滚到旧 reader。

## Camp Principle Check

本功能会直接经过以下代码路径：

- `runtime/boundary/src/recoverable.rs`：recoverable expected type validation、durable DB decode policy、missing field precheck。
- `runtime/service-db/src/mapping.rs` 和 `runtime/service-db/src/capability.rs`：DB stored value materialization、recoverable-envelope field 读写、projection/query 边界。
- `runtime/model/src/recoverable.rs` 与 artifact metadata crates：expected type plan、recoverable metadata、future migration plan metadata。
- spawn / queue payload runtime modules：platform-owned durable payload 的 decode 和 dispatch 前控制点。
- `doc/architecture/recoverable-value.md`、`doc/reference/db.md`、`doc/reference/static-semantics.md`：公开语义和约束。

会被本功能放大的现有隐式契约：

- “durable read policy 等于 recoverable codec policy”现在被硬编码在 recoverable decode 和 DB adapter 之间。新增 migration 后必须把 shared rule 收敛到 `DurableMigrationRunner` 和 cell adapter contract，不能在 DB object、recoverable envelope、spawn、queue 各自复制一套 missing-field 兼容逻辑。
- `required` 目前同时承担 current type required 和 historical storage required 的含义。实现必须引入 durable shape revision 和 migration plan，避免继续用 `nullable` 表达历史兼容。
- DB object 与 recoverable-envelope field 都有 recoverability requirement，但存储 lane 不同。实现必须先定义 cell adapter boundary，防止 envelope-specific metadata 泄漏到 schema-projectable DB document。
- query/index 与 read-time materialization 的边界目前主要靠文档约定。实现 migration 时必须同步增加 metadata guard，让 migrated fields 默认不能参与 storage query/index。

本次必须前置或同阶段完成的清理：

- 抽出 migration runner 与 adapter contract，再接入各读路径。
- 收敛 durable write path，确保 DB document、DB field envelope、spawn payload、queue payload 的新写入都使用同一套 revision metadata 规则。
- 增加 migration hook 的受限执行模型和 effect 校验，不能只靠约定要求 hook 纯。
- 把 missing required field 的错误分类从 generic decode error 拆成 stable migration configuration/runtime error。
- 为 reserved storage metadata 建立统一命名和拒绝用户字段冲突的校验。

允许作为 follow-up 的内容：

- 语言外 batch migration tool。用户明确要求本方案不处理批量修复工具。
- production index backfill rollout 细节。本文只要求语言/runtime 拒绝 unsafe query/index，不定义运维编排工具。
- 最终语法打磨和 IDE 体验。第一版仍必须有最小可用声明入口，不能只靠手写 artifact metadata。

## Core Definitions

### Recoverable

`recoverable` 表示一个值可以从 durable / transient payload 恢复成当前 request 中的等价值。它描述 encode/decode 能力，不包含写回能力。

示例：

- DB recoverable-envelope field 中的 `any I` self payload。
- spawn payload。
- queue payload。
- 显式 recoverable slot。

### Repairable

`repairable` 表示某个 recoverable durable value 有明确 storage cell、写回权限和 CAS / transaction 语义，可以把迁移后的当前 durable shape 写回。

并非所有 recoverable value 都 repairable：

- 普通 service 参数、runtime transient payload、public API payload、已从 DB 读出的 heap object 没有 repair target。
- 嵌套值不能独立 repair；只能 repair 所在的外层 durable cell，例如整个 DB document 或整个 DB field envelope。
- partial projection 默认不 repair，因为读取方没有完整 durable cell。

### Durable Cell

`DurableCell` 是 read migration / read-repair 的最小单位。

第一版支持以下 cell kind：

- `DbDocumentCell`：一个 DB object/document 的完整 stored document。
- `DbFieldEnvelopeCell`：一个 top-level recoverable-envelope DB stored field。
- `SpawnPayloadCell`：一个平台可写的 spawn payload record。
- `QueueItemPayloadCell`：一个平台可写的 queue item payload record。

后续可增加 Redis / future storage adapter，但必须提供同样的 revision、read、CAS / transaction 和权限语义。

### Durable Shape Revision

`DurableShapeRevision` 是业务 durable shape 的版本身份，用于选择 migration chain。它独立于：

- `RecoverableEnvelope.schema_version`：codec/envelope format version。
- service version / package version：发布与依赖选择版本。
- build id / artifact identity：artifact locator。
- `LocalConcrete.concrete_type_identity`：恢复 concrete type 的 stable key。

第一版 revision 格式应是稳定字符串，例如：

```text
durable-shape:<owner>:<cell-kind>:<identity>:<revision>
```

其中：

- `owner` 是 service id 或 package id。
- `cell-kind` 是 `db-document`、`db-field-envelope`、`spawn-payload`、`queue-payload` 等。
- `identity` 是 DB object identity、DB field identity、spawn target identity 或 queue topic / payload identity。
- `revision` 是开发者维护的单调或显式兼容 revision 名。

spawn / queue 的 `identity` 必须包含 target binding / handler binding identity，因为它们的 current revision 由当前 binding 的 expected payload shape 决定，不是 service 的全局最新 shape。

## Target Architecture

所有 durable read 先通过 cell adapter 得到 stored durable shape，再进入同一 migration runner：

```text
storage read
  -> durable cell adapter loads stored bytes/document
  -> detect stored durable shape revision
  -> select migration chain to current revision
  -> decode and validate historical input with input_shape_plan
  -> apply pure migration hooks
  -> validate current durable shape
  -> readonly: decode and return current value
  -> read-repair: CAS / transaction write current shape, then decode and return
```

DB object/document 与 recoverable envelope 共用 migration runner，区别在 adapter：

- DB object adapter 读写 BSON/document。
- DB field envelope adapter 读写 `RecoverableEnvelope` / `RecoverableNode` binary。
- spawn / queue adapter 读写平台 durable payload record。

## Stored Revision Placement

第一版 schema-projectable DB object 只使用 document-level durable shape revision，不做 field-level revision metadata。field-level safety 只用于 projection / write / query / index guard，不用于选择 per-field migration chain。

### DB Document

DB document cell 必须有独立 metadata 存储当前 durable shape revision。第一版采用 reserved metadata field：

```text
__skiffDurableShapeRevision
```

规则：

- 该字段不属于业务 stored fields。
- 不可被业务 `fields` projection、`where`、`order`、change block 或 index 引用。
- full document read 必须读取它。
- 旧文档缺少该字段时，adapter 根据当前 DB object 的 `legacy_revisions` 中的 `AbsentRevisionMetadata` rule 判定历史 revision。
- 没有匹配的 legacy rule 且缺 metadata 时，read migration 必须报稳定 migration 配置错误。
- 即使读路径没有 matching legacy rule，query/index safety 仍必须把 pre-metadata 数据当作可能存在的 `AbsentRevisionMetadata` source，除非 control-plane 有 creation epoch 或 `RevisionPresenceMarker { state: Absent }` 证明该 cell 不存在缺 metadata 的历史 rows/items。

### DB Field Envelope

recoverable-envelope DB field 的 revision 不复用 `RecoverableEnvelope.schema_version`。第一版在 envelope root 附近增加 durable state metadata，或在 BSON field wrapper 中加旁路 metadata。

实现上优先采用 wrapper BSON shape，避免改变 envelope codec contract：

```json
{
  "__skiffRecoverableEnvelope": <binary>,
  "__skiffDurableShapeRevision": "..."
}
```

兼容规则：

- 现有 binary-only field 视为 legacy encoding。
- legacy binary field 的 revision 由 DB field migration plan 的 `legacy_revisions` detection rule 指定。
- wrapper shape 只用于 recoverable-envelope lane，不影响 schema-projectable lane。
- business materialization 不暴露 wrapper metadata。

如果后续决定把 revision 放入 recoverable envelope 内部，必须保持 `schema_version` 仍只表示 codec format，不承担业务 migration selection。

### Spawn Payload

spawn payload repair 只能发生在平台拥有 payload storage cell 且尚未交付业务执行前。

revision 存放在 spawn work item metadata：

```text
payload_shape_revision
```

规则：

- claim / dispatch 前可 readonly migrate 或 read-repair。
- payload 已交付给业务执行后不 repair。
- “current revision” 由该 spawn target 在当前恢复/dispatch context 中绑定的 expected payload shape 决定，不是全局最新版。

### Queue Payload

queue payload 与 spawn 类似，revision 存放在 queue item metadata：

```text
payload_shape_revision
```

规则：

- 只能在 claim / dispatch 前的受控事务中 repair。
- item 一旦交付给业务 handler，本次交付不再改写 payload。
- current revision 由 queue topic / handler binding 的当前 expected payload shape 决定。

## Durable Cell Plan Model

artifact metadata 增加 durable cell plan registry。每个 durable cell 都必须有 `DurableCellPlan`，即使它没有 migration step。没有 migration 的 cell 使用 zero-step plan：`migrations = []`，但仍然声明 current revision、current shape、write policy 和默认 safety。

```rust
struct DurableCellRegistry {
    cells: Vec<DurableCellPlan>,
    hooks: Vec<DurableMigrationHook>,
}

struct DurableCellPlan {
    cell_identity: DurableCellIdentity,
    current_revision: DurableShapeRevision,
    current_shape_plan: DurableShapePlan,
    current_shape_plan_digest: ArtifactDigest,
    legacy_revisions: Vec<LegacyRevisionRule>,
    migrations: Vec<DurableMigrationStep>,
    current_write: CurrentRevisionWritePolicy,
    read_repair: ReadRepairPolicy,
    storage_field_safety: Vec<StorageFieldSafetyRule>,
}

struct DurableMigrationStep {
    from_revision: DurableShapeRevision,
    to_revision: DurableShapeRevision,
    input_shape_plan: DurableShapePlan,
    output_shape_plan: DurableShapePlan,
    hook_id: String,
    affected_fields: Vec<DurableFieldIdentity>,
    migration_edge_digest: ArtifactDigest,
}

struct DurableMigrationHook {
    hook_id: String,
    entrypoint: ArtifactFunctionIdentity,
    hook_digest: ArtifactDigest,
    input_shape_plan_digest: ArtifactDigest,
    output_shape_plan_digest: ArtifactDigest,
    effect: MigrationHookEffect,
}

enum MigrationHookEffect {
    PureDeterministic,
}

struct LegacyRevisionRule {
    revision: DurableShapeRevision,
    detection: LegacyRevisionDetection,
}

enum LegacyRevisionDetection {
    AbsentRevisionMetadata,
}

struct StorageFieldSafetyRule {
    field_identity: DurableFieldIdentity,
    operation: StorageFieldOperation,
    index_identity: Option<DurableIndexIdentity>,
    from_revision: DurableShapeRevision,
    to_revision: DurableShapeRevision,
    proof: StorageFieldSafetyProof,
}

enum StorageFieldOperation {
    Project,
    Write,
    Where,
    Order,
    Index,
    UniqueConstraint,
}

enum StorageFieldSafetyProof {
    StoredCompatibleAcrossRevision,
    ExternalBackfillComplete { marker_id: String },
}

struct StorageBackfillMarker {
    marker_id: String,
    cell_identity: DurableCellIdentity,
    field_identity: DurableFieldIdentity,
    operation: StorageFieldOperation,
    from_revision: DurableShapeRevision,
    to_revision: DurableShapeRevision,
    target_shape_plan_digest: ArtifactDigest,
    migration_edge_digest: ArtifactDigest,
    compatibility_scope: CompatibilityScopeId,
    storage_watermark: StorageWatermark,
    writer_fence_id: WriterFenceId,
    index_identity: Option<DurableIndexIdentity>,
}

enum StoredRevisionRef {
    Explicit(DurableShapeRevision),
    AbsentRevisionMetadata,
}

struct RevisionPresenceMarker {
    marker_id: String,
    cell_identity: DurableCellIdentity,
    revision: StoredRevisionRef,
    target_revision: DurableShapeRevision,
    target_shape_plan_digest: ArtifactDigest,
    migration_edge_digest: Option<ArtifactDigest>,
    compatibility_scope: CompatibilityScopeId,
    storage_watermark: StorageWatermark,
    writer_fence_id: WriterFenceId,
    state: RevisionPresenceState,
}

enum RevisionPresenceState {
    Present,
    Absent,
}

enum CurrentRevisionWritePolicy {
    RollbackCompatible { requirement: CurrentWriteCompatibilityRequirement },
    Gated { gate_id: DurableRevisionGateId },
}

enum CurrentWriteCompatibilityRequirement {
    OperatorAssertion { assertion_id: String },
}

enum ReadRepairPolicy {
    Disabled,
    BestEffort { gate_id: DurableRevisionGateId },
    Required { gate_id: DurableRevisionGateId },
}
```

`StoredRevisionRef` 是 storage 中“可能存在的 source revision”身份。migration DAG、`current_revision`、`to_revision` 和 `StorageFieldSafetyRule.from_revision/to_revision` 仍只使用 explicit `DurableShapeRevision`。

`AbsentRevisionMetadata` 在 read migration 中是 legacy detection rule；在 storage safety 中是 `StoredRevisionRef::AbsentRevisionMetadata` reserved sentinel。若 service 声明 `LegacyRevisionRule { detection: AbsentRevisionMetadata }`，缺 revision metadata 的 stored value 映射到该 rule 的 concrete historical `DurableShapeRevision`，后续 migration/safety coverage 使用这个 concrete revision。若 service 没有声明该 rule，runtime 不能读取这类 value，但 query/index/partial safety 仍必须把 reserved absent sentinel 当作可能存在，直到 creation epoch 或 revision absence marker 证明不存在 pre-metadata 数据。该 sentinel 不能作为 `current_revision`、`to_revision` 或 migration edge endpoint。

`DurableShapePlan` 由 adapter 决定：

- DB document：schema-projectable stored field graph、envelope field placeholders、document metadata fields。它不包含 recoverable-envelope wrapper bytes 的内部 shape。
- DB field envelope：recoverable expected type plan 加 field identity table。
- spawn / queue payload：expected recoverable payload type plan。

compiler 必须为所有 durable cells 生成 plan：

- DB object/document cell。
- top-level recoverable-envelope DB field cell。
- spawn payload cell。
- queue payload cell。

如果 artifact 中的 durable write path 找不到对应 `DurableCellPlan`，artifact load 或 write must fail with stable durable cell registration error。没有 metadata 的 current write 不能退化成 legacy absent rule。

### Zero-Step Cell Bootstrap

没有 explicit migration declaration 的 durable cell 由 compiler 自动生成 zero-step `DurableCellPlan`：

- `current_revision` 使用当前 cell identity 和 current shape digest 生成稳定 bootstrap revision。
- `current_shape_plan` / `current_shape_plan_digest` 来自当前 schema 或 expected payload type plan。
- `legacy_revisions = []`。旧 absent-revision 数据不会自动当作 current；如果需要读取 pre-metadata 数据，service 必须显式声明 `AbsentRevisionMetadata` legacy rule 和 no-op migration，或先做语言外 backfill。
- zero-step plan 不声明 absent-revision read migration。对 storage safety 来说，除非 control-plane 有 cell creation epoch 或 `RevisionPresenceMarker { state: Absent }` 证明没有 pre-metadata 数据，否则 `AbsentRevisionMetadata` 仍是该 cell 的可能 stored revision。
- `migrations = []`。
- `current_write = Gated { gate_id: bootstrap gate }`。
- bootstrap gate id 由 compiler 从 cell identity、current revision 和 shape digest 生成。
- deployment/control-plane 在首次激活该 artifact 前自动创建 bootstrap gate record；只有确认 compatibility scope 中没有不兼容 active/rollback reader/writer 时才打开。
- gate missing 等价 closed；compiler 不得自行证明 live-scope safety，也不得绕过 gate。
- `read_repair = Disabled`。
- schema-projectable DB document current-to-current baseline safety 由当前 schema 自动生成，只覆盖 zero-length current path；如果 possible stored revisions 包含 `AbsentRevisionMetadata` 或其它 historical revision，baseline safety 不能放行 query/index/partial projection。
- recoverable-envelope zero-step cell 没有 DB storage field safety。

这意味着普通新 service 不需要手写 migration declaration，但仍会获得可写 current revision metadata；有历史 absent-revision 数据的 service 必须显式选择 migration 或外部 backfill。

对已有 collection/cell 的 zero-step artifact，query/index/partial projection 默认不能假设全量数据都是 current revision。deployment/control-plane 必须先写入 creation epoch / revision absence marker，或 service 显式声明 legacy rule + backfill/safety rule，才能启用依赖 current-only baseline 的 storage operation。

revision 与 shape plan 绑定规则：

- `current_revision` 必须与 `current_shape_plan_digest` 一起校验。
- 同一 artifact 中同一个 `DurableShapeRevision` 不能绑定到多个不同 shape digest。
- migration step 的 `to_revision == current_revision` 时，`output_shape_plan` digest 必须等于 cell 的 `current_shape_plan_digest`。
- zero-step cell 的 current read/write 仍按 `current_shape_plan` validate。

migration edge digest 绑定规则：

- 每个 `DurableMigrationStep` 必须有 `migration_edge_digest`。
- digest 覆盖 `cell_identity`、`from_revision`、`to_revision`、input/output shape plan digest、`hook_digest`、effect、affected fields 和 storage safety metadata。
- compatibility scope 内同一 `cell_identity + from_revision + to_revision` 必须唯一绑定一个 edge digest。
- hook 实现变化、affected fields 变化或 safety metadata 变化，必须 bump revision 或生成不同 edge digest；deployment/control-plane 不允许不同 edge digest 的同 edge artifacts 同时进入 compatibility scope。

migration runner 必须选择一条确定的链：

```text
stored_revision -> ... -> current_revision
```

如果不存在精确链，read 必须以稳定 migration error 失败。runner 不得跳过 revision、推断 shape 兼容，或在多条可能链中任选一条。

第一版 legacy detection 只支持每个 cell 一个 `AbsentRevisionMetadata` rule。也就是说，缺少 revision metadata 的旧数据只能映射到一个历史 revision。如果同一 cell 已经存在多个无法通过 metadata 区分的历史 shape，service 必须先用语言外工具 backfill 到带 revision 的 shape，再启用语言内 read migration。

## Migration Declaration

第一版必须提供最小用户可见声明入口，并由 compiler 生成 artifact metadata。不能要求开发者手写 `DurableCellRegistry`。

语法可以标记为 unstable，但必须表达：

- cell identity。
- from / to durable shape revision。
- legacy revision detection。
- migration hook entrypoint。
- affected logical fields。
- current write policy。
- read-repair policy。
- storage field safety rules。

示意：

```skiff
@durableMigration(
  cell: dbFieldEnvelope("skiff.run/agine:HostProviderMount:self"),
  from: "durable-shape:skiff.run/agine:db-field-envelope:HostProviderMount.self:v1",
  to: "durable-shape:skiff.run/agine:db-field-envelope:HostProviderMount.self:v2",
  legacy: absentRevisionMetadata,
  affectedFields: ["currentDirectory"],
  currentWrite: gated("agine-host-provider-mount-v2-write"),
  readRepair: disabled,
  storageFieldSafety: [],
)
pure function migrateHostProviderMountV1(input: HostProviderMountV1): HostProviderMountV2 {
  ...
}
```

这个示例是 `DbFieldEnvelopeCell`，`currentDirectory` 是 envelope 内部 logical field，不是 DB schema-projectable storage field；因此 `storageFieldSafety` 为空。opaque envelope 内部字段不能参与 DB projection / where / order / index / partial document write safety。

如果第一版不想稳定最终语法，可以把它实现成 internal annotation / manifest-backed declaration，但要求：

- declaration 必须在 service source tree 中，随 artifact 编译。
- compiler 必须 typecheck hook input/output 与 shape plan。
- compiler 必须生成 hook digest、shape plan digest 和 migration registry。
- runtime 只消费 artifact metadata，不从 source tree 重新解析 migration declaration。

## Migration Hook Contract

Migration hook 是开发者编写、由 artifact metadata 引用的代码。

约束：

- 必须纯且确定。
- 不得 DB read/write。
- 不得访问文件系统或网络。
- 不得读取 clock/random。
- 不得读取 live connection、当前进程 cwd、环境变量、host metadata 或 runtime config。
- 不得产生 spawn / queue / telemetry 副作用。
- 必须在正常 request 执行限制内终止。
- 输入是 `input_shape_plan` 声明的历史 durable shape。
- 输出必须通过 `output_shape_plan` 校验。

如果新增 required 字段只能从外部状态计算，语言层 read migration 不成立。service 必须先用语言外批量工具或显式业务 repair/backfill 处理，再声明这些历史数据兼容。

### Historical Input Decode

runner 在调用 hook 前必须按 migration step 的 `input_shape_plan` 解码并校验 stored value：

- historical decode 使用 adapter-specific historical policy，不使用 current expected type policy。
- input shape 中缺 required field 是 corrupt historical data，不能把 ill-typed value 传给 hook。
- input shape 中缺 nullable field 是否 materialize 为 null，由 input shape plan 和 adapter historical policy 决定。
- unknown fields 只能按 input shape plan 的 declared behavior 保留或忽略，不能由 current shape policy 推断。
- union branch 必须能按 historical shape identity 精确识别；无法识别时报 stable migration configuration/runtime error。
- corrupt envelope bytes、unknown node kind、shape digest mismatch 或 adapter decode failure 都必须在 hook 前失败。

hook 只接收已经通过 `input_shape_plan` 校验的 historical value。

### Hook Execution Model

hook 的纯度必须由 compiler 和 runtime 共同 enforcement：

- migration hook 只能声明为 `pure function`。
- compiler 在 empty capability environment 下 typecheck hook。
- hook 不能捕获 request-scoped capability、DB capability、host capability、runtime config、environment 或 live connection。
- hook 只能调用同样标记为 pure deterministic 的函数。
- compiler 拒绝 clock/random、filesystem、network、telemetry、spawn、queue、DB read/write 等 effect。
- artifact metadata 记录 hook bytecode / IR digest，以及 input/output shape plan digest。
- runtime 在 capability-free evaluator frame 中执行 hook；如果执行中请求任何 capability，立即 trap 为 stable migration hook violation。
- runtime 在应用 hook 后再次校验 output shape，不能只信任 compiler。

测试必须覆盖：

- 调用非 pure 函数被 compiler 拒绝。
- hook 尝试访问 DB / host / clock / random 被拒绝。
- hook digest 或 shape plan digest 不匹配时 artifact load 或 migration read 失败。
- hook 输出缺 required field 时 migration 失败。

## Current Durable Write Path

所有 current durable write 必须通过 durable cell adapter 写入 current revision metadata。新写入数据不能继续依赖 absent-revision legacy detection。

current revision full-cell write 路径包括：

- DB document insert / replace / full-cell change block commit。
- top-level recoverable-envelope DB field write。
- spawn work item create。
- queue item enqueue。

DB schema-projectable field update 如果不 materialize full document，是 revision-preserving partial write，不是 current revision full-cell write。
change block commit 如果 materializes full durable document，走 current revision full-cell write；如果只写 selected schema-projectable fields 且保留 observed revision metadata，走 revision-preserving partial write，并必须满足 `Write` safety。
read-repair writeback 不是 ordinary current revision full-cell write；它由 `ReadRepairPolicy` 控制，但使用同一 durable evolution state store 的 gate/digest/scope 语义。

写入规则：

- 写入前必须按 current shape plan 校验业务值。
- ordinary current full-cell write 授权模型由 `CurrentRevisionWritePolicy` 单独决定：
  - `Gated { gate_id }`：只有 matching durable revision gate open 时成功。operator assertion proof 对该 policy 无效，不能绕过 gate。
  - `RollbackCompatible { requirement }`：只有 matching privileged operator assertion proof 有效时成功。gate 对该 policy 无效，不能替代 proof。
- `CurrentRevisionWritePolicy::RollbackCompatible` 表示 artifact 声明了 compatibility requirement；runtime 必须从 privileged proof store 验证对应 proof 后，才可以直接写 current revision。
- `CurrentRevisionWritePolicy::Gated` 表示 current write 是 data migration point；gate closed 时，任何会创建或更新该 cell current revision 的普通业务写入都必须以 stable current-write-gate error 失败。
- 当前 write policy 授权通过后，adapter 写入 current shape 和 current durable shape revision。
- current write 缺失 revision metadata 是 writer bug，必须稳定失败，不能被 `AbsentRevisionMetadata` legacy rule 吞掉。
- gate 是 monotonic deployment/runtime state。第一版不支持 gate 从 open 回到 closed 后继续声明普通 rollback safety。

授权真值表：

| Write kind | Artifact policy | 需要的 state-store record | 缺失或不匹配时 |
| --- | --- | --- | --- |
| ordinary current full-cell write | `Gated { gate_id }` | matching open gate，绑定 cell identity、target revision、target shape digest、relevant migration edge digests（如果存在）和 compatibility scope | `durable_revision_gate_error`；proof 不参与判断 |
| ordinary current full-cell write | `RollbackCompatible { OperatorAssertion { assertion_id } }` | matching privileged operator assertion proof，绑定 assertion id、cell identity、target revision、target shape digest、relevant migration edge digests（如果存在）和 compatibility scope | `durable_current_write_proof_error`；gate 不参与判断 |
| read-repair writeback | `ReadRepairPolicy::BestEffort/Required` | matching open repair gate，绑定 cell identity、target revision、target shape digest、relevant migration edge digests（如果存在）和 compatibility scope | `BestEffort` 降级为 readonly；`Required` 报 `durable_repair_gate_error`；proof 不参与判断 |

`RollbackCompatible` assertion 规则：

- artifact metadata 只能声明 `CurrentWriteCompatibilityRequirement`，不能携带最终 proof。
- 第一版只支持 `OperatorAssertion`。它要求 proof store 中存在 privileged deployment assertion；service artifact 只能引用 assertion id，不能自己声明或伪造。
- assertion 必须绑定 exact target revision、target shape digest、relevant migration edge digests 和 compatibility scope。revision 没有全序，不能用“大于等于”或前缀匹配推断兼容。
- assertion 缺失、scope mismatch、target digest mismatch、edge digest mismatch 或权限不足时，`RollbackCompatible` current write 以 stable current-write-proof error 失败。

adapter 规则：

- DB document writer 必须写入 `__skiffDurableShapeRevision = current_revision`，并保留 adapter 未建模但需要保留的 reserved metadata。
- DB partial update 如果不 materialize 完整 document，只能保留 observed revision metadata，并且只能更新在该 observed revision 下有 `Write` safety rule 的字段。
- DB partial update 的 observed revision 必须是 explicit metadata revision；缺 metadata / absent legacy document 不能走 revision-preserving partial write，必须 full materialize + migrate + CAS 到 current revision，或 fail closed。
- DB partial update 没有 `Write` safety rule 时，必须 full materialize durable cell、运行 migration 到 current revision，并通过 current write policy / gate 检查后用 CAS / transaction 写 current revision。
- 不允许把 v2 语义字段直接写进仍标记为 v1 的 document，除非该字段对 observed revision 有 explicit `Write` safety proof。
- recoverable-envelope DB field writer 必须写 wrapper shape，包含 envelope bytes 和 `__skiffDurableShapeRevision`。
- spawn / queue writer 必须在 work item metadata 写 `payload_shape_revision = current_revision`。
- enqueue / spawn create 不能写 absent metadata payload。

## Readonly Migration

readonly migration 运行在可以 materialize 完整 durable cell、但不应写回的 read path 中。

允许：

- Full DB document read.
- Full recoverable-envelope DB field read.
- Spawn / queue payload decode before dispatch when platform does not write repair.

不允许：

- 省略了 migration 所需部分的 partial DB projection。
- DB query predicate evaluation。
- DB order / index evaluation。
- 已经 materialize 的 heap object。

readonly migration 向业务代码返回当前值，但不更新 stored revision 或 bytes。

## Read-Repair

read-repair 是可选能力，由 `ReadRepairPolicy` 和 deployment/runtime durable revision gate 共同控制。

它需要 `RepairHandle`：

```rust
struct RepairHandle {
    cell_identity: DurableCellIdentity,
    observed_revision: DurableShapeRevision,
    observed_etag: StorageEtag,
    write_capability: RepairWriteCapability,
}
```

只有满足以下条件时才允许 repair：

- adapter 已加载完整 durable cell。
- adapter 有 CAS / transaction primitive。
- 当前 request 有执行平台 repair 的权限。
- migration chain 是纯的，并产出了当前 durable shape。
- `ReadRepairPolicy` 不是 `Disabled`。
- policy 对应的 `DurableRevisionGateId`、target shape digest 和 relevant migration edge digests 可在 durable evolution state store 中查询。

repair 行为：

- CAS conflict：reload 一次并重新运行 migration。
- gate open 且 CAS success：写入 current shape/current revision，并返回 decoded current value。
- `BestEffort` 遇到重复 CAS conflict、gate closed、target digest mismatch 或 storage write unavailable 时，返回 readonly migrated value 并记录 diagnostic telemetry。
- `Required` 遇到重复 CAS conflict、gate closed、target digest mismatch 或 storage write unavailable 时，以稳定 repair error 失败。
- projection read 不 repair，除非 adapter 能证明所选 projection 就是完整 durable cell。

read-repair 是平台维护副作用，不是用户可见 DB update statement。它必须通过平台 telemetry 可观测，并且除了消除历史 shape debt 之外，不得改变业务 operation 语义。

repair write 必须保留 current reader 不理解但 storage adapter 要求保留的字段：

- DB document repair 不能用 blind full replace 丢弃 unknown stored fields 或 reserved metadata。
- DB field envelope repair 只能替换对应 top-level field wrapper。
- 如果 migration 明确删除或重命名业务字段，adapter 必须基于 output shape plan 和 affected fields 执行有目标的 merge/update。

## Rollback Contract

readonly migration 不写 storage，因此只要没有普通 current write 或 read-repair writeback 发生，旧 artifact 仍按旧 shape 读取旧数据。

普通 current write 和 read-repair 都会把 storage cell 写成 current revision，因此它们都是显式 data migration point。第一版不提供自动 reverse migration。打开 `Gated` current write gate、写入 `RollbackCompatible` proof，或打开 read-repair gate 前，必须满足以下任一条件：

- rollback window 已结束，deployment 系统不再允许回滚到不能读取 current revision 的 artifact；
- 所有允许回滚的 artifact 都携带能读取 current revision 的 forward-compatible reader；
- operator 明确使用语言外工具完成数据回滚或重新 backfill。

`DurableRevisionGateId` 是 deployment/runtime 状态，不是 artifact build id。artifact 只声明“这个 cell 的 `Gated` current writes 或 BestEffort/Required repair 需要哪个 gate”。`RollbackCompatible` artifact 不使用 gate 放行 ordinary current write，只使用 privileged operator assertion proof。

回滚规则：

- `Gated` ordinary current write 在 gate closed / missing / mismatch 时以 current-write-gate error 失败；operator assertion proof 不能放行。
- `RollbackCompatible` ordinary current write 在 proof missing / mismatch / unauthorized 时以 current-write-proof error 失败；gate 不能放行。
- `BestEffort` repair 在 repair gate closed / missing / mismatch 时等价于 readonly migration；`Required` repair 以 `durable_repair_gate_error` 失败；proof 不参与 read-repair。
- gate open 或 proof 写入后，部署系统必须把不能读取 current revision 的 artifact 标记为不可回滚目标。
- 如果 service 需要在 gate open / proof 写入后仍允许旧 artifact 回滚，旧 artifact 必须先发布 forward-compatible reader 或 reverse migration 由语言外工具完成。

验收必须包含：`Gated` ordinary current write 在 gate closed 时不会写 storage，即使 proof store 中有 unrelated proof；`RollbackCompatible` ordinary current write 只有 valid privileged operator assertion proof 时可写，即使 gate open 也不能替代 proof；repair gate closed 时不会写 storage；授权通过后完成对应 ordinary write / repair；写入 current revision 后回滚到不支持 current revision 的 artifact 被部署门禁拒绝或由 forward-compatible reader 成功读取。

## DB Query, Projection, Order, And Index Policy

read migration 只发生在 storage 已经选出候选 rows/items 之后。它不能修复 query-time 语义。

规则：

- `where` / `order` / index 在 migration 前使用 storage shape。
- 如果新增或变更字段参与 `where`、`order`、unique constraint 或 index definition，deployment 必须要求先完成外部 backfill / index rollout，再启用该 query/index。
- 当字段所需 migration 未在 `StorageFieldSafetyRule` 中标记为 safe 时，compiler 或 runtime metadata 应拒绝这类 projection/write/query/index 使用。
- storage field safety 默认为 false，runner 不得从 hook body、字段名或 shape diff 推断 safety。
- readonly migration 可以为返回对象 materialize 缺失的 required field，但不会让旧 rows 因该字段而变得可搜索。

`StorageFieldSafetyRule` 只适用于 schema-projectable DB document lane。recoverable-envelope lane 是 opaque bytes；envelope 内部 logical fields 只能出现在 migration hook input/output 和 `affected_fields` 中，不能作为 DB storage projection/write/query/index safety 对象。

`StorageFieldSafetyRule` 的粒度是 stored field identity + operation + migration edge：

- `Project`：字段可在 partial projection 中直接 materialize 为 current value，不需要 full-cell migration。
- `Write`：字段可在保留 observed document revision metadata 的 partial update 中写入，不会制造未命名的 mixed shape。
- `Where`：字段可出现在 storage predicate 中。
- `Order`：字段可出现在 storage sort key 中。
- `Index`：字段可作为 non-unique index key。
- `UniqueConstraint`：字段可作为 unique constraint / unique index key。

每条 rule 必须有证据：

- `StoredCompatibleAcrossRevision`：该字段在 exact edge `from_revision -> to_revision` 的 storage 表示未改变，且 source revision 已存储可直接读取/查询的值。
- `ExternalBackfillComplete { marker_id }`：语言外 backfill 或 index rollout 已完成，并由 deployment/runtime 提供对应 marker。

`StoredCompatibleAcrossRevision` 不是 developer assertion。compiler 必须用 input/output shape plan 和 storage encoding 证明它成立：

- field identity 在 source 和 target shape 中相同。
- storage encoding、nullable/required materialization、collation/order semantics、normalization、index key encoding 和 unique comparison semantics 均未改变。
- operation 为 `Project` / `Write` 时，current value 的 storage representation 与 observed revision representation 相同，不需要 migration hook 才能读写。
- operation 为 `Where` / `Order` / `Index` / `UniqueConstraint` 时，storage predicate/sort/index/unique semantics 在 exact edge 内相同。
- rename / split / merge / derived field / changed normalization / changed index encoding 都不能使用该 proof，必须使用 `ExternalBackfillComplete`。

compiler 无法证明时必须拒绝 `StoredCompatibleAcrossRevision` rule。runtime 不重新证明该 proof，只消费 compiler-validated metadata。

`from_revision -> to_revision` 必须对应 migration DAG 中的一条 exact edge，或 current-to-current zero-length baseline。rule 不跨多跳路径、不跨分叉路径、不自动继承到未来 current revision。多跳 migration 必须为每条 edge 分别声明/生成 safety，或用 revision absence marker 证明中间 revision 已不存在。

affected fields 完整性：

- compiler 必须从 input/output shape plan 计算 structural field diff。
- declaration 中的 `affected_fields` 必须覆盖 compiler 计算出的 diff。
- 如果 hook 做同 shape 的语义变换，开发者也必须显式声明 affected fields。
- 对 schema-projectable DB document lane，如果 hook 不是 compiler 可证明的 identity-preserving transformation，则该 edge 上该 cell 的 schema-projectable fields 默认 unsafe，直到通过 `ExternalBackfillComplete` marker 或 compiler-proven `StoredCompatibleAcrossRevision` rule 放行。
- 对 recoverable-envelope lane，affected logical fields 不产生 DB storage safety。
- 对 rename/split/merge 这类 hook，开发者必须显式列出所有受影响的新旧 stored field identity。

如果某字段缺少覆盖目标 operation 和 required edge 的 safety rule，则相关 projection/query/order/index/unique constraint 必须失败。

current-to-current baseline safety：

- 对 current revision 到 current revision 的 zero-length path，schema-projectable fields 按当前 schema 的普通 DB semantics baseline-safe。
- zero-step cell 可以正常 project/write/query/index current data，不需要显式 migration safety rule。
- baseline safety 只在本次 operation 的 possible stored revisions 集合精确等于 `{ current_revision }` 时放行。
- baseline safety 不跨 historical revision 生效；只要 storage 中可能存在 historical revision 或 `AbsentRevisionMetadata`，就必须按混合 revision 覆盖算法检查 safety。

混合 revision 数据集覆盖算法：

- 对 `Project`，目标 revision set 是被读取 row 的 observed revision；没有 matching `Project` safety rule 时，partial projection 必须失败，除非 adapter 显式升级为 full-cell read path、运行完整 read migration 语义后只向业务返回投影字段。
- 对 `Write`，目标 revision set 是被更新 document 的 observed revision；没有 matching `Write` safety rule 时必须 full-cell read/migrate/CAS 到 current revision 后再写。
- 对 `Where` / `Order` / `Index` / `UniqueConstraint`，目标 revision set 是该 collection/cell 当前可能存在于 storage 中的所有 stored revisions。
- 可能存在的 stored revisions 由 durable marker store 的 per-cell revision-presence universe 提供，类型是 `StoredRevisionRef`，不能只从当前 artifact 的 `legacy_revisions` 或 migration DAG 推导。
- possible set 至少包含 state store 中所有 `Present` 且未被有效 `Absent` marker 覆盖的 revisions、当前 artifact 的 `current_revision`、当前 artifact 声明的 `legacy_revisions` 和 migration DAG reachable revisions。
- 对任何没有 creation epoch 或 `RevisionPresenceMarker { state: Absent, revision: AbsentRevisionMetadata }` 证明不存在 pre-metadata 数据的 cell，`StoredRevisionRef::AbsentRevisionMetadata` 也属于 possible set，即使该 cell 是 zero-step plan 且 `legacy_revisions = []`。
- 对 `StoredRevisionRef::Explicit(r)`，如果 `r == current_revision`，可以使用 current-to-current baseline；否则 `r` 到 current revision 的每条 edge 都必须有覆盖目标 field/operation 的 safety rule，或有 marker 证明该 source revision 已不存在于 storage。
- 对 `StoredRevisionRef::AbsentRevisionMetadata`，如果 cell plan 有 matching `AbsentRevisionMetadata` legacy rule，则先映射到该 rule 的 concrete historical revision，并按 explicit revision 覆盖；如果没有 matching legacy rule，则只有 creation epoch / absence marker 能移除该 source，否则相关 storage operation 必须失败。
- 混合 absent-revision / v1 / v2 数据集只要有一个可能 revision 未被覆盖，相关 storage query/order/index/unique usage 必须失败。

### Durable Evolution State Store

第一版必须提供 production durable evolution state store，不允许只实现 test injection。

生产实现：

- store 使用 Skiff platform/runtime control-plane durable storage，逻辑上独立于业务 DB object collections。
- 第一版至少提供四类 records：storage backfill markers、revision-presence markers、revision gates、compatibility proofs。
- writes 只能来自 privileged deployment/backfill actor；service runtime、service code 和普通 request 只能 read。
- 每条 record 必须绑定 `cell_identity`、target revision、target shape plan digest 和 compatibility scope；只有与 source->target migration/backfill 相关的 record 才绑定相关 migration edge digest。`Present` marker 和 creation epoch absence marker 不得伪造不存在的 edge digest。
- revision-presence store 是 per-cell revision universe，不属于单个 artifact。它必须保留历史上被允许写入或仍可能存在的 `StoredRevisionRef`，直到有有效 absence marker 证明该 source revision 已不存在。
- compatibility scope 包含所有 active live artifact/route/worker/queue handler binding，以及 rollback candidates。gate/proof 不能只检查 rollback candidates。
- deployment 打开 gate、写 proof、写 backfill marker 或写 revision-presence marker 前，必须确认 scope 内没有不兼容 active reader/writer，或它们都带 forward-compatible reader。
- deployment 打开 `Gated` current write gate、写入 `RollbackCompatible` proof、或打开 read-repair gate 前，必须先写入或确认 `RevisionPresenceMarker { state: Present, revision: Explicit(target_revision) }`。这样后续 artifact 即使没有声明旧 migration，也会从 possible-stored-revisions API 看到该 revision。
- artifact / route / worker / queue handler 激活前，deployment/control-plane 必须对每个 durable cell 查询 possible stored revisions；cell plan 必须能读取/migrate 所有 present explicit revisions，并能处理或排除 absent sentinel，否则拒绝激活。只含 zero-step plan 的 artifact 不能覆盖已有 present historical revision。
- 对 marker writes，scope 还必须排除会继续写入 covered old revision / unbackfilled data 的 active writers，或先通过 writer gate/fence 阻止这些 writes。
- gate/proof/marker 写入后形成 durable compatibility fence。任何 artifact、route、worker、queue handler binding 或 spawn target binding 后续进入 active scope 前，deployment/control-plane 必须查询这些 fences，并拒绝不兼容 entrant。
- production store 必须提供 query APIs 给 runtime：gate lookup、proof lookup、marker lookup、possible stored revisions lookup。
- `possible_stored_revisions(cell_identity, scope)` 必须返回 `StoredRevisionRef` 集合，来源是 per-cell revision-presence universe 加当前 artifact 声明的 current/legacy/DAG revisions；它不能因为当前 artifact 没有声明旧 revision 就把 state store 中的 present revision 丢掉。
- 本地测试可以注入 in-memory store，但 production adapter 不能从 service code 接受 override。

### Backfill Marker Contract

`ExternalBackfillComplete { marker_id }` 引用 deployment/runtime 的 durable marker store。第一版只定义 marker contract，不定义语言外 backfill 工具实现。

marker 必须满足：

- marker id 精确匹配 `StorageFieldSafetyRule.proof.marker_id`。
- `cell_identity`、`field_identity`、`operation`、`from_revision`、`to_revision`、`target_shape_plan_digest`、`migration_edge_digest` 都与 safety rule / cell plan 匹配。
- `Index` / `UniqueConstraint` marker 还必须匹配 `index_identity`。
- marker 由 privileged deployment/backfill actor 写入，service runtime 只读。
- marker 是 monotonic durable state；第一版不支持自动过期或由 service 代码撤销。
- marker 写入必须绑定 storage watermark / transaction snapshot 和 writer fence id，证明 backfill 覆盖该 watermark 前的数据；watermark 之后的 writes 必须已经被 writer gate/fence 约束。
- marker 缺失、scope mismatch、edge digest mismatch、watermark/fence mismatch 或 index mismatch 都按 unsafe 处理。

marker store 还必须提供 revision-presence state：

- `revision_present(cell_identity, StoredRevisionRef)`：该 source revision 可能仍存在。对 explicit revision，present record 由 gate/proof/repair gate 或 artifact activation 写入；对 absent sentinel，缺少 creation epoch / absence marker 时默认可能存在。
- `revision_absent(cell_identity, StoredRevisionRef, marker_id)`：privileged actor 通过 `RevisionPresenceMarker { state: Absent }` 声明该 source revision 已不存在。
- `RevisionPresenceMarker { state: Present }` 必须绑定 `cell_identity`、`revision = Explicit(target_revision)`、target revision、target shape digest、compatibility scope、writer fence id；`migration_edge_digest` 为 `None`，因为 present 表示 target 可能被写入，不是某条 source->target migration 已完成。
- explicit source revision 的 absence marker 必须绑定 `cell_identity`、`revision = Explicit(source_revision)`、target revision、target shape digest、相关 migration edge digest、compatibility scope、storage watermark、writer fence id 和完成扫描/backfill 的 marker id。
- `AbsentRevisionMetadata` sentinel 的 absence marker 必须绑定 `cell_identity`、`revision = AbsentRevisionMetadata`、target revision、target shape digest、compatibility scope、storage watermark 和 writer fence id；如果它来自 creation epoch，`migration_edge_digest = None`，因为没有 source->target migration edge；如果它来自把 pre-metadata 数据 backfill 到 concrete revision/current revision 的操作，必须绑定对应 backfill marker id 和相关 edge digest。
- creation epoch 是 revision-presence store 中由 privileged deployment actor 写入的 cell activation proof，语义上等价于在 cell 首次可写入前对 reserved `StoredRevisionRef::AbsentRevisionMetadata` 写入 `RevisionPresenceMarker { state: Absent }`；它必须绑定 activation watermark、writer fence 和 compatibility scope，并且只能在 control-plane 能证明该 cell 在 watermark 前没有任何业务可写入路径时写入。
- storage field backfill marker 不能替代 cell-wide revision absence marker。
- runtime 查询 project/write/query/index safety 时，先取可能 stored revisions，再检查每个 revision 的 field operation coverage。

Proof store contract：

- `CurrentWriteCompatibilityRequirement` 只在 artifact metadata 中表达 required proof scope。
- proof store 由 privileged deployment actor 写入，service runtime 只读。
- 第一版 compatibility proof 只支持 privileged `OperatorAssertion`。
- `OperatorAssertion` proof 必须绑定 `cell_identity`、target revision、target shape plan digest、relevant migration edge digests、`assertion_id` 和 compatibility scope。
- 本地测试可以注入 in-memory proof store；production adapter 不能从 service code 接受 proof override。
- proof scope mismatch、target revision mismatch、target digest mismatch、edge digest mismatch、missing proof 或权限不足都等价于 no proof。

Gate store contract：

- `DurableRevisionGateId` 与 cell identity、target revision、target shape plan digest、relevant migration edge digests 和 compatibility scope 绑定。
- gate 由 privileged deployment actor 打开，service runtime 只读。
- gate state 是 durable monotonic state；第一版只支持 closed -> open。
- 本地测试可以注入 in-memory gate store，但 production adapter 不能从 service code 接受 gate override。
- gate scope mismatch、target digest mismatch、edge digest mismatch 或 gate missing 等价于 closed。

全局 shape digest invariant：

- 在同一 compatibility scope 内，`cell_identity + DurableShapeRevision` 必须唯一绑定一个 `current_shape_plan_digest`。
- deployment/proof/gate/marker store 必须拒绝同一 scope 内同 revision 不同 digest、或同 migration edge 不同 edge digest 的 artifact 集合。
- gate/proof/marker lookup 都必须同时匹配 target revision、target shape digest 和 compatibility scope；record 绑定 migration edge digests 时，还必须匹配 relevant migration edge digests。

示例：

```skiff
type User {
  id: string,
  displayName: string,
  normalizedName: string,
}
```

如果新增 `normalizedName` 并用 `where normalizedName == ...` 查询，read migration 不足以保证正确性。没有存储 `normalizedName` 的旧 rows 不会匹配 storage predicate。service 必须先 backfill，再启用该 query 或 index。

partial projection 规则：

- partial projection / partial update 必须由 adapter 内部读取 `__skiffDurableShapeRevision` 和必要 etag；metadata 不暴露给业务 projection result。
- 如果 storage adapter 无法在 partial path 中内部读取 revision metadata，相关 partial projection / partial update 必须 fail closed。
- projection 包含完整 durable cell 时，可以按 full read 规则运行 readonly migration 或 read-repair。
- projection 不包含完整 durable cell 时，不运行 migration、不 repair。
- 这类 projection 只允许返回具有 `Project` safety rule 的 selected fields。
- 如果 projected field 需要 migration 才能 materialize 为 current value，partial projection 必须以 stable projection migration error 失败；实现可以选择在 adapter 内部改走 full-cell read，但那已经不是 partial path，必须执行完整 durable read 语义和权限/repair 规则。

## DB Object Field Versus Recoverable Envelope

DB stored field 在语义上是 recoverable：写入 DB 的值必须能跨 request boundary 恢复。

但它们并不都以 recoverable envelope 存储：

- schema-projectable lane 将 plain data 存为 DB canonical storage，并支持 projection / where / order / index。
- recoverable-envelope lane 将 behavior-bearing 或 adapter-bearing value 存为 opaque envelope。

因此：

```text
DB stored field 有 recoverable requirement。
DB stored field 不等同于 RecoverableEnvelope。
```

Durable schema evolution 适用于两条 lane，但 adapter 不同：

- schema-projectable DB document migration 升级 BSON/document shape。
- recoverable-envelope migration 升级 envelope node/state shape。

## Cell Composition And Repair Scope

同一个 DB document 可以同时包含 document-level migration 和 top-level recoverable-envelope field migration。执行顺序必须固定：

1. full document read 先运行 `DbDocumentCell` migration，升级 schema-projectable document shape 和 document-level revision。
2. materialize 某个 recoverable-envelope field 时，再运行对应 `DbFieldEnvelopeCell` migration。
3. 如果 read path 同时需要 document repair 和 field envelope repair，必须在同一个 document etag / transaction boundary 下提交，或按 document repair 后 reload field envelope 的方式串行化，不能基于过期 etag 写回。

`DbDocumentCell` validation 只校验 schema-projectable fields、reserved document metadata 和 envelope placeholder 是否存在/类型正确。它不能要求 envelope field 已经是 current wrapper shape；legacy binary envelope 和 current wrapper envelope 都是 placeholder 的合法 physical representations。具体 envelope bytes/wrapper 的 migration 和 validation 属于对应 `DbFieldEnvelopeCell`。

write composition：

- full DB document insert / replace / change block commit 必须先运行 document cell writer，再对所有写入的 top-level recoverable-envelope fields 运行对应 field envelope writer。
- document cell gate/proof 和所有 affected envelope field cell gate/proof 必须在同一 operation 中检查。
- 如果任一 cell writer 失败，整个 DB write 必须原子失败，不得只写 document revision 或只写 envelope wrapper。
- 成功写入时，document revision metadata、schema-projectable fields、envelope placeholders 和 envelope wrappers 必须在同一 CAS / transaction boundary 内提交。

repair scope：

- `DbDocumentCell` repair 只更新 document-level revision、schema-projectable fields 和 adapter metadata，不得重写 opaque envelope bytes，除非 migration step 的 affected fields 明确包含该 envelope field。
- `DbFieldEnvelopeCell` repair 只更新对应 top-level field wrapper 和 field-level durable shape revision。
- 两类 repair 都必须保留 adapter 未建模但当前 storage 需要保留的 metadata。

## Handling Missing Required Fields

历史 durable shape 缺失 required field，不等于当前业务字段 nullable。

decode 流程：

1. 识别 historical revision。
2. 运行 migration chain 到 current revision。
3. 校验 current shape 包含所有 required fields。
4. decode 当前类型。
5. 如果 migration 无法产出 required field，以 migration error 失败。

这意味着：除非 service 提供 migration plan，能够从 historical durable state 构造当前 required field，或外部 backfill 已经把旧数据重写成 current shape，否则不能声称兼容这些历史数据。

## Stable Error Taxonomy

第一版至少定义以下稳定错误分类：

- `durable_cell_registration_error`：缺少 `DurableCellPlan`、revision 与 shape digest 冲突、current write 缺 metadata。
- `durable_migration_configuration_error`：缺 migration chain、legacy detection 不匹配、missing required field 无 migration。
- `durable_migration_hook_violation`：hook 使用 forbidden capability、digest mismatch、output shape invalid 或 resource limit exceeded。
- `durable_projection_migration_error`：partial projection/update 缺 revision metadata、缺 `Project` / `Write` safety rule 或无法内部读取 observed revision。
- `durable_storage_safety_error`：query/order/index/unique 缺 safety coverage、marker mismatch 或 mixed revision coverage 不完整。
- `durable_revision_gate_error`：`Gated` current write gate closed、gate scope/digest mismatch。
- `durable_current_write_proof_error`：`RollbackCompatible` current write 缺 privileged proof、proof scope/digest mismatch 或权限不足。
- `durable_repair_gate_error`：`Required` repair gate closed、repair gate scope/digest mismatch。
- `durable_repair_error`：required repair storage write unavailable、重复 CAS conflict 或 repair handle invalid。

## Reserved Metadata Handling

`__skiff*` namespace 是 storage metadata namespace。

规则：

- 新 schema 不允许声明 `__skiff*` 用户 stored field。
- legacy document 如果已经存在同名 user field，adapter 不能静默覆盖或解释为 metadata，第一版必须报 stable migration configuration error。service 必须先用语言外 backfill/rename 工具修复该 collision，再启用语言内 durable migration。
- business materialization 永不暴露 reserved metadata fields。

## Example: HostProviderMount.currentDirectory

期望的当前类型：

```skiff
type HostProviderMount {
  threadId: string,
  threadToolProviderMountId: string,
  slotIndex: integer,
  toolProviderId: string,
  providerId: root.api.agine.ProviderId,
  nameSnapshot: string,
  qualifierSnapshot: string,
  actionAllowlist: Array<string>,
  currentDirectory: string,
  active: bool,
}
```

如果历史 durable state 缺少 `currentDirectory`，只有当它能仅依赖历史 state 算出该值时，语言层 migration 才成立。如果正确值需要 live host metadata、当前进程 cwd 或 DB lookup，语言层 migration hook 无效。

因此 Agine 应该：

- 收紧新写入路径，确保 mount config 和 runtime bindings 包含非空 `currentDirectory`；
- 通过 service-specific 外部 backfill 或可查询 host metadata 的显式业务 repair 修复旧数据；
- 然后再把内部 `HostProviderMount.currentDirectory` 改成 `string`；
- 外部 host protocol input 可在合适位置继续 nullable，但必须在构造 durable internal state 前于边界处 normalize/reject。

## Implementation Phases

### Phase 0: Documentation And Metadata Shape

文件：

- `doc/architecture/recoverable-value.md`
- `doc/reference/db.md`
- `doc/reference/static-semantics.md`
- `runtime/model/src/recoverable.rs`
- 包含 recoverable metadata 的 artifact model crates

任务：

1. 文档化 `recoverable != repairable`。
2. 文档化 durable shape revision，并禁止把 codec schema version / service version / build id 当作 shape revision。
3. 文档化 current durable write、readonly migration、read-repair policy 和 rollback/write gate。
4. 增加 durable cell plan、hook、legacy detection、storage field safety、backfill marker、current write policy 和 repair policy metadata structs。
5. 增加 shared durable cell write policy evaluator，统一处理 current write gate 和 rollback-compatible proof lookup。
6. 增加 production durable evolution state store，包括 gate store、proof store、marker store、revision-presence store、privileged write API、runtime read API 和 local test injection。
7. 增加 deployment/control-plane compatibility scope 枚举：active routes、loaded workers、queue handler bindings、spawn target bindings 和 rollback candidates。
8. 增加 gate/proof privileged write 校验，写入前检查 compatibility scope、target revision、shape digest、适用时的 migration edge digest 和 operator assertion。
9. 增加 rollback target 拦截点，拒绝回滚到不兼容已打开 gate 的 artifact。
10. 增加 migration declaration 的最小 compiler 入口，并生成 artifact metadata。
11. 增加 hook pure/effect 校验，确保 hook 在 empty capability environment 下 typecheck。
12. 增加校验，确保 migration chain 确定且无环。
13. 增加校验，确保 `StoredCompatibleAcrossRevision` 可由 shape plan / storage encoding / operation semantics 证明。
14. 增加校验，确保 affected storage fields 的 projection/query/index 使用都有 explicit safety rule。
15. 增加校验，确保 declared affected fields 覆盖 compiler 计算出的 storage field diff。

### Phase 1: Recoverable Envelope Write And Readonly Migration

文件：

- `runtime/boundary/src/recoverable.rs`
- `runtime/service-db/src/mapping.rs`
- `runtime/eval/src/recoverable_behavior.rs`

任务：

1. 增加 `DurableMigrationRunner` interface。
2. 增加 capability-free hook evaluator frame，并在 runner 中校验 hook digest / shape plan digest。
3. 增加 envelope write adapter，所有 current envelope writes 先通过 shared write policy evaluator；`Gated` gate 无效或 `RollbackCompatible` proof 无效时不得写 wrapper/current revision，二者不能互相替代。
4. 增加 envelope read adapter，可读取 legacy binary-only envelope 和 wrapper envelope。
5. 在 expected type precheck 前应用 readonly migration。
6. 对未迁移的 current revision 保持当前 durable DB policy。
7. 增加测试：current envelope write 不能缺 revision metadata，wrapper 可 roundtrip。
8. 增加测试：`Gated` current envelope write 在 gate closed 时失败、gate open 时成功，且 proof 不能绕过 gate；`RollbackCompatible` current envelope write 在 proof missing / mismatch 时失败、valid proof 时成功，且 gate 不能替代 proof。
9. 增加测试：历史 envelope 缺 required field，但 migration 能产出该字段。
10. 增加测试：缺 required 且没有 migration 时以稳定 migration error 失败。
11. 增加测试：hook 尝试使用 forbidden capability 时失败。

### Phase 2: DB Document Write And Readonly Migration

文件：

- `runtime/service-db/src/mapping.rs`
- `runtime/service-db/src/capability.rs`
- DB boundary metadata modules

任务：

1. 确保 DB insert / replace / change block commit 写入 reserved current revision metadata。
2. DB write adapter 复用 shared write policy evaluator；`Gated` gate closed 时拒绝会写 current revision 的普通 write，`RollbackCompatible` proof 缺失或不匹配时拒绝会写 current revision 的普通 write。
3. 确保 full DB document read 包含 reserved revision metadata。
4. 当 metadata 缺失且 plan 声明了 matching legacy rule 时，识别 legacy revision。
5. 在 business value materialization 前运行 document migration。
6. partial projection 没有完整 cell 时不运行 migration，只允许有 `Project` safety rule 的 selected fields。
7. 增加 storage field safety guard，拒绝缺少 safety rule 的 project/where/order/index/unique usage。
8. 增加 `Write` safety guard；缺少 rule 的 partial update 必须 full materialize + migrate + CAS，或失败。
9. 增加 backfill marker 和 revision-presence lookup，marker scope / revision / index mismatch 时按 unsafe 处理。
10. 增加 mixed absent/v1/v2 revision 数据集上的 query/index guard 测试。
11. 增加 document migration 与 envelope field migration 同时存在时的执行顺序测试。
12. 增加新增 required schema-projectable field 的测试。
13. 增加测试证明 `where` 不使用 read migration，query/index 字段需要 backfill marker。

### Phase 3: Read-Repair For DB Cells

文件：

- `runtime/service-db/src/capability.rs`
- `runtime/service-db/src/mapping.rs`
- Mongo adapter layer

任务：

1. 为 full document 和 field envelope read 引入 `RepairHandle`。
2. 实现 deployment/runtime `DurableRevisionGateId` 查询，并在 gate closed 时按 `BestEffort` / `Required` policy 处理。
3. 实现 current shape 和 revision 的 CAS / transaction writeback。
4. CAS conflict 时 reload 并重试一次；重复 conflict 时 `BestEffort` 降级 readonly，`Required` 失败。
5. 确保 repair write merge/preserve unknown storage metadata，不做 blind full replace。
6. 增加 rollback gate telemetry。
7. 增加 telemetry counters：
   - `durable_migration_readonly_total`
   - `durable_migration_repair_attempt_total`
   - `durable_migration_repair_success_total`
   - `durable_migration_repair_conflict_total`
   - `durable_migration_failure_total`
   - `durable_migration_repair_gate_closed_total`
8. 增加 successful repair、CAS conflict、gate closed、write unavailable、rollback gate 和 projection no-repair 测试。

### Phase 4: Spawn / Queue Payload Write And Migration

文件：

- runtime spawn payload modules
- queue runtime modules
- platform work items 的 transport / storage metadata

任务：

1. 给 platform-owned work item 增加 payload shape revision metadata。
2. spawn create / queue enqueue 必须写 current payload shape revision；`Gated` gate closed 时拒绝 incompatible current payload write，`RollbackCompatible` proof 缺失或不匹配时拒绝 incompatible current payload write。
3. 在 dispatch 前应用 readonly migration。
4. 只允许在 claim/dispatch completion 前 repair，且同样受 `DurableRevisionGateId` 控制。
5. current revision 相对于 target binding / handler expected payload 定义，而不是全局最新 service shape。
6. 增加 current payload write roundtrip、pre-dispatch repair、gate closed 和 already-delivered no-repair 测试。

## Worktree And Merge Plan

建议用独立 worktree 承载实现，避免长周期 runtime/schema 改动污染 `main` worktree：

```bash
git worktree add ../skiff-durable-schema-evolution -b durable-schema-evolution
```

子任务与 worktree 关系：

- 同一个 implementation worktree 内按 phase 提交，保持每个 commit 可 review、可回滚。
- Documentation/metadata phase 完成后先提交，作为 runtime tracks 的共同依赖。
- Recoverable envelope 和 DB document write/read tracks 可以在同一 worktree 内串行落地，或拆分短期分支后合并回 `durable-schema-evolution`。
- Read-repair 与 spawn/queue tracks 依赖 runner API 稳定后再开始，避免平行实现复制临时接口。
- Phase commit 可以用于本地 review，但不能发布会写 current revision 的 adapter，除非 shared durable cell plan registration、write policy evaluator、gate store 和 proof store 已经同时可用。

收尾：

- 验收通过后，将 `durable-schema-evolution` 合并回 `main`。
- 删除已合并 worktree 和临时分支。
- push 必须等维护者明确要求。

## DAG Task Breakdown

可并行 tracks：

1. Documentation track:
   - 更新 architecture/reference docs。
   - 定义术语和安全规则。

2. Artifact metadata track:
   - 增加 cell plan、hook、field safety、backfill marker、current write policy、repair policy structs。
   - 增加 shared write policy evaluator、gate/proof/marker store interface。
   - 增加 chain、effect、field safety、affected-field completeness、cell registration validation 和 serialization tests。

3. Recoverable envelope write/read runtime track:
   - 增加 migration runner interface。
   - 增加 capability-free hook evaluator。
   - 集成 current wrapper write 和 recoverable decode。
   - 增加 unit tests。

4. Control-plane durable evolution store track:
   - 增加 production gate/proof/marker/revision-presence store。
   - 增加 privileged write API、active scope enumeration、activation admission 和 rollback target rejection。
   - 增加 marker writer fence 和 storage watermark handling。

5. DB document write/read runtime track:
   - 增加 current write revision metadata handling。
   - 增加 full-document migration。
   - 增加 projection/write/query/index guard tests。

6. Read-repair track:
   - 增加 repair handle、repair gate 和 CAS writeback。
   - 增加 telemetry、rollback gate 和 conflict tests。

7. Spawn / queue write/read track:
   - 增加 payload current write revision metadata。
   - 增加 pre-dispatch migration。

依赖：

- Documentation 和 metadata track 必须先于 runtime integration 落地。
- Control-plane durable evolution store track 必须先于任何会写 current revision 的 adapter 发布。
- metadata 落地后，Recoverable envelope 和 DB document write/read migration 可以并行推进。
- Read-repair 依赖 readonly migration。
- Current write gate 和 read-repair gate 依赖 deployment/runtime gate metadata，但不依赖语言外 batch tool。
- Spawn / queue 依赖 migration runner，但不依赖 DB adapter。

## Testing Strategy

### Unit Tests

- Migration chain selection：
  - 选中 exact chain；
  - missing chain 失败；
  - ambiguous chain 失败；
  - cycle 被拒绝。
- Pure hook validation：
  - stored historical value 先按 `input_shape_plan` decode/validate，再调用 hook；
  - corrupt historical input、unknown union branch、corrupt envelope bytes 在 hook 前失败；
  - 非 `pure function` hook 被拒绝；
  - hook 调用非 pure 函数被拒绝；
  - hook 使用 DB / host / clock / random / telemetry capability 被拒绝；
  - hook digest / shape plan digest mismatch 被拒绝；
  - output 通过 target shape 校验；
  - migration 后仍缺 required 时失败；
  - unknown output fields 遵循 current target policy。
- Revision handling：
  - 使用 explicit revision；
  - absent revision 映射到已配置 legacy revision；
  - absent revision 且无 legacy config 时失败。
- Durable cell registration：
  - zero-step cell 有 current revision、shape digest 和 write policy 时可读写；
  - compiler 为没有 explicit migration declaration 的 durable cell 自动生成 zero-step plan；
  - pre-metadata 数据没有 explicit `AbsentRevisionMetadata` rule 时不能被 zero-step plan 当作 current；
  - zero-step cell 缺 creation epoch / revision absence marker 时，current-to-current baseline 不能放行可能覆盖 pre-metadata 数据的 query/index/partial projection；
  - zero-step cell 有 creation epoch 或 `RevisionPresenceMarker { state: Absent }` 时，可以把 possible stored revisions 收敛为 current-only；
  - durable write path 缺 `DurableCellPlan` 时失败；
  - 同一 revision 绑定不同 shape digest 时失败。
- Storage field safety：
  - compiler 计算出的 storage field diff 未被 declared affected fields 覆盖时失败；
  - affected field 默认不可用于 project/write/where/order/index；
  - current-to-current zero-length path 只在 possible stored revisions 为 current-only 时使用 baseline safety，不阻塞已证明 current-only 的 query/index；
  - explicit `StoredCompatibleAcrossRevision` rule 放行；
  - 无法由 shape plan / storage encoding / operation semantics 证明的 `StoredCompatibleAcrossRevision` rule 被拒绝；
  - `ExternalBackfillComplete` 缺 marker 时失败，有 marker 时放行；
  - marker cell / field / operation / edge / edge digest / watermark / writer fence / index identity mismatch 时失败；
  - revision absence marker 缺失或 scope / edge digest / watermark / writer fence mismatch 时，不能证明 historical revision 已不存在。
- Current write policy：
  - current write 缺 revision metadata 失败；
  - `RollbackCompatible` 缺 operator assertion proof 或 proof 不匹配时失败；
  - `RollbackCompatible` operator assertion proof 有效时可直接写 current revision；
  - `RollbackCompatible` 即使 gate open 但 proof 缺失也失败；
  - `Gated` write 在 gate closed 时失败，在 gate open 时写 current revision；
  - `Gated` 即使存在 unrelated proof 但 gate closed 也失败。
- Durable evolution state store：
  - gate/proof/marker production store record scope mismatch 时失败；
  - active live artifact/worker 不兼容时 gate/proof 不能放行；
  - active old writer 仍可能写入 covered old revision 时，backfill/revision-absence marker 写入被拒绝；
  - compatibility scope 内同 revision 不同 shape digest 被拒绝；
  - compatibility scope 内同 migration edge 不同 edge digest 被拒绝。

### Recoverable Codec Tests

- Current recoverable-envelope write 输出 wrapper 和 current revision。
- recoverable-envelope 内部 logical fields 不能声明 DB storage field safety。
- 无 migration 的 current recoverable-envelope cell 通过 zero-step plan roundtrip。
- Historical envelope 缺 required field 时可迁移到 current shape。
- Historical envelope 带 unknown fields 时，只按声明规则迁移/忽略。
- Current envelope 仍使用严格 current expected type validation。
- 缺 required field 且没有 migration plan 时，以稳定错误失败。

### Service DB Tests

- DB insert / replace / change block commit 写入 current document revision metadata。
- 无 migration 的 DB document cell 通过 zero-step plan 写入并读取 current revision。
- Gate closed 时 current-incompatible DB write 失败，不写 storage。
- Full document read 运行 readonly migration。
- Partial projection 不 repair、不运行 migration，并且只能返回有 `Project` safety rule 的 selected fields。
- Partial update 在 observed revision 缺少 `Write` safety rule 时，必须 full materialize + migrate + CAS，或失败。
- Mixed absent/v1/v2 revision 数据集上，缺少完整 revision coverage 的 query/order/index 失败。
- Recoverable-envelope field wrapper 可 roundtrip。
- 配置 legacy revision 时，legacy binary envelope 可迁移。
- Full DB document write 包含 envelope field 时，document cell 和 envelope field cell writer 在同一 transaction/CAS 中原子成功或失败。
- Document migration 与 envelope field migration 同时存在时按固定顺序执行，且 repair scope 不互相覆盖。
- Read-repair 在 gate open 时写入 current revision。
- Gate closed 时 `BestEffort` 降级 readonly、`Required` 失败。
- CAS conflict 重试一次；重复 conflict 时按 policy 降级或失败。
- Repair write 保留 unknown stored fields 和 reserved metadata。
- 涉及 migrated field 的 project/write/query/order/index 需要 storage field safety metadata，否则失败。

### Spawn / Queue Tests

- spawn create / queue enqueue 写入 current payload revision。
- gate closed 时 incompatible payload write 失败，不创建 work item。
- Pre-dispatch payload migration 成功。
- claim 前且 gate open 的 payload repair 写入 current revision。
- gate closed 时 payload repair 按 policy 降级或失败。
- 已交付 item 不 repair。
- Current revision 从 target binding 选择，而不是全局最新 artifact。

### Integration Tests

- 带 DB object v1 数据的 service 部署 v2 后，通过 readonly migration 支持新增 required field。
- 同一 service 启用 read-repair 后，第一次 full read 会升级 stored document。
- 对新迁移字段发起 query 的 service，在 metadata 标记 storage backfill complete 前被拒绝。
- 带 historical durable state 的 recoverable `any I` self payload 会在 interface restore 前迁移。
- `Gated` gate closed 时没有普通 current write 落盘，`RollbackCompatible` proof 无效时没有普通 current write 落盘，repair gate closed 时没有 read-repair 落盘，因此可以回滚到旧 artifact。
- gate open、proof 写入或 repair gate open 并发生普通 current write / repair 前，所有 active live artifact/route/worker/queue handler 和 rollback candidates 必须兼容 target revision + shape digest + migration edge digest。
- gate/proof 授权的 ordinary current write 或 repair 发生后，部署系统拒绝回滚到不能读取 current revision 的 artifact，或 forward-compatible reader 成功读取。
- rolling deploy 中存在旧 route/worker/queue handler 时，gate/proof 写入被拒绝，直到 scope 内 active artifacts 都兼容或被移除。

## Acceptance Criteria

- Service 可以通过最小 migration declaration 给 durable shape 增加 required field，并提供 pure migration hook，使当前业务代码看到非空 current value。
- 历史数据缺 required field 且无 migration plan 时，以稳定 migration configuration error 失败，而不是 generic decode error。
- 非 pure hook、使用 forbidden capability 的 hook、hook digest mismatch 或 output shape 不匹配都会被稳定拒绝。
- nullable fields 仍表示语义 nullable fields，不表示历史兼容标记。
- DB stored fields 仍分为 schema-projectable 和 recoverable-envelope 两条 lane。
- storage field safety 只适用于 schema-projectable DB document lane；opaque envelope 内部字段不能参与 DB storage projection/query/write safety。
- Current writes 必须写入 current revision metadata；缺 metadata 的 current write 是稳定错误，不能被 legacy absent rule 接受。
- 每个 durable cell 必须有 `DurableCellPlan`；没有 migration 的 cell 必须使用 zero-step plan，缺 plan 时读写稳定失败。
- compatibility scope 内 `cell_identity + DurableShapeRevision` 必须唯一绑定 shape digest，且同一 migration edge 必须唯一绑定 edge digest；gate/proof/marker 必须匹配 revision、shape digest 和 edge digest。
- Read migration 不能让旧 rows 因新增字段而变得可查询；涉及 affected fields 的 project/write/where/order/index/unique usage 必须有 explicit safety rule 和必要 marker。
- 没有 full durable cell 和显式 repair handle 时，read-repair 永不运行。
- 所有 repair write 都必须由 CAS / transaction 保护、保留未受影响 storage fields，并且可观测。
- `Gated` current write / read-repair gate closed 时不会写 current revision；`RollbackCompatible` current write 只有 valid privileged operator assertion proof 时可写 current revision。任何 path 写入 current revision 后，都不能回滚到无法读取 current revision 的 artifact，除非有 forward-compatible reader 或语言外数据回滚。
- gate/proof 写入必须经过 deployment/control-plane scope 检查，覆盖 active live artifacts/routes/workers/queue handlers 和 rollback candidates。

## Risks And Mitigations

- 风险：migration hook 变成隐藏业务逻辑。
  - 缓解：compiler 在 empty capability environment 下校验 hook，runtime 在 capability-free evaluator 中执行，并用 digest/shape plan 校验防止漂移。

- 风险：read-repair 改变 read operation 的副作用。
  - 缓解：把 repair 建模为平台维护副作用，要求显式 policy、repair gate、telemetry、CAS，且不具备业务可见 update 语义。

- 风险：普通 current write 或 read-repair 后代码回滚失败。
  - 缓解：`Gated` write gate closed 时不写 current revision；`RollbackCompatible` write 只接受 privileged operator assertion proof；gate open、proof 写入或 repair gate open 前要求 active live artifacts/workers 和 rollback candidates 全部兼容 target revision + shape digest + migration edge digest，任何 path 写入 current revision 后部署系统拒绝回滚到不能读取 current revision 的 artifact。

- 风险：`RollbackCompatible` 被错误声明后绕过 gate。
  - 缓解：第一版只接受 privileged `OperatorAssertion` proof；proof 缺失、scope mismatch 或权限不足时 `RollbackCompatible` current write 失败，且 proof 对 `Gated` current write 无效。

- 风险：query/index 语义看起来兼容但实际不兼容。
  - 缓解：storage field safety 默认 false；迁移字段上的 projection/write/query/index 必须有 field/operation/migration-edge 级 safety rule 和必要 backfill marker。

- 风险：revision metadata 与用户字段冲突。
  - 缓解：保留 `__skiff*` metadata namespace，并在适用位置拒绝使用该前缀的用户 stored field。

- 风险：migration chain 意外依赖 current build。
  - 缓解：durable shape revision 必须显式且稳定；migration hook 由 artifact metadata 选择，但 input/output revision 不是 build id。

## Follow-Up Work

- 稳定最终的 migration declaration 公开语法和 IDE 体验。第一版必须已有 internal/unstable 最小声明入口。
- 定义语言外 batch migration tool interface。
- 定义 index backfill 和 storage field safety markers 的 operator runbook。production store contract 和 privileged write API 已在第一版定义。
- 增加 outstanding legacy revision counts 的 admin observability。
- 后续评估 schema-projectable DB fields 是否需要 field-level revision metadata。第一版已经明确只使用 document-level revision。
