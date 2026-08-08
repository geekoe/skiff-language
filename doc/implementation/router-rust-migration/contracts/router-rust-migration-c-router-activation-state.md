# Router Rust Migration C-router-activation-state contract

日期：2026-08-02
状态：frozen（contract pack 交付；W-activation-state-repository 为其实现 lane）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`
  - §2.2 第三类 model：Router/platform durable activation model；Mongo adapter Router-owned；
    Runtime 不消费 durable record；
  - §3.2 owner 表：`ActivationStateRepository` 唯一拥有 durable DTO/revision/audit、Mongo indexes、
    read/CAS/retry；不拥有 coordinator transaction、routing epoch；
  - §5.3 `C-router-activation-state`：exact committed/pending DTO、revision、audit、
    read/CAS/retry/index/driver contract；`CommittedActivationBootstrapReader` 只消费 repository
    read-only port；coordinator 与 bootstrap 共用同一个 repository instance；
  - §5.4 pack 必填项；
  - §4.1/§4.2 durable authoritative 语义（commit CAS 发出后 outcome 由 durable state 决定；
    cold recovery 由 durable pending 驱动）。
- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（contracts-activation 节点）。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-activation-leaf.md`。
- 现有实现：`deployment/src/storage/activation.rs`（DTO + CAS + recovery reducer）、
  `deployment/src/storage/io.rs`（`CanonicalArtifactStore` 文件 adapter）、
  `cross-system-fixtures/package-service-ecosystem/activation-state.json` /
  `activation-raw-cases.json`（现有 corpus）。

## 1. Owner 与 invariant

唯一 owner：`ActivationStateRepository`（Router-owned durable activation state；Mongo adapter
Router-owned，不得被 Runtime/transport 消费）。

不变式（frozen）：

1. 每个 environment 恰好一个 committed activation；最多一个 pending activation。
2. `pending.expected_generation == committed.generation`；`candidate_generation == expected + 1`
   （溢出为永久错误，不产生 pending）。
3. `pending.participant_replica_ids` 非空、去重、排序（`BTreeSet` 规范化），只写 replica ID，
   不写 ephemeral session epoch（live binding 由 coordinator 单独捕获）。
4. DTO 一律 canonical JSON（`skiff-canonical-json`），strict parse（deny unknown fields），
   schema version 精确等于 `skiff-environment-activation-state-v2`；v1 与缺失
   `configSnapshot` 成员按既有 hard-cut 测试拒绝。
5. 所有 assembly/config ref 必须通过 lexical validation，且 committed/pending 引用的
   `RuntimeAssembly` 必须存在于同一 artifact store（reference existence check）。
6. 同一 environment 的 mutation 串行化（当前文件 adapter 用 exclusive pointer lock；
   Mongo adapter 用 CAS filter），读到的 revision 一致（无撕裂读）。
7. 幂等性：完全相同的 mutation tuple 重放返回当前 state（不报错、不重复写）；
   任何不相同的 tuple 对同一 pending 槽位 → `CasMismatch`。

## 2. 精确 DTO（frozen 目标形态）

现有 `deployment/src/storage/activation.rs` 的以下 public 类型即冻结形态，W-lane 不改变字段：

```text
EnvironmentActivationState {
  schema_version: "skiff-environment-activation-state-v2",
  environment: String,
  committed: CommittedActivation { generation: u64, assembly: RuntimeAssemblyRef,
                                    config_snapshot: RuntimeConfigSnapshotRef },
  pending: Option<PendingActivation { activation_id: String,
                                      expected_generation: u64,
                                      candidate_generation: u64,
                                      assembly: RuntimeAssemblyRef,
                                      config_snapshot: RuntimeConfigSnapshotRef,
                                      participant_replica_ids: Vec<String> }>,
}
```

`RuntimeAssemblyRef` / `RuntimeConfigSnapshotRef` 由 artifact-model owner 定义，本契约不重复定义。

## 3. Revision 契约

