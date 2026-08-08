# Router Rust Migration Batch 7 — W-activation Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_activation`
集成目标：`/root/router_rust_integration_b7`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-7.md`
  （W-activation 节点；baseline `main@7d8779c4`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（owner/invariant）、§3.3（active routing 单一 authority）、
  §3.4（identity/fence）、§4.1（live transaction 步骤 1-10）、§4.2
  （cold recovery）、§5.4（C-activation-coordinator → W-activation）、
  §5.5（demux 与 sink bundle，本节点不写 demux）、§7 E-activation
  （full-chain 真实 Runtime 归后续节点）。
- 冻结契约（batch 3 已合入基线）：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-activation-coordinator.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-activation.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-router-activation-state.md`
  - `doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-activation-leaf.md`
- 同链交付叶子：
  - W-activation-state（`router/src/activation/` repository/retry/health/
    index/memory，已合入 main@7683b7c8，本节点只消费不改）
  - W-bootstrap（`router/src/bootstrap/`：`RoutingEpoch` /
    `ActiveRoutingEpochStore` / `BlockingLoader` / `BootstrapStrictLoader`，
    已合入 main@85596193）
  - W-routing-query（`router/src/routing/query.rs`：`RuntimeCandidateQuery` /
    `RegisteredSessionLease` / `CandidateDirectoryView`，已合入 main@8cabf352）
  - W-session（`router/src/session/`：`RuntimeRegistrationDirectory` /
    `RuntimeSessionEpoch` / `ConsumerKind::ActivationCoordinator` 预留，
    已合入 main@7683b7c8）

## 零 worktree 只读预检结论（锚定 main@7d8779c4）

1. 基线：`git rev-parse 7d8779c4` =
   `7d8779c4b96c90c4d2d23748112ec1c0328091d7`；worktree
   `/Users/geek/workspace/wt-w-activation` 已创建，分支
   `feat/router-rust-w-activation`，HEAD 即该 commit。主 worktree 已由
   集成 Agent 移到 `integration/router-rust-migration-batch-7`
   （efde0dd9，仅批次父文档）；`main` 另有无关 actor 文档 commit
   （dc61c020），本节点仍锚定任务指定基线。
2. 消费端口全部就绪且为冻结形态：
   - repository：`ActivationStateRepository`（read/prepare/commit/abort/
     append_audit/initialize/ensure_indexes/health/close）+
     `MemoryActivationStateRepository` fake；
   - epoch/publish：`RoutingEpoch`（environment/generation/refs/
     `registered_tuple()`）+ `ActiveRoutingEpochStore::publish`（不可失败
     whole-pointer swap）+ `BlockingLoader`（bounded semaphore/deadline/
     shutdown）；
   - candidate query：`RuntimeCandidateQuery`（stateless）+ typed
     `RegisteredSessionLease`/`CandidateDirectoryView`；
   - session：`ConsumerKind::ActivationCoordinator` 已在
     `router/src/session/consumer.rs` 预留（W-session 静态 manifest），
     `SessionConsumer::on_session_closed` 是断连通知端口；activation
     writer 的 transaction sink seam（C-model-activation §7）尚未实现，
     由本节点以 `SessionEnqueuePort` 定义，E-activation 接线。
3. contracts-activation corpus：
   `cross-system-fixtures/package-service-ecosystem/activation-transaction-cases.json`
   共 22 个 case（live 16 / coldRecovery 6，含 run-based 重启 case），
   语义由 `runtime/transport/tests/activation_transaction_corpus.rs`
   reference harness 冻结；本节点以真实 `ActivationCoordinator` 逐条驱动。
4. `AssemblyActivationRequest`（tooling→router strict request）与
   `AssemblyActivationControl`（Prepare/Prepared/Reject/Commit/Abort wire
   DTO）已在 `skiff-artifact-model` 冻结；candidate generation 由
   coordinator 派生（expected + 1），participant set 由 coordinator 冻结。
