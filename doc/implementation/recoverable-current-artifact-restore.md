# Recoverable 当前 artifact 恢复实现计划

本文是实现计划，不是长期架构契约。对应长期契约需要在实现前同步修订
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

`spawn` 的情况不同。`spawn` submit 控制面已经携带 build id，执行端应 claim 同一
build 的 artifact。payload 是同 build 内部短期传递，不需要在每个 local behavior
节点里重复写 artifact/build。

## 目标

1. owner-internal local behavior 的恢复使用**当前执行上下文 artifact/build**，而不是
   durable payload 中保存的 artifact/build。
2. durable payload 中不再写 `artifact_identity` / `build_id` 作为 local behavior
   的恢复 hard gate。
3. typed recoverable boundary 的 expected type plan 提供 interface/projection；
   `InterfaceValueState` 不再把 interface/projection 当 durable truth 重复保存。
4. local behavior self 只保存恢复当前值必需的稳定 concrete restore key 和 durable
   state。
5. 移除 local interface self wrapper 的 `restore_schema_version`。未来如需应用级迁移，
   必须由 concrete type 或 DB schema migration 显式定义，不能复用 runtime wrapper
   版本字符串。
6. DB recoverable-envelope lane 采用 durable decode 策略：对本方案上线后写入的 v2
   recoverable-envelope 记录，artifact/build mismatch 不再早于 schema 检查失败；不同
   build 写入的 v2 记录可以进入当前 expected type 的兼容判断。
7. `spawn` 保持 strict same-build 语义。它不因 DB 的 durable decode 策略而变成跨版本
   payload。

## 非目标

- 不开放 cross-service 或 external-untrusted behavior envelope。非 owner-internal 边界
  仍然拒绝 `InterfaceValue`、`NominalObject`、`LocalCode`、`NativeAdapter` 等
  behavior-bearing node。
- 不实现完整 DB schema migration、backfill、dual-write 或 read-repair 工作流。
- 不兼容旧 recoverable v1 bytes。Skiff 尚未发布，本地 dev/stable DB 里已由旧 build
  写下的 recoverable-envelope 字段可以删除或重建。
- 不让 `carrier = Remote` 的 `any I` 变成 durable value。
- 不设计 native adapter 的长期 schema compatibility。`NativeAdapter.adapter_schema_version`
  是另一条线，本文只处理 local artifact/build gate。
- 不保证新写入的 recoverable v2 bytes 能被旧 runtime 读取。Skiff 尚未发布，旧 runtime
  回滚需要清理或重写对应本地数据。

## 目标数据模型

新增 recoverable envelope v2。encoder 只写 v2；decoder 只接受 v2。旧 v1 bytes 一律以
unsupported recoverable schema/version 或 state invalid fail closed，不做 compatibility shim。

v2 的 local behavior 身份不再叫 `LocalCode`，避免误导为“定位某个历史 artifact 的代码”：

