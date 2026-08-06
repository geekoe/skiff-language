# Router Rust Migration C-bootstrap：repository read port + projection + strict loader + epoch publication 契约

日期：2026-08-02
状态：frozen（contract pack；供 W-bootstrap 直接消费，不写 production）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.3（`ActiveRoutingEpochStore`
  唯一 authority）、§3.8（boundedness）、§4.2（cold recovery 只读 committed 前置）、
  §5.4（C-bootstrap + M-bootstrap-wire + M-artifact + P-activation-state →
  W-bootstrap + E-bootstrap）、§7 E-bootstrap（committed 只读、pending fail closed、
  missing/malformed/identity mismatch、loader saturation、shutdown fail closed）、§10
  （health counters）。冲突时以权威设计为准。
- 批次文档：`doc/implementation/router-rust-migration-batch-3.md`。
- 叶子：`doc/implementation/router-rust-migration-contracts-bootstrap-leaf.md`。
- 兄弟契约：`router-rust-migration-c-model-bootstrap-wire-contract.md`、
  `router-rust-migration-c-model-artifact-contract.md`。

## 1. 冻结范围

初始 bootstrap 链（E-bootstrap 前置）：

1. `CommittedActivationBootstrapReader`：repository **只读** port；
2. durable→shared projection：`CommittedActivation` → shared refs；
3. strict loader：refs → validated `RuntimeAssembly` + `RuntimeConfigSnapshot` →
   完整 `RoutingEpoch`；
4. `ActiveRoutingEpochStore` 初始发布契约（原子 `Arc` replacement）；
5. E-bootstrap 负例矩阵（committed 只读 / pending fail closed / missing / malformed /
   identity mismatch / loader saturation / shutdown fail closed）。

非目标：不实现 W-bootstrap；不实现 `RoutingEpoch`/`ActiveRoutingEpochStore` production 类型
（W-bootstrap 实现）；不定义 actor routing projection schema（A0 独占，这里只留 opaque
字段槽）；不定义 activation transaction/recovery（contracts-activation 独占）；不定义
`RuntimeBootstrapProvider` 实现（wire 契约已冻结签名）。

## 2. 冻结 port 与类型（契约定义，不写 production）

### 2.1 `CommittedActivationBootstrapReader`（read port，W-bootstrap 实现）

```text
read_committed(environment) -> BootstrapReadOutcome
```

`BootstrapReadOutcome`（closed enum，本包冻结语义）：

| outcome | 触发 |
| --- | --- |
| `StableCommitted { generation, assembly: RuntimeAssemblyRef, config_snapshot: RuntimeConfigSnapshotRef }` | committed record 存在且校验通过，pending 不存在 |
| `FailClosedPending { activation_id }` | committed 存在但 durable pending 存在（E-bootstrap 范围：**一律 fail closed**，不投影、不 stage） |
| `FailClosedMissing` | environment activation state record 不存在 |
| `FailClosedMalformed { message }` | record 非 canonical JSON / schemaVersion 错误 / 字段非法 |
| `FailClosedIdentityMismatch { message }` | committed ref 指向的 assembly record 缺失、identity mismatch 或 exact ref 不符 |

约束：

- **只读**：不写 activation state、不 CAS、不创建 record、不改变 pending；
- 消费既有 `CanonicalArtifactStore::read_environment_activation` +
  `validate_activation_references`（identity 校验由 store 完成），port 本身不复制校验；
- 每次调用独立校验，无缓存；调用方（W-bootstrap）不得绕过 port 直读 durable bytes。

### 2.2 durable→shared projection（纯函数）

```text
project_committed(CommittedActivation { generation, assembly, config_snapshot })
  -> CommittedBootstrapRefs { generation, assembly: RuntimeAssemblyRef,
                              config_snapshot: RuntimeConfigSnapshotRef }
```

- total：committed 存在即投影成功（store 已校验 refs）；不存在 committed → 无投影；
- pending 存在 → 无投影（fail closed），**绝不允许把 pending candidate 投影为 committed**；
- 输出与 C-model-bootstrap-wire 的 `RouterBootstrapActivationFrameHeader` 字段一一对应
  （environment 由调用方 environment 提供，generation/assembly/configSnapshot 来自投影）。