5. 写面无重叠：`router/src/activation/` 现有 repository/error/health/
   retry/index/memory；本节点只新增 coordinator.rs / recovery.rs（mod.rs
   与 lib.rs additive）。兄弟 worktree（wt-e-session-gate /
   wt-w-websocket / wt-w-actor）与本节点文件集合不重叠。
6. 依赖：`skiff-router` 已含 mongodb/async-trait/serde_json/tokio(sync,
   time) 及全部 workspace crate；不新增 Cargo 依赖。
7. 无设计空洞，任务可闭合；不返回 TASK_SCOPE_EXPANDED /
   TASK_NOT_EXECUTABLE。full-chain（真实 Runtime prepare/commit/re-register）
   与 demux transaction sink 接线归 E-activation，本节点以 typed ports +
   真实 wire codec 验证边界。

## 实现决策（冻结契约语义内）

### 1. `ActivationCoordinator` 形态

- 每 environment 一个 actor：`ActivationCoordinator::spawn(ports, options)`
  返回 cloneable `ActivationCoordinatorHandle`（bounded mailbox + 共享
  health/phase watch + actor task）。
- 外部事件全部 non-blocking `try_send`：start_live、deliver_ack、
  on_session_closed（断连）、on_session_replaced、force_ack_timeout、
  register（recovery rebind）、shutdown、hard_abort（process-exit 语义）。
  mailbox full → `CoordinatorError::MailboxFull` + saturation 计数；
  断连投递失败按 reserved-slot 语义返回 Err（session layer fail-stop）。
- actor 单任务顺序处理；内部 continuation 事件（`BeginCommit`、
  `PublishAndCommitEnqueue`）经同一 mailbox 排队，使 durable commit 返回后
  **先处理已排队的外部事件**再 publish/enqueue——这是
  `live-disconnect-after-commit-reconciles` 与
  `cold-recovery-exit-after-commit-before-swap` 的确定性基础（corpus
  在 commit 与 enqueue 之间插入 disconnect/exit）。
- coordinator 只 await 自己的 ports（repository/loader）；session port
  enqueue 为同步 non-blocking；不跨 await 持有其它 owner state；不占用
  session/snapshot/dispatcher mailbox。
- `ActivationCoordinatorHandle` 实现 `SessionConsumer`
  （`ConsumerKind::ActivationCoordinator`）：`on_session_closed` 把
  `RuntimeSessionClosed` 转成 Disconnect 事件。

### 2. Ports（C-activation-coordinator §8 fake seam）

```text
repository: ActivationStateRepositoryPort   // 既有 trait，不改
loader:     CandidateLoaderPort             // async；production adapter =
                                            //   BlockingLoader + BootstrapStrictLoader
candidates: RuntimeCandidateQueryPort       // freeze leases + revalidate
sessions:   SessionEnqueuePort              // prepare/commit/abort + abort_session
publish:    PublishCommittedEpochPort       // 不可失败 Arc swap
health:     HealthSinkPort                  // owner-published snapshot
```

- `RuntimeCandidateQueryPort::freeze(environment)` 捕获 current epoch 并返回
  exact leases；coordinator 校验每条 lease 的
  `exact_registered_tuple.{environment, generation}` 与 durable committed
  一致，不一致 fail closed（§4.1 step 1）。`revalidate(activation_id,
  frozen)` 返回 ok/stale（step 4/5/7 共用）。
- production adapter（本节点交付，供 E-activation 接线）：包装
  `ActiveRoutingEpochStore` + `RuntimeCandidateQuery` +
  `CandidateDirectoryView`（directory lock 内 coherent snapshot）；activation
  participant 是所有 exact session，不按 dispatch capability 过滤，因此对
  unary 与 serverStream 两个 mode 各查一次并去重
  （documented seam：capability binding 由调用方注入，directory 尚未保留
  dispatch_modes；E-activation 接线时可复核）。
