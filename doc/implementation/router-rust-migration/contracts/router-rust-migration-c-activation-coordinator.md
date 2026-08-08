# Router Rust Migration C-activation-coordinator contract

日期：2026-08-02
状态：frozen（contract pack 交付；W-activation 为其实现 lane）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`
  - §3.2 owner 表：`ActivationCoordinator` 唯一拥有 durable activation transaction lifecycle 与
    live/recovery participant binding；不拥有 active epoch storage、session mutation、socket write；
  - §3.3 `ActiveRoutingEpochStore` 唯一 authority（原子 Arc swap 发布）；
  - §4.1 live transaction 步骤 1-10；§4.2 cold recovery；
  - §5.4 `C-activation-coordinator + M-activation + P-activation-state`；
  - §5.3 禁止“其它 domain owner 等待 Mongo”，但 coordinator 可以 await 自己的 persistence adapter；
  - §10 health 字段（activation durable/live/recovery transaction；per-owner mailbox 占用）。
- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（contracts-activation 节点）。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-activation-leaf.md`。

## 1. Owner 与 invariant

唯一 owner：`ActivationCoordinator`。

不变式：

1. 一个 environment 同时只有一个 live transaction（durable pending 槽位唯一，CAS 强制）。
2. durable commit CAS 一旦发出，transaction outcome 由 durable state 决定；disconnect/timeout
   不得再直接假定 abort，必须读 durable state reconcile（§4.1 step 7）。
3. durable commit 成功后，active epoch 只能经 `ActiveRoutingEpochStore` 的**已验证、不可失败**
   atomic `Arc` swap 发布；coordinator 没有自己的 eligibility cache 或 pending publication token。
4. durable commit/abort 成功后的 enqueue 失败 → abort exact session；绝不回滚已提交的 durable state。
5. coordinator 可以 await 自己的 persistence adapter，但不得占用 session/snapshot/dispatcher
   mailbox，也不跨 await 持有其它 owner state。
6. cold recovery 中 committed epoch 先发布，pending recovery 后台继续，不阻塞 Runtime listener；
   readiness/admission 归 E-session gate。

## 2. Typed inputs / outputs

Inputs：

- `AssemblyActivationRequest`（tooling→router，候选 generation 由 router 派生，participant set
  由 router 冻结）；
- repository read：`EnvironmentActivationState`（committed + pending + 派生 revision）；
- blocking loader：候选 `RoutingEpoch`（load ok | missing | malformed）；
- candidate query：`RegisteredSessionLease { session_epoch, registration_revision,
  exact_registered_tuple, cancellation }`（§3.3，stateless `RuntimeCandidateQuery`）；
- live bindings：`ActivationParticipantBinding { replica_id, RuntimeSessionEpoch }`；
- ACK events：`Prepared` / `Reject`（来源 session 已知）；
- enqueue results：ok | queue full；
- publish port：`PublishCommittedEpoch(committed) -> Published`（infallible）；
- shutdown signal。

Outputs：

- 每 session 的 prepare/commit/abort frame enqueue；
- durable prepare/commit/abort CAS 调用；
- exact session abort（enqueue 失败/断连时）；
- `ActiveRoutingEpochStore` atomic swap；
- health snapshot（durable/live/recovery transaction）。

## 3. Live transaction（§4.1 步骤 1-10，逐条冻结）

1. **读 durable revision + active epoch**：一次一致性读 committed generation 与当前
   `Arc<RoutingEpoch>` generation；不等则等待/重试或 fail closed。
2. **blocking loader**：有界 `spawn_blocking` 池（semaphore + timeout + shutdown）加载并验证
   候选 `RoutingEpoch`；saturation/missing/malformed → fail closed，不写 pending。
3. **candidate query 冻结**：用同一 current epoch 经 `RuntimeCandidateQuery` 冻结
   exact matching replica IDs 与 leases；不引入额外 heartbeat eligibility；
   立即校验 cancellation 与 current-by-replica binding。
4. **durable prepare CAS 前 revalidate**：再次校验 session epoch、registration revision、
   exact tuple、cancellation；任一失败不写 pending（terminal：failed，无 durable 副作用）。
5. **非阻塞 enqueue prepare**：向每个 exact session non-blocking enqueue；enqueue 失败
   → durable abort + abort exact session；decision 前任何 replacement/disconnect/cancellation
   → durable abort。
6. **ACK 校验**：按 live participant binding 校验（replica_id + `RuntimeSessionEpoch`）；
   stale/new session ACK 拒绝（counter + health），不产生 durable effect。
7. **durable commit CAS**：全部 exact ACK 后再次校验 live bindings，然后发 commit CAS；
   CAS 一旦发出 outcome 由 durable state 决定；CAS mismatch → 读 durable state reconcile：
   durable committed → 发布该 epoch + 向仍 exact 的 staged session enqueue commit；
   durable aborted → enqueue abort。
8. **atomic Arc swap**：durable commit 成功后执行已验证、不可失败的 swap；发布后
   新 admission 捕获新 epoch。