### 2.3 strict loader

```text
load_epoch(environment, CommittedBootstrapRefs)
  -> Result<RoutingEpoch, BootstrapLoadFailure>
```

顺序（W-bootstrap 实现，本包冻结顺序与 fail-closed 语义）：

1. `CanonicalArtifactStore::read_runtime_assembly(&assembly)`（§3 校验链）；
2. `RuntimeConfigSnapshotStore::read(&config_snapshot)`（或 `RuntimeConfigSnapshotResolver`
   resolve）；
3. 校验 snapshot 的 `environment()` 与调用方 environment 一致；
4. 构造完整 `RoutingEpoch`（含 ingress/deployment/actor projection 字段）；
5. 任一失败 → `BootstrapLoadFailure`，**不产生 partial epoch、不发布**。

loader 容量契约（§3.8）：sync reader 只经 bounded `spawn_blocking` 池（默认并发 8，
W-bootstrap 可配置）；池饱和 = fail closed（不排队无限等待）；每次 read 有 deadline
（默认 5s，超时 fail closed）；shutdown 时 drain 池、在飞 read 在 deadline 内完成或 abort，
shutdown 后新 load 拒绝。

### 2.4 `RoutingEpoch` 与 `ActiveRoutingEpochStore` 初始发布（§3.3）

`RoutingEpoch`（W-bootstrap production 类型；本包冻结字段语义）：

```text
RoutingEpoch {
  environment,
  assembly_generation,
  assembly_identity,        // 来自 RuntimeAssemblyRef
  config_snapshot_id,       // 来自 RuntimeConfigSnapshotRef
  immutable ingress/deployment/actor routing projection
}
```

- actor routing projection 字段：**A0 冻结 schema/identity**；本包只声明该字段存在且
  immutable，不定义其内部结构；
- `ActiveRoutingEpochStore` invariant：**当前 immutable routing epoch 的唯一权威**；
  不拥有 pending activation、session eligibility cache、pin map、health history；
- 发布：原子 `Arc` replacement（`swap` 语义）；capture 返回完整 epoch 引用（whole-epoch，
  禁止混合 tuple）；旧 epoch 通过已捕获 `Arc` 延续（§3.3 old request/WS 语义）；
- 初始发布：仅从 `StableCommitted` outcome + 成功 load 构造；初始 skeleton 遇到
  pending 时 **fail closed 且不发布**（完整 cold recovery 归 E-activation/§4.2）；
- 发布 port（契约 port，W-bootstrap 实现）：`publish_committed(RoutingEpoch)`，typed
  input，atomic swap output；无第二 writer；publish 不可失败回滚（epoch 已 immutable）。

### 2.5 初始 bootstrap 顺序（W-bootstrap 装配）

```text
reader.read_committed(environment)
  -> projection(committed)
  -> strict loader（assembly + snapshot -> RoutingEpoch）
  -> epoch_store.publish_committed(epoch)
  -> RuntimeBootstrapProvider（wire 契约）向连接发送 router.bootstrap
```

readiness：committed epoch 发布后可启动 public listener；admission 开放依赖 E-session gate
（§4.2），本包不定义。

## 3. E-bootstrap 负例矩阵（本包冻结）

| 负例 | 期望 |
| --- | --- |
| committed record 缺失 | `FailClosedMissing`；无 epoch 发布 |
| committed record malformed | `FailClosedMalformed`；无 epoch 发布 |
| committed ref → assembly record missing | `FailClosedIdentityMismatch`；无 epoch 发布 |
| committed ref → assembly identity mismatch | `FailClosedIdentityMismatch`；无 epoch 发布 |
| snapshot record missing/malformed/id mismatch | `BootstrapLoadFailure`；无 epoch 发布 |
| pending 存在 | `FailClosedPending`；无 epoch 发布、不 stage candidate |
| blocking loader 饱和 | fail closed（`LoaderSaturated`）；无无限排队 |
| shutdown | drain/abort 后零 residue；新 load 拒绝 |