- `SessionEnqueuePort` 同步 non-blocking，返回 `EnqueueResult { Ok,
  QueueFull }`；wire control 由 coordinator 用
  `AssemblyActivationControl::Prepare/Commit/Abort` 构造（候选 refs 来自
  request/pending；`service_db` 来自 options，默认 None），replica_id 来自
  participant binding。`abort_session(session)` 是 queue-full 的 exact
  session fence。
- `PublishCommittedEpochPort` production adapter 直接包装
  `ActiveRoutingEpochStore::publish`（infallible）。

### 3. Live transaction（§4.1 步骤 1-10）

1. 读 durable state；request.validate()；committed generation !=
   request.expected → fail closed（不写 pending）。
2. `freeze(environment)` 冻结 leases（含 tuple generation/env 校验；
   empty → fail closed）。
3. `loader.load_candidate(expected+1 refs)`：Missing/Malformed →
   fail closed（不写 pending）；loader 基础设施错误同样 fail closed。
4. `revalidate` 失败 → terminal Failed，无 durable 副作用。
5. `repository.prepare` CAS（真实 reducer 语义）；CasMismatch → Failed。
6. enqueue prepare 前再次 `revalidate`（ok 后逐 binding non-blocking
   enqueue，staged += (replica, session_epoch)）；queueFull → abort exact
   session + durable abort + 向已 staged 且仍 exact 的 session enqueue
   abort。
7. 等待 ACK（可配置 deadline；`force_ack_timeout` 供 corpus/测试）。
   ACK 校验：decision none、replica ∈ participant set、staged 含
   (replica, epoch)、binding epoch 相等、未重复 prepared/rejected、
   control tuple 与 pending 精确匹配；任一失败 → stale ACK 计数 +1，
   无 durable effect。Reject 被接受 → durable abort 路径。
8. 全部 exact prepared 后：`revalidate` live bindings → `repository.commit`
   CAS（connected = 当前 bindings，prepared = prepared set）。CAS
   mismatch → 读 durable reconcile：committed → publish + 向仍 exact 的
   staged enqueue commit；aborted → enqueue abort。
9. commit 成功后 `PublishAndCommitEnqueue` 内部事件：publish candidate
   epoch（infallible swap）→ 向仍为 exact binding 的 staged participants
   enqueue commit；queueFull → abort 该 session（不回滚 durable）。
10. 新 admission 从 swap 后 epoch 捕获（本节点不写 directory/admission）。

Abort 路径：durable abort 成功后向仍 exact 且已 staged 的 session enqueue
abort（排除已 reject 的 replica）；queueFull → abort 该 session。

### 4. Cold recovery（§4.2）

`start_recovery(environment)`：

1. 读 durable state；无 pending → 经 loader 构造 committed epoch 并
   publish → terminal Committed（listener 可开，readiness 归 E-session）。
2. 有 pending → 先 publish committed epoch（不阻塞 listener）→ 安装
   recovery transaction（phase `WaitingRecovery`，readiness =
   waiting replica set 为空）→ 后台加载 candidate（pending refs）；
   load 任一失败 → reducer durable abort（terminal Aborted）。
3. expected replica `register`（recovery rebind）：用 replica id 绑定新的
   exact session（允许 session epoch 变化）并 enqueue prepare；非 expected
   replica 忽略。
4. ACK 按 recovery 产生的 ephemeral binding 校验（stale/new 拒绝）。
5. 全部 exact prepared 后与 live 相同：commit CAS → reconcile → publish →
   commit enqueue。
6. durable commit 后、swap 前进程退出：下一次启动从 committed state 构造
   发布，无 pending publication token / 第二份 eligibility cache
   （`cold-recovery-exit-after-commit-before-swap` run 2）。

live disconnect abort 与 cold recovery rebind 是两个明确合同，corpus
分别覆盖。

### 5. 状态与 health