现有 DTO 没有独立 revision 字段。冻结目标：**不新增字段**，revision 由既有字段派生：

- durable revision = `(committed.generation, pending.activation_id 或 ∅)`；
- CAS anchor 按 mutation 类型：
  - prepare：`(environment, committed.generation == expected_generation, pending == ∅ |
    与目标 tuple 完全相同的 existing pending)`；
  - commit：`(environment, committed.generation == expected_generation,
    pending tuple 全等（activation_id/expected/candidate/assembly/config/participants）)`，
    且 connected/prepared ACK set 必须满足
    `participants ⊆ connected && prepared == participants`；
  - abort：`(environment, committed.generation == expected_generation,
    pending.activation_id == activation_id)`；无 pending 时幂等成功（返回当前 state）。
- Mongo 目标实现（W-lane）用上述派生 tuple 构造 CAS filter，不需要 migration 或 v3 schema；
  任何需要加 revision 字段的改动属于公共契约变更，必须先停止上报。

差异记录：baseline 的 `CanonicalArtifactStore` 是文件/artifact-store adapter（exclusive pointer
lock + full-state CAS），不是 Mongo。DTO/revision/CAS 语义在两个 adapter 上必须一致；
`router-live:activation-mongo`（E-activation）用真实 replica set 验证 Mongo adapter 的同一契约。

## 4. Read 契约

- `read_environment_activation(environment)`：strict parse + validate + reference existence check；
  缺失文件 → `CasMismatch`（“state does not exist”），environment/path 不匹配 → `InvalidRecord`。
- read-only port：`CommittedActivationBootstrapReader`（C-bootstrap 消费）只读 committed 投影
  （generation/assembly/config_snapshot），不读 pending 语义；coordinator 与 bootstrap 必须共用
  同一个 repository instance，禁止 bootstrap 临时 Mongo reader 后期替换（§5.3）。
- live 步骤 1 的“current durable revision + active epoch”一致性读：revision 与 active epoch
  分别来自 repository 与 `ActiveRoutingEpochStore`，读到的 committed generation 必须等于
  captured active epoch 的 assembly generation 才能进入 candidate 流程。

## 5. CAS / Retry 契约

### CAS

prepare/commit/abort 三种 mutation 的 CAS 语义见 §3。失败分类：

- `CasMismatch`：可恢复的并发冲突（stale generation、不同 pending 占用、ACK set 不精确、
  commit tuple 不匹配）。调用方不得自动重试写，必须重新 read 后由 coordinator 决定
  （durable authoritative，§4.1 step 7）。
- `InvalidRecord`：不可恢复的输入/持久化损坏（schema、token、排序、canonical bytes）。
- `ImmutableConflict`/`Io`/`Json`：基础设施错误，按 retry 契约处理。

### Retry

- 只对基础设施类瞬态错误重试（driver 连接、backoff、读锁竞争），有界、指数退避、总 deadline；
- mutation 重试幂等：同一 tuple 重放成功且不产生重复 effect（见 §6 audit 不重复）；
- `CasMismatch`/`InvalidRecord` 不重试（重试只会再次失败或掩盖冲突）；
- retry 状态计入 health（当前退避次数、最后错误分类），不无限重试。

## 6. Audit 契约（目标形态，W-lane 实现）

当前实现无 audit（差异记录）。冻结目标：

- 每个成功 mutation 写一条 append-only audit event：
  `{ event_id, environment, activation_id, operation: prepare|commit|abort,
    expected_generation, candidate_generation, outcome: ok|cas_mismatch|invalid|error,
    participant_replica_ids? , timestamp }`；
- audit 写入与 state mutation 同事务/同 driver 会话；audit 写入失败 → 整个 mutation 回滚
  （E-activation 断言 “audit 失败回滚”）；
- retry 不重复 audit：幂等重放命中 identical state 时不追加新 audit（或追加 dedup event，
  以 event_id 幂等）；以 `(environment, activation_id, operation, generation tuple)` 为去重键；
- audit 不包含 Mongo URL、secret、业务 payload（§10 health/log 约束）。

