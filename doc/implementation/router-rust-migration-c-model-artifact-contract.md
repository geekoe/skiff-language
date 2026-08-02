# Router Rust Migration C-model-artifact：artifact model 消费边界契约

日期：2026-08-02
状态：frozen（contract pack；供 W-model-artifact / W-artifact 直接消费，不写 production）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §2.2（三类 model 必须分开）、
  §2.3（shared Cargo closure）、§3.3（`RoutingEpoch` 的 artifact 输入）、§3.8（bounded
  blocking store）、§5.3（C-model-artifact → W-model-artifact → M-artifact）、§5.4、
  §7 E-bootstrap。冲突时以权威设计为准。
- 批次文档：`doc/implementation/router-rust-migration-batch-3.md`。
- 叶子：`doc/implementation/router-rust-migration-contracts-bootstrap-leaf.md`。

## 1. 冻结范围

本 pack 冻结 compiler/Router/Runtime 的 artifact model **消费边界**：

- artifact identity 的 owner 与校验链（声明 identity、内容计算 identity、record path）；
- strict reader boundary（谁可以读、读什么、读失败怎么终止）；
- §2.2 三类 model 分类的消费面（wire model / artifact model / durable activation model），
  以及 Router consumer 不得跨越的依赖边界；
- strict loader 的容量/超时/关闭契约（§3.8、§7 E-bootstrap 的 loader saturation）。

非目标：不实现 W-model-artifact/W-artifact；不写 deployment/artifact-model production；
不定义 `RoutingEpoch`（归 C-bootstrap）；不定义 A0 actor projection。

## 2. 冻结 owner（全部已存在，引用不重写）

| 面 | owner crate | 冻结事实 |
| --- | --- | --- |
| artifact DTO | `skiff-artifact-model` | `RuntimeAssembly`、`RuntimeAssemblyRef`、`RuntimeConfigSnapshotRef`、`RuntimeConfigSnapshotId`；Deserialize 校验 + `deny_unknown_fields` |
| identity 计算 | `skiff-artifact-identity` | `runtime_assembly_identity`（content projection → sha256）、`runtime_assembly_ref`、`validate_runtime_assembly_identity`、`validate_runtime_assembly_surface`、`runtime_assembly_identity_hash` |
| record 路径 | `skiff-artifact-identity` | `RuntimeAssemblyRecordPath`（`records/runtime-assemblies/<hash>.json`）、`EnvironmentActivationStatePath`、`ArtifactRelativePath` |
| assembly strict reader | `skiff-deployment::storage::CanonicalArtifactStore` | `write_runtime_assembly`（write-once immutable）、`read_runtime_assembly`（四重校验，见 §3） |
| snapshot store / resolver | `skiff-runtime-config-snapshot` | `RuntimeConfigSnapshotStore`（publish write-once / strict read）、`RuntimeConfigSnapshotResolver`（backend-neutral trait） |
| wire 消费 | `skiff-runtime-transport` | 只消费 `RuntimeAssemblyRef`/`RuntimeConfigSnapshotRef` 的 typed ref，不读 raw record |

## 3. Strict reader boundary（不可绕过）

`read_runtime_assembly(reference)` 每次读取执行完整校验链：

1. `RuntimeAssemblyRecordPath::new(reference)` 由 ref 推导 record path（identity hash）；
2. 文件存在（missing → fail closed）；
3. bytes 解析为 strict JSON（malformed → fail closed）；
4. raw `assemblyIdentity` 与 reference 精确相等（path/declared identity 一致）；
5. typed Deserialize 为 `RuntimeAssembly`（unknown field → fail closed）；
6. `validate_runtime_assembly_identity`：内容重新计算 identity 并与 declared identity
   相等（identity mismatch → fail closed）；
7. `runtime_assembly_ref(&assembly) == reference`（exact ref 一致）；
8. `ensure_canonical`：bytes 与 canonical JSON 完全一致（非 canonical → fail closed）。

`RuntimeConfigSnapshotStore::read(reference)` 同族：schemaVersion、ref 校验、size 上限
（`MAX_CONFIG_SNAPSHOT_BYTES = 16 MiB`）、id/path mismatch 拒绝、canonical bytes 校验。

Invariant：**任何 consumer 不得绕过 strict reader**——不允许 TS mirror/hash、raw JSON
reader、路径猜测、partial record 消费或 alias/fallback；identity 不可由调用方传入未校验
字符串后“自行解释”。同步 reader 只能经 bounded `spawn_blocking` 池调用（§3.8），该池
契约归 C-bootstrap/W-bootstrap 实现。

## 4. §2.2 三类 model 分类消费面

### 4.1 Router↔Runtime wire model（`skiff-runtime-transport` + request-contract）

- 消费 `RuntimeAssemblyRef`/`RuntimeConfigSnapshotRef` 作为 **opaque typed refs**；
- 不得内嵌 durable record、Mongo doc、config snapshot 全文或 RuntimeAssembly 全文；
- Runtime 不消费 activation durable DTO（`EnvironmentActivationState` 等）；
- Router consumer 不得直接/传递依赖宽 `skiff-runtime-model`、runtime-host、eval、
  request execution（M0 gate 已建立，本包继续冻结）。