```rust
enum RecoverableCodeIdentity {
    None,
    LocalConcrete {
        concrete_type_identity: String,
        package: Option<PackageCoordinate>,
    },
    NativeAdapter {
        adapter_identity: String,
        adapter_schema_version: String,
        owner: NativeAdapterOwner,
        native_type_identity: String,
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

- `LocalConcrete.concrete_type_identity` 是 stable concrete restore key。它仍然需要存：
  恢复 `any I` 时 expected type 只说明“这里需要一个 `I`”，不知道历史值是
  `A implements I` 还是 `B implements I`。
- `LocalConcrete.package` 参与 lookup。`package = None` 表示当前 service artifact 内的
  concrete type；`Some(package)` 表示 concrete type 来自当前 linked artifact 的某个 package
  unit。`package` 必须是稳定 package coordinate，不得包含 package build id、source hash 或
  local path。lookup key 是 `(package, concrete_type_identity)`；若当前 linked program
  找不到该 package/concrete pair，decode fail closed。
- `artifact_identity` / `build_id` 从 local behavior durable state 中移除。当前 request
  的 linked program 和 method table 是唯一解释上下文。
- `InterfaceValueState.interface_identity` 和 `method_projection_identity` 从 durable state
  中移除。decode 时由 `RuntimeRecoverableExpectedTypeNode::AnyInterface` 提供，并用当前
  linked program 校验 `LocalConcrete.concrete_type_identity` 是否仍 conform。
- `restore_schema_version` 从 local interface self wrapper 中移除。envelope 自身的
  `schema_version` 仍保留，它只描述 recoverable 容器/二进制格式版本。
- 新写入绝不产生 `LocalCode`。旧 v1 `LocalCode { artifact_identity, build_id,
  concrete_type_identity, ... }` 不被解释为 `LocalConcrete`；旧 DB 数据由清理策略处理。

## 前置不变量

删除 artifact/build gate 之前，必须先完成 stable restore key 审计。该审计是实现前置
任务，不是风险项里的提醒：

- 审计 `concrete_type_identity` 的来源，证明它不包含 build id、artifact path、source
  hash、临时 runtime address 或本地文件路径。
- 审计 `package` coordinate 的来源，证明它是 package 语义坐标，不是 package build
  identity 或本地 package store 路径。
- 如果现有 `concrete_type_identity` 或 `package` 不稳定，先定义新的
  `LocalConcreteRestoreKey { package, concrete_type_identity }`，并让 v2 encoder 写新 key；
  不允许把不稳定旧 id 带进 v2。
- 增加 identity fixture/unit test：同一 service/package concrete type 在两个不同 build id 下，
  stable restore key 相同；不同 concrete type 或不同 package coordinate 不碰撞。

没有通过该审计时，不得进入 runtime/model 和 DB 集成实现。

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
2. 遇到 local `InterfaceValue` 时，要求 boundary 是 owner-internal，且 expected node 是
   `AnyInterface`。unresolved expected plan 不允许编码 behavior node。
3. `EvalRecoverableBehaviorHooks::encode_local_interface_self` 根据当前 linked program 找到
   `(interface, projection, concrete_type)` 的 method table entry。
4. 使用当前 artifact 中 `LocalConcreteRestoreKey { package, concrete_type_identity }`
   对应的 recoverable expected plan 编码 self payload。取不到 concrete self expected plan
   时，encode fail closed；不允许继续使用 `unresolved("local interface self")` 作为
   production 路径。
5. 写出 `self_node`：

```text
value_kind = NominalObject
code_identity = LocalConcrete { concrete_type_identity, package }
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
4. 遇到 `InterfaceValue` 时，从 expected type 取得 interface/projection。payload 中不再
   读取 wrapper interface/projection。
5. 校验 `self_node` 是 `NominalObject + LocalConcrete`；`LocalCode`、旧 wrapper 或旧
   `restore_schema_version` 一律不是 v2 合法 local self。
6. 用当前 linked program 查找 `concrete_type_identity` 的 restore plan，decode durable
   state 得到 concrete self。
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

## Spawn 策略

`spawn` 不跨版本。实现必须把“same artifact/build”约束放在控制面，而不是 recoverable
payload 内：

- `submit_spawn_statement` 已在 `SpawnSubmitControlRequest.build_id` 中提交当前 request
  build id。
- router/actor claim 必须把 spawned request 派发给同一 build 的 runtime。若找不到该 build，
  spawn claim/dispatch 在 payload decode 前失败。payload decode 不参与 build 校验，也不能
  作为 fallback。