## 7. Index / Driver 契约（Mongo 目标，W-lane 实现）

- collection：`activation_state`（每 environment 一条）+ `activation_audit`；
- index：
  - `activation_state`：`environment` unique；
  - `activation_audit`：`(environment, activation_id, operation, expected_generation)` 查询键 +
    `environment + timestamp` 维护键；
- driver：Mongo driver 归本契约（C-net 已声明“Mongo driver 归 C-router-activation-state”）；
  连接串来自 strict final Router config 的 `serviceDb.mongoUrl`，replica set 语义；
  driver 有连接超时、退避、关闭（process shutdown 最后关闭 Mongo）；不读 ambient env；
- adapter 是 Router-owned：`skiff-deployment`（或 W-lane 所在 Router-owned crate）只暴露
  repository port，coordinator/bootstrap 经 port 消费；Runtime/transport 不得依赖该 crate。

## 8. 容量、Queue full、Timeout / Disconnect / Shutdown terminal

- capacity：每 environment 一个 pending（CAS 强制）；retry 有界；audit append 无队列。
- queue full：不适用（repository 是同步/有界 adapter；无 domain mailbox）。
- timeout：driver 操作总 deadline 超时按瞬态错误处理，retry 后仍失败 → coordinator 视为
  decision 前失败（不写 pending）或 decision 后 reconcile（读 durable state）。
- disconnect/replacement：repository 不感知 session（只存 replica ID）；session 事件由
  coordinator 映射为 durable abort（decision 前）或 reconcile（decision 后）。
- shutdown：先完成 in-flight mutation/audit 的确定性结果，再关闭 driver；不保留半写状态
  （文件 adapter：replace 是单文件原子替换；Mongo adapter：事务）。

## 9. Health fields

repository health 至少暴露：

- committed generation + pending activation id（durable revision 投影）；
- 最近一次 mutation outcome（ok / cas_mismatch / invalid / transient）；
- retry/backoff 当前状态（attempt、next backoff、总 deadline）；
- audit 状态（最近 event、失败计数）；
- driver 连接状态（connected / reconnecting / closed）与关闭完成标志（shutdown residue = 0）。

## 10. Fake seam

冻结 repository port（W-lane 实现，本批次 corpus 用 fake/真实 store 双测）：

```text
ActivationStateRepository {
  read(environment) -> Result<EnvironmentActivationState, RepositoryError>
  prepare(input: PrepareInput) -> Result<EnvironmentActivationState, RepositoryError>
  commit(input: CommitInput) -> Result<EnvironmentActivationState, RepositoryError>
  abort(input: AbortInput) -> Result<EnvironmentActivationState, RepositoryError>
  append_audit(event) -> Result<(), RepositoryError>
}
```

`PrepareInput`/`CommitInput`/`AbortInput` 即现有 mutation 参数（environment、activation_id、
generation tuple、refs、participant/ACK replica sets）。corpus 测试分别用真实
`CanonicalArtifactStore`（`deployment/tests/activation_state_contract.rs`）与纯 state 断言驱动；
`router-live:activation-mongo` 是 managed Mongo 的真实边界 probe。

## 11. 真实边界 probe（至少一条）

`router-live:activation-mongo`（E-activation 起）：临时 Mongo replica set + 真实 repository，
断言：CAS revision 冲突、retry 不重复 audit、audit 失败回滚、重启/rebind 后 committed/pending
读取一致。corpus 测试在 baseline 先以真实文件 store 覆盖同一 CAS/retry/recovery 语义。

## 12. 验证映射

现有 `deployment/src/storage/activation/tests.rs`（golden strict decode、raw corpus）与
`deployment/src/storage/tests.rs`（prepare/abort/commit CAS 矩阵、crash recovery、幂等 replay）
冻结于 baseline；本批次新增 `deployment/tests/activation_state_contract.rs` +
`activation-state-contract-cases.json` 把 read/CAS/retry/recovery 语义固化为可执行 corpus。