## 4. §5.4 pack 必填项

### 唯一 owner / invariant

- owner：`ActiveRoutingEpochStore`（W-bootstrap 实现）。
- invariant：**任一时刻最多一个当前 epoch，且该 epoch 完整、已校验、immutable**；epoch
  capture 与 publish 之间不产生第二个 authority；pending/eligibility/cache 永不进入
  epoch store。

### Typed inputs / outputs

- input：environment + `CommittedBootstrapRefs`（typed）；publish input `RoutingEpoch`
  （typed、已校验）。
- output：`BootstrapReadOutcome` / `BootstrapLoadFailure` / 原子 publish result；
  wire 输出 `RouterBootstrapFrameHeader`（C-model-bootstrap-wire 契约）。

### Capacity / queue full

- epoch store capacity = 1（current epoch），无队列；publish non-blocking atomic swap；
- loader pool：默认并发 8，饱和 = fail closed（`LoaderSaturated`），无 unbounded queue；
- reader 无内部队列。

### Timeout / disconnect / replacement / shutdown terminal

- 每次 store read 有 deadline（默认 5s），超时 fail closed；
- disconnect：bootstrap 链无 participant 语义（E-session 才引入），N/A；
- replacement：下一次 durable commit 成功后 atomic swap 替换当前 epoch；旧 epoch 由
  captured `Arc` 延续，**不被替换操作取消或删除**；
- shutdown：drain loader pool、abort 等待、停止 publish；fail-closed counters 归零检查
  （§10 序列测试要求）。

### Health fields

- `activeRoutingEpoch.{environment,generation,assemblyIdentity,configSnapshotId}`；
- `bootstrapReader.failClosed.{missing,malformed,identityMismatch,pending}`；
- `blockingLoader.{occupancy,queued,saturated,deadlineAborts}`；
- `epochStore.publishCount`。

### Fake seam

- fake `CommittedActivationBootstrapReader`（内存 outcome）；
- fake `RuntimeConfigSnapshotResolver`（内存 snapshot）；
- fake `RuntimeBootstrapProvider`（固定 header）；
- real seams：临时 root `CanonicalArtifactStore` + `RuntimeConfigSnapshotStore`（corpus 测试
  使用真实文件边界）。

### 至少一条真实边界 probe

- `bootstrap_chain_corpus.rs`：真实临时 root + `CanonicalArtifactStore`，覆盖
  committed-only（OK + `recovery_action == StableCommitted`）、pending（repository 读取
  返回 pending，bootstrap outcome 冻结为 fail closed）、missing、malformed、
  committed-ref missing / identity mismatch；同时冻结投影字段与 epoch 字段语义；
- `bootstrap_snapshot_reader_corpus.rs`：真实 snapshot store 正负例（C-model-artifact
  包的 snapshot 侧 probe）。

## 5. 本 pack 新增 corpus

`deployment/tests/fixtures/bootstrap-chain-corpus.json`
（schema `skiff-router-rust-bootstrap-chain-corpus-v1`）：

- `states`：committedOnly / pendingPresent / missing / malformed / committedRefMissing /
  committedRefMismatch，每项冻结 `repositoryRead`（现有 store 行为）与
  `bootstrapOutcome`（W-bootstrap 义务）；
- `projection`：committed → shared refs 字段映射；
- `epoch`：字段清单、publication 语义（atomic swap、单一 authority、pending 不进入）。

消费测试：`deployment/tests/bootstrap_chain_corpus.rs`。

## 6. W-bootstrap 交付义务（非本包实现）

1. 实现 2.1–2.5 全部 port/类型并消费本 corpus + artifact/snapshot corpus；
2. E-bootstrap 负例矩阵全部 fail closed 且 counters 可观测；
3. 不改变 `EnvironmentActivationState`/`CanonicalArtifactStore` 现有语义；
4. M-bootstrap-wire/M-artifact gate 通过后，与 session lane 汇合完成 E-session。