- 控制面 enforcement 的实现点包括：
  - `runtime/eval/src/spawn_ops.rs`：submit 时必须继续填 `SpawnSubmitControlRequest.build_id`
    和 `activation_identity`。
  - `runtime/host/src/capability_context/actor.rs`：submit 透传 build id，不得丢弃。
  - `runtime/host/src/host/spawn_worker.rs`：claim response 构造 `RequestEnvelope` 时必须使用
    claim item 的 build id，并在进入 request runner 前确认该 build 已 loaded。
  - `runtime/host/src/host/route_registry.rs` / request dispatch：wrong build 或 missing build
    必须在 route lookup 阶段失败，不能落到 payload decode。
- spawn args payload 写 v2 recoverable bytes，不含 `artifact_identity` / `build_id`。
- spawn decode 使用 target executable 的当前 expected plan，policy 仍 strict。payload
  schema 不一致说明 control plane 路由到了错误 build 或 payload 损坏，应 fail closed。

需要补测试：构造 spawn args 中含 local `any I`，断言 canonical envelope 中没有
artifact/build 字符串；同 build decode 成功；普通 runtime binary decode 仍拒绝 recoverable
magic；cross-service/external trust 仍拒绝 behavior-bearing envelope。

## 实现 DAG

### A0. Stable restore key 审计

依赖：无。

改动：

- 审计 compiler/linker 产出的 concrete type identity 和 package coordinate。
- 若现有 identity 不稳定，先新增稳定 `LocalConcreteRestoreKey`，再让 v2 recoverable 写
  新 key。
- 增加不同 build id 下同一 concrete/interface schema key 相同、不同 package/concrete
  不碰撞的测试。

验收：identity/key 测试先于 B-F 通过；文档记录该 key 不含 build/source/path/runtime
address。

### A. 文档契约同步

依赖：A0。

改动：

- 更新 `doc/architecture/recoverable-value.md`：删除“行为值按写入时 artifact 恢复”的长期
  结论，改为 owner-internal local behavior 按当前 execution context 恢复。
- 更新 `doc/reference/spawn.md`：说明 spawn payload 不承载 artifact/build，same-build 是控制面
  约束。
- 更新 `doc/reference/any-interface-value.md`：说明 typed recoverable boundary 的
  interface/projection 来自 expected type，不来自 durable wrapper truth。

验收：文档不再同时声明旧模型和新模型。B-F 可以并行准备实现分支，但不能在 A 合入前
合入或作为完成状态验收。

### B. Recoverable model 与 canonical codec

依赖：A0。A 可以并行开始，但 B 的最终命名应与 A 一致。

改动：

- 在 `runtime/model/src/recoverable.rs` 增加 v2 schema 常量和 `LocalConcrete`。
- v2 encoder 写 `LocalConcrete`，不写 artifact/build。
- v2 `InterfaceValueState` 只写 `self_node`。
- v2 `NominalObjectState::Custom` 不写 `restore_schema_version`。
- decoder 只接受 v2；v1 或未知 envelope schema/version fail closed。
- `collect_artifact_refs` 不收集 `LocalConcrete`。

验收：model/boundary 单测覆盖 v2 roundtrip、v1/unknown schema decode 失败、新写入不含
artifact/build。

### C. Boundary codec 与 decode policy

依赖：B。

改动：

- `runtime/boundary/src/recoverable.rs` 的 `RecoverableBoundaryCodec::decode*` 增加
  policy-aware 内部入口，公开 strict 默认入口保持现有调用语义。
- `precheck_record_fields` 按 policy 处理 unknown fields。
- record decode 在 expected record 下 materialize missing nullable field 为 `Null`。
- plain/unresolved expected plan 不允许 decode behavior-bearing `InterfaceValue`。
- policy 默认值为 strict：`unknown_record_fields = Reject`、
  `materialize_missing_nullable_fields = false`。
- untrusted behavior scan 继续在 expected precheck 之前执行。

验收：strict payload 仍拒绝 extra field；DB durable policy 忽略 extra field；missing
nullable field decode 为 `Null`；missing required field 失败。

### D. Eval behavior hooks

依赖：B、C。

改动：