- `ActivationPhase`：idle / freezing / prepared / waitingRecovery /
  committing / committed / aborted / failed / shutdown / exited；
  `ActivationCoordinatorHealth`：phase、environment、activation id、
  expected/candidate generation、participant binding 数、prepared/reject
  ACK 计数、stale ACK 计数、decision 状态、recovery active、rebind 数、
  waiting replica 集、readiness、mailbox occupancy/capacity/saturation、
  shutdown。不暴露 payload/secret。

### 6. Corpus 映射

- 测试驱动脚本按 case 的 steps 预置 fakes：readState → 初始化真实
  `MemoryActivationStateRepository`；captureActiveEpoch → 发布 current
  epoch；loadCandidate → fake loader 结果；queryCandidates → fake leases；
  revalidate → fake 首次结果；durablePrepare → 真实 reducer CAS（预期
  casMismatch 由初始 pending/stale generation 自然产生）；enqueuePrepare /
  enqueueCommit / enqueueAbort → fake session port 结果脚本。
- coordinator 自动执行步骤 4-5 与 7-10；驱动在 `Prepared` /
  `WaitingRecovery` / terminal phase 间用 phase watch 同步。需要确定性
  交错时（disconnect-after-commit、exit-after-commit）用 fake repository
  的 commit gate（commit-started 信号 + release），先排队外部事件再放行
  commit，保证队列顺序为 [外部事件, 内部 PublishAndCommitEnqueue]。
- 22 个 case 逐条断言：terminal、durable state（committed/pending）、
  published、listenerOpen、readiness、recovery、session aborts、enqueues
  顺序、stale ACK 计数、active epoch；run-based case 按 run 携带 durable
  状态重开新 coordinator。

### 7. 临时 Mongo 探针

- `router/tests/activation_coordinator_mongo_probe.rs`（`#[ignore]`）：
  真实 `MongoActivationStateRepository` + 真实 `ActiveRoutingEpochStore` +
  fake loader/candidates/sessions；两条完整
  prepare→ACK→commit→swap→commit-enqueue 链（唯一 activation_id），断言
  generation 7→8→9、epoch store 两次 swap、audit 每有效 mutation 恰好
  一条且无重复、stale ACK 拒绝计数、queue-full commit 只 abort session。
  full-chain 真实 Runtime 归 E-activation。
- `scripts/run-router-activation-coordinator-mongo-probe.mjs` 复用
  `scripts/lib/activation-state-live-harness.mjs`
  （`ActivationStateMongoHarness`，45000-45999 租约端口 + mktemp dbPath +
  用后清理），不触碰 stable Mongo/instance/PM2/4004-4007。

## 写集

生产（仅本叶子）：

- `router/src/activation/coordinator.rs`（新）
- `router/src/activation/recovery.rs`（新）
- `router/src/activation/mod.rs`（仅 additive：`pub mod coordinator;`
  `pub mod recovery;` + re-export）
- `router/src/lib.rs`（仅 additive re-export）

测试 / 脚本 / 文档：

- `router/tests/activation_coordinator_corpus.rs`（22 case 逐条）
- `router/tests/activation_coordinator_unit.rs`（序列/负例/端口单元）
- `router/tests/activation_coordinator_mongo_probe.rs`（ignored live probe）
- `scripts/run-router-activation-coordinator-mongo-probe.mjs`
- `doc/implementation/router-rust-migration/execution/router-rust-migration-w-activation-leaf.md`（本文件）

禁止写：`run_router`/`main.rs`/`listener.rs`、
`router/src/session`、`router/src/routing`、`router/src/bootstrap`、
`router/src/ws`、`router/src/actor`、runtime crate、`runtime/transport/src`、
deployment、AGENTS.md、scripts README、verify 注册表/selector graph、
verify.yml、`scripts/skiff-instance.mjs`；不操作 stable instance / Mongo /
PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| coordinator corpus 22 case | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-router --test activation_coordinator_corpus` |
| 单元/序列 | `cargo test -p skiff-router activation_coordinator_unit`（含 unit + corpus 文件内非 corpus 断言） |
| 全 crate 回归 | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-router`（既有测试不回归） |
| 临时 Mongo probe | `node scripts/run-router-activation-coordinator-mongo-probe.mjs`：两条 prepare→commit→swap→commit 链、audit 去重、stale ACK、cleanup 通过 |
| 聚焦 verify | `CARGO_TARGET_DIR=<worktree>/target node scripts/verify.mjs --only router-rust` |
| rustfmt/clippy | 触碰 Rust 文件 `cargo fmt --check`；`cargo clippy -p skiff-router --all-targets`（无新增 error） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦；禁止区零命中 |