9. **commit enqueue**：向仍为 exact binding 的 participants non-blocking enqueue commit；
   enqueue 失败 → abort exact session，不回滚 durable state；Runtime reconnect 按 committed
   bootstrap 收敛并丢弃 staged candidate。
10. **新 admission**：从 swap 后 epoch 捕获，directory 无需 mutation/barrier。

Abort 路径：durable abort 成功后向仍为 exact binding 且已 staged 的 session enqueue abort；
enqueue 失败 → abort 该 session。prepare/commit/abort writer queue failure 一律按
exact session fence 处理。

## 4. Cold recovery（§4.2，冻结）

1. 启动读 durable state：committed 先构造并发布 active epoch（public listener 可开）；
2. durable pending 先安装 recovery transaction，不在 listener 启动前等待 participant；
3. Runtime listener 打开后，expected replica 注册时用 replica IDs 绑定新 exact session
   （session epoch 允许变化）并发送 prepare；
4. 候选加载失败 → 按 reducer durable abort；
5. recovery transaction 产生新的 ephemeral participant bindings；ACK 仍按该 binding 校验；
6. 进程在 durable commit 后、epoch swap 前退出：下次启动从 committed state 构造 epoch，
   不需要 pending publication token 或第二份 eligibility cache；
7. public listener 可在 committed epoch 发布后启动，但 readiness/admission 只有在至少存在
   满足 current routing epoch 的 session 并通过 E-session gate 后开放；pending recovery
   后台继续并通过 health 显式报告，不阻塞 Runtime listener（无冷启动死锁）。

live disconnect abort 与 cold recovery rebind 是两个明确合同，shared corpus 必须分别覆盖。

## 5. Capacity 与 Queue full

- capacity：每 environment 一个 pending（CAS）；blocking loader 池容量有界；
  coordinator mailbox 有控制/terminal reserved slot。
- queue full：prepare/commit/abort enqueue 全部 non-blocking；失败即 exact session fence
  （见 §3 step 5/9 与 abort 路径）；mailbox full 时 lifecycle terminal 仍可观察
  （cancellation watcher 独立于 mailbox dequeue，§3.6）。

## 6. Timeout / Disconnect / Replacement / Shutdown terminal

与 C-model-activation §5 一致（decision 前/后两列），coordinator 补充：

- ACK timeout（decision 前）→ durable abort + enqueue abort（staged exact sessions）；
- disconnect/replacement（decision 前）→ durable abort；decision 后 → durable authoritative
  reconcile，不向新 binding enqueue；
- shutdown：stop new activation → 若 in-flight 未 decision，先 reconcile durable pending
  （abort 或按 durable 结果收敛）→ 归零 pending/enqueue/session abort 计数 → 退出；
- 所有 terminal 路径 pending/permit/timer 归零；超时/异常路径不得泄漏 coordinator state。

## 7. Health fields

- durable transaction：committed generation、pending activation id、revision；
- live transaction：phase（idle/prepared/committing/committed/aborted）、activation id、
  expected/candidate generation、participant binding 数、prepared/reject ACK 计数、
  stale ACK 计数、decision 状态；
- recovery transaction：recovery 是否 active、已 rebind participant 数、等待注册 replica 集、
  readiness gate 状态；
- mailbox：coordinator data/control occupancy、saturation terminal 次数；loader 池占用。

## 8. Fake seam

```text
ActivationCoordinatorPorts {
  repository: ActivationStateRepositoryPort,     // read + prepare/commit/abort CAS + audit
  loader: BlockingLoaderPort,                    // load candidate RoutingEpoch
  candidates: RuntimeCandidateQueryPort,         // freeze leases for current epoch
  sessions: SessionEnqueuePort,                  // non-blocking prepare/commit/abort per exact session
  publish: PublishCommittedEpochPort,            // atomic Arc swap（infallible）
  health: HealthSinkPort,                        // owner-published snapshot
}
```

corpus harness 以 test double 实现除 wire codec 外的全部 port，并走真实
`encode/decode_assembly_activation_frame`。

## 9. 真实边界 probe（至少一条）

`router-live:activation-full-chain`（E-activation）：activate HTTP → durable prepare →
real Runtime prepared → durable commit → epoch swap → Runtime commit → 同 session re-register →
new-generation HTTP request 成功，同时 old captured epoch request 可按原 lease 完成。
冷启动变体：durable commit 后 swap 前杀进程 → 重启从 committed 构造 epoch；带 pending 重启 →
expected replica 注册时 rebind + 重新 prepare。

## 10. 验证映射

本批次 `runtime/transport/tests/activation_transaction_corpus.rs` +
`activation-transaction-cases.json` 以可执行 corpus 覆盖：live happy path、stale ACK、
duplicate ACK、reject、disconnect/replacement/timeout（decision 前 abort）、queue full
（prepare/commit 两处）、commit CAS mismatch（durable authoritative）、cold recovery
（committed-only、rebind+commit、等待 participant、candidate load failure、commit 后
swap 前退出）。