### 4.2 compiler/Router/Runtime artifact model

- compiler/deployment 写入 immutable records（`write_runtime_assembly`/`publish`），
  Router/Runtime 只经 strict reader 读取；
- Router 的 `RoutingEpoch` 构造输入 = `(RuntimeAssemblyRef, RuntimeConfigSnapshotRef)`
  及其 strict-loaded 内容（C-bootstrap 冻结加载顺序）；
- Runtime 经 bootstrap wire 收到的只是 refs，实际 assembly 内容由 Router
  strict-loaded 后经 epoch 投影消费，Runtime 不自行读 artifact filesystem。

### 4.3 Router/platform durable activation model（`skiff-deployment`）

- `CommittedActivation`/`PendingActivation`/`EnvironmentActivationState` 是 durable
  DTO，owner 为 deployment/persistence lane；
- 只被 Router 与 deployment tooling 消费；不是 Runtime wire contract；
- contracts-bootstrap 只读 committed 面（C-bootstrap），完整 recovery/transaction 归
  C-router-activation-state/contracts-activation。

## 5. §5.4 pack 必填项

### 唯一 owner / invariant

- owner：`skiff-artifact-identity`（identity）+ `skiff-deployment`（assembly strict
  reader）+ `skiff-runtime-config-snapshot`（snapshot strict reader）+
  `skiff-artifact-model`（DTO）。
- invariant：**identity 与内容不可分离**——任何读取路径必须同时满足 §3 校验链；
  任何校验失败 fail closed，不产生 partial `RoutingEpoch`。

### Typed inputs / outputs

- input：`RuntimeAssemblyRef` / `RuntimeConfigSnapshotRef`（已校验 typed）。
- output：`Arc<RuntimeAssembly>` / `RuntimeConfigSnapshot`（已校验 typed）；错误为
  `StorageResult` / `RuntimeConfigSnapshotResult`，不返回裸 bytes/Value。

### Capacity / queue full

- sync reader 必须经 bounded blocking pool；契约默认并发上限 8（W-bootstrap 可配置化，
  默认值与 config schema 由 C-config 冻结的 `runtime` 语义对齐，不在此新增 config 字段）。
- pool 饱和 = `LoaderSaturated` fail closed，**不排队无限等待**；
  per-record byte budget：snapshot 16 MiB（现有常量），assembly 受 canonical JSON + u32
  record 上限约束。

### Timeout / disconnect / replacement / shutdown terminal

- 每次读取有 deadline（W-bootstrap 实现；默认 5s，超时 fail closed）；
- disconnect：reader 无 session 语义，N/A；
- replacement：records immutable，无替换；epoch replacement 是发布层语义（C-bootstrap）；
- shutdown：pool drain，在飞 read 在 deadline 内完成或 abort，shutdown 后新 read 拒绝，
  residue 归零。

### Health fields

- `blockingLoader.occupancy`（active/queued/reserved）、`blockingLoader.saturated`、
  `blockingLoader.deadlineAborts`、`bootstrapReader.failClosed{missing,malformed,
  identityMismatch,pending}`（后三者在 C-bootstrap 包冻结，W-bootstrap 实现 counter）。

### Fake seam

- `RuntimeConfigSnapshotResolver`（已存在 trait）：fake resolver 返回内存 snapshot；
- assembly side：`CanonicalArtifactStore` 以临时 root 作 fake filesystem（现有测试模式），
  契约定义 `BootstrapStrictLoader` port（C-bootstrap），接受 store 句柄 + resolver，
  fake 实现用于 W-bootstrap 测试。

### 至少一条真实边界 probe

- `bootstrap_artifact_reader_corpus.rs`：真实临时 root + `CanonicalArtifactStore`，对
  valid/missing/malformed/identityMismatch/nonCanonical/unknownField/schemaMismatch
  record 做真实文件边界断言；
- `bootstrap_snapshot_reader_corpus.rs`：真实临时 root + `RuntimeConfigSnapshotStore`，
  对 valid/missing/idMismatch/malformed/nonCanonical/schemaMismatch/unknownField 做真实
  文件边界断言。

## 6. 本 pack 新增 corpus

- `deployment/tests/fixtures/bootstrap-artifact-corpus.json`
  （schema `skiff-router-rust-bootstrap-artifact-corpus-v1`）：assembly refs 正负例 +
  assembly record cases（kind → expected outcome）。
- `runtime-config-snapshot/tests/fixtures/bootstrap-snapshot-corpus.json`
  （schema `skiff-router-rust-bootstrap-snapshot-corpus-v1`）：snapshot refs 正负例 +
  snapshot record cases。

## 7. W-model-artifact / W-artifact 交付义务（非本包实现）

1. 消费两套 corpus：全部 positive 通过、negative fail closed。
2. 实现 `BootstrapStrictLoader`（C-bootstrap 定义签名）并保持 §3 校验链不被绕过。
3. M-artifact gate：Router/Runtime/compiler 真实 consumer 直接消费同一 corpus；
  新增 artifact 类型必须注册 owner，不得复制 DTO/私有兼容层。