## 交接

完成后向 `/root/router_rust_integration_b7` 报告 branch、worktree、commit
hash、实际写集、自验收矩阵与已知 seam（production adapter 的 capability
union / `SessionEnqueuePort` 生产接线 / demux transaction sink / epoch
source 装配归 E-activation），并通知 root。

## 执行结果（提交前自验收填写）

（2026-08-02 提交前填写，全部通过）

1. coordinator corpus 22 case 逐条通过：
   `cargo test -p skiff-router --test activation_coordinator_corpus`
   （live 16 / coldRecovery 6，含 run-based
   `cold-recovery-exit-after-commit-before-swap` 两 run，与
   `runtime/transport/tests/activation_transaction_corpus.rs` 同一 JSON
   fixture 驱动真实 `ActivationCoordinator`）。wire 事件全部经真实
   `skiff-runtime-transport` codec 往返。
2. 单元/序列 13 项通过：
   `cargo test -p skiff-router --test activation_coordinator_unit`
   （production adapter 真实 epoch/directory/blocking pool、SessionConsumer
   fence、mailbox saturation、同步拒绝、空候选/epoch 不匹配 fail closed、
   service-db wire、同一 coordinator 连续两笔 transaction、生命周期
   terminal、phase/decision 词表）。
3. 全 crate 回归与聚焦 verify：
   `cargo test -p skiff-router` 全绿；
   `CARGO_TARGET_DIR=<worktree>/target node scripts/verify.mjs --only router-rust`
   passed（1 task，0 failed）。未跑全量 `pnpm verify`。
4. 临时 Mongo 探针：
   `node scripts/run-router-activation-coordinator-mongo-probe.mjs` 通过——
   真实 Mongo replica set（45000-45999 租约 + mktemp dbPath）上两条完整
   prepare→ACK→durable commit→epoch swap→commit-enqueue 链（generation
   7→8→9）、stale/new session ACK 拒绝（counter=1）、audit 每有效 mutation
   恰好一条且无重复（activation-8 / activation-9 各 2 条，总 4 条）；
   `scripts/run-router-activation-mongo-probe.mjs`（既有 repository 探针）
   复跑通过；mongod/临时目录/端口租约全部清理，未触碰 stable
   Mongo（27017）/instance/PM2/4004-4007。
5. rustfmt：全部新增/触碰 Rust 文件
   `rustfmt --edition 2021 --check` 通过；
   clippy：`cargo clippy -p skiff-router --all-targets` exit 0，本节点文件
   零 warning/error（其余 warning 均为既有 crate baseline）。
6. 写集：`git status` 仅含本叶子声明文件（2 个 additive 修改 +
   7 个新增）；`git diff` 证明 mod.rs/lib.rs 只增不删；未触碰
   run_router/main/listener、session/routing/bootstrap/ws/actor、
   runtime crate、runtime/transport/src、deployment、AGENTS.md、
   scripts README、verify 文件、skiff-instance.mjs；Cargo.toml/Cargo.lock
   零变化。
7. 已知 seam（交接给 E-activation）：`SessionEnqueuePort` 生产接线
   （demux transaction sink + session writer）、`RoutingCandidateQueryPortAdapter`
   的 capability-union 语义（directory 尚未保留 dispatch_modes）、
   `BlockingLoaderCandidatePort` 的 `ActorRoutingProjectionRef` 输入、
   epoch source/run_router 装配；full-chain 真实 Runtime
   prepare/commit/re-register 归 E-activation。