- `runtime/eval/src/recoverable_behavior.rs` 停止写
  `INTERFACE_SELF_RESTORE_SCHEMA_VERSION`。
- encode hook 写 `LocalConcrete`。
- restore hook 只接受 v2 `LocalConcrete`；不比较 artifact/build。
- restore hook 从当前 linked program 查 concrete restore expected plan，并用当前 method table
  registry 校验 conformance/projection。
- 错误文案从 “written by a different artifact/build” 改为当前 concrete type/projection 不可用
  或 durable state 不匹配。

验收：两个不同 build id 的 hook 对同一 concrete/interface schema roundtrip 成功；当前
program 中移除 concrete 或 projection 时稳定失败。

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

验收：focused service-db tests 通过。

### F. Spawn integration

依赖：B、C、D。

改动：

- `runtime/eval/src/spawn_ops.rs` 和 `runtime/eval/src/recoverable_spawn_payload.rs` 使用
  v2 behavior-aware recoverable payload。
- 保持 spawn decode strict policy。
- 在 `runtime/host/src/host/spawn_worker.rs`、`runtime/host/src/host/route_registry.rs`
  或对应 host tests 中覆盖 wrong-build claim/dispatch：错误 build 必须在 request route
  lookup 前后、payload decode 前失败。
- 增加 eval/host tests，确保 submitted build id 是 spawn 控制面字段，payload 中不含 local
  artifact/build。

验收：现有 spawn tests 通过；新增 local `any I` spawn payload 测试通过；wrong-build
claim/dispatch 不进入 payload decode。测试应使用会在 decode 时失败的 payload、panic
decode stub 或 decode counter 证明该路径没有调用 payload decode。

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

- Worker 1：B + C，负责 `runtime/model`、`runtime/boundary` 和对应单测。
- Worker 2：D + F，负责 `runtime/eval` 的 behavior hook 和 spawn tests。
- Worker 3：E，负责 `runtime/service-db` integration 和 DB tests。

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
- **LocalConcreteRestoreKey 不够稳定**：如果 `concrete_type_identity` 或 `package` 随
  rebuild 漂移，DB 仍无法恢复。缓解：A0 作为前置任务，未通过 key 稳定性测试不得进入
  runtime/model 实现。
- **旧 DB 数据读取失败**：v2 不兼容 v1，旧 DB 数据会 fail closed。缓解：本地清理受影响
  recoverable-envelope 字段/集合；需要保留业务事实时由应用层重建，不在 codec 中迁移。
- **放宽 record precheck 影响安全边界**：policy 必须显式传入；默认 strict；cross-service 和
  external-untrusted 仍先拒绝 behavior-bearing payload。
- **旧 runtime 回滚**：新 runtime 写 v2 后旧 runtime 不能读。Skiff 未发布，回滚策略是 revert
  code 后清理或重写本地受影响 DB field，不提供双写。

## 完成标准

- 新写入的 owner-internal local behavior recoverable bytes 不包含 artifact/build。
- `LocalConcreteRestoreKey { package, concrete_type_identity }` 经测试证明跨 build 稳定，
  且不包含 build/source/path/runtime address。
- typed interface wrapper 的 interface/projection 来自 expected type plan，不来自 durable payload。
- local interface self 不再写 runtime wrapper `restore_schema_version`。
- DB read 不再因 stored artifact/build 与当前 build 不同而失败。
- v1/旧 local self 不被任何 boundary 接受；旧 DB recoverable-envelope 数据按清理策略处理。
- spawn payload 不含 local artifact/build，same-build 约束由 spawn 控制面测试覆盖，wrong-build
  dispatch 在 payload decode 前失败，并由测试证明 decode 未被调用。
- DB schema mismatch 行为符合本文 v2 矩阵：新增 nullable 可读为 null，新增 required
  失败，删除字段可读，类型不兼容失败。
- 非 owner-internal behavior envelope 仍 fail closed。
