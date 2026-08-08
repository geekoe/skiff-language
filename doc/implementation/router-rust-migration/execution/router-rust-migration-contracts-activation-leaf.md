# Router Rust Migration Contracts-activation Leaf Task

日期：2026-08-02

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。
节点：contracts-activation（activation 链 contract pack，一次性有界会话）
Agent：`/root/dev_contracts_activation`
集成目标：`/root/router_rust_integration_b3`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`
  （contracts-activation 节点、写边界、验证 owner、退出检查点）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5）
  - §2.2 三类 model 分离（第三类：Router/platform durable activation model，
    Runtime 只消费 activation prepare/commit/abort wire projection，不消费 durable record）；
  - §3.4 identity/fence 类型（`ActivationId` / `ActivationParticipantBinding`）；
  - §4.1 live transaction（步骤 1-10）；
  - §4.2 cold recovery（committed 先发布、pending recovery、rebind、E-session readiness）；
  - §5.3 `C-router-activation-state`（exact committed/pending DTO、revision、audit、
    read/CAS/retry/index/driver contract）；
  - §5.4 contract pack 必填项（owner/invariant、typed inputs/outputs、capacity、queue full、
    timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、真实边界 probe）；
  - §10 health 字段（activation durable/live/recovery transaction）。
- 直接父批次（batch 2，已合入基线）：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-2.md`、
  `doc/implementation/router-rust-migration/contracts/router-rust-migration-m0-decisions.md`（M0 已把 activation family
  独立为 `assembly_activation.rs`，wire corpus 与方向规则已存在）。
- 仓库：`/Users/geek/workspace/skiff`
- Baseline：`main@1d442366`（`git rev-parse 1d442366` = `1d442366e63e17085c4a4ab0d306627c5f494e3a`，已核对）
- Worktree：`/Users/geek/workspace/wt-contracts-activation`，branch `feat/router-rust-contracts-activation`

## 零 worktree 只读预检结论

1. 基线锚定成功；主 worktree 当前在 `integration/router-rust-migration-batch-3`（仅比基线多批次文档），
   不影响本节点；主 worktree 有一个未跟踪文件 `doc/architecture/actor-instance-evaluator-design.md`，
   非本节点产物，保持不动。
2. durable activation state 已存在于 `deployment/src/storage/activation.rs`：
   `EnvironmentActivationState`（v2）/`CommittedActivation`/`PendingActivation`，
   已实现 read / prepare / abort / commit CAS、`recovery_action`、canonical JSON strict parser；
   现有测试覆盖 CAS 失败矩阵、幂等 replay、recovery action。不存在 audit、Mongo driver、显式 revision 字段。
3. activation wire 已存在于 `artifact-model/src/assembly_activation_control.rs`
   （`AssemblyActivationControl`：Prepare/Prepared/Reject/Commit/Abort/Register，v2 strict 校验）与
   `runtime/transport/src/assembly_activation.rs`（frame codec + direction 规则）；
   `cross-system-fixtures/package-service-ecosystem/` 已有 `control-wire.json`/`runtime-wire.json`
   golden byte corpus 与 `activation-raw-cases.json`，由 transport/deployment 测试消费。
4. `RuntimeSessionEpoch` 类型在 baseline 尚不存在（由 contracts-session / C-session 冻结）；
   participant binding `{ replica_id, RuntimeSessionEpoch }` 在本节点只冻结为跨契约引用，
   不定义 production 类型。
5. 兄弟 ownership：A0（deployment projection 模块）、contracts-bootstrap（bootstrap-wire corpus）、
   contracts-session（registration/handshake corpus）与本节点文件集合无重叠；
   本节点只新增 activation 专属文档、JSON corpus 与独立测试文件。
6. 任务可在不修改 production、不改变公共契约的前提下闭合：现有 durable/wire 形态即目标形态，
   差异（无 audit、无 Mongo driver、revision 派生、文件 adapter 为现状）在契约文档中记录并冻结目标。

## 交付物（写集）

| 文件 | 内容 |
| --- | --- |
| `doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-activation-leaf.md` | 本文件（叶子任务） |
| `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-router-activation-state.md` | C-router-activation-state 契约冻结 |
| `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-activation.md` | C-model-activation 契约冻结 |
| `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-activation-coordinator.md` | C-activation-coordinator 契约冻结 |
| `cross-system-fixtures/package-service-ecosystem/activation-transaction-cases.json` | live/cold recovery 事务 corpus（新共享 fixture） |
| `cross-system-fixtures/package-service-ecosystem/activation-state-contract-cases.json` | durable state read/CAS/retry/recovery corpus（新共享 fixture） |
| `runtime/transport/tests/activation_transaction_corpus.rs` | wire + coordinator corpus 可执行测试（fake harness） |
| `deployment/tests/activation_state_contract.rs` | durable state corpus 可执行测试（真实 store） |

禁止写：skiff-router production、runtime/transport production 模块结构、
deployment A0 模块与既有 production 文件、AGENTS.md、scripts README、verify 注册表/selector graph/
verify.yml、skiff-instance.mjs；不操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 契约冻结要点（对应 §5.4 必填项）

- 每个 pack 文档定义唯一 owner/invariant、typed inputs/outputs、capacity、queue full、
  timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、至少一条真实边界 probe。
- `C-router-activation-state`：冻结现有 DTO 精确形态；revision 语义 = committed.generation +
  pending identity（不新增字段）；audit 目标形态（append-only、失败回滚、重试不重复）；
  index/driver 为 Mongo 目标契约（W-activation-state-repository 实现）；记录文件 adapter 现状差异。
- `C-model-activation`：冻结事务 wire（Prepare/Prepared/Reject/Commit/Abort）与方向矩阵；
  stale ACK 拒绝与 participant binding `{ replica_id, RuntimeSessionEpoch }`（epoch 不上 wire）。
- `C-activation-coordinator`：冻结 §4.1 步骤 1-10 与 §4.2 cold recovery 两个明确合同
  （live disconnect abort vs cold recovery rebind），corpus 分别覆盖。

## 自验收矩阵

| 项 | 命令/断言 |
| --- | --- |
| wire + coordinator corpus | `cargo test -p skiff-runtime-transport --test activation_transaction_corpus` |
| durable state corpus | `cargo test -p skiff-deployment --test activation_state_contract` |
| 现有 transport/deployment 回归 | `cargo test -p skiff-runtime-transport -p skiff-deployment`（本节点新增文件不破坏既有 corpus） |
| 契约文档 §5.4 必填项 | 三份文档逐项覆盖（owner/invariant、typed IO、capacity、queue full、terminal、health、fake seam、真实 probe） |
| 反向搜索（无 production consumer 提前依赖） | `rg -n "ActivationParticipantBinding|ActivationTransactionFrameSink|W-activation" runtime/transport/src router/src deployment/src` 无新 consumer 引用；本节点新增引用仅存在于 tests/ 与 doc/ |
| 写集边界 | `git status` 仅含上表文件 |
| rustfmt/clippy | 触碰 Rust 文件 `cargo fmt --check`；`cargo clippy`（触碰 crates，无新增 error） |

## 停止条件

- 需要改公共契约（DTO schema、wire schema、既有 corpus 语义）：停止上报，不自行扩展。
- 与兄弟节点文件重叠：先通知 root。
- 设计空洞：返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 执行结果（提交前自验收）

（2026-08-02 提交前填写，全部通过）

1. `cargo test -p skiff-runtime-transport -p skiff-deployment`：
   deployment 65 passed（lib）+ 1 passed（activation_state_contract corpus）；
   transport 113 passed（lib）+ 2 passed（activation_transaction_corpus）+ 2 passed
   （既有 assembly_replica_registration）。无回归。
2. live vs cold recovery 两个合同由 `activation-transaction-cases.json` 22 个 case
   （live 16 / coldRecovery 6，含 run-based 重启 case）经真实 wire codec 驱动通过；
   durable read/CAS/retry/recovery 由 `activation-state-contract-cases.json` 6 个 case
   驱动真实 `CanonicalArtifactStore` 通过。
3. rustfmt：新增两个 Rust 文件 `rustfmt --edition 2021 --check` 通过；
   workspace `cargo fmt --all --check` 仅剩 baseline 既有 `runtime/eval/src/actor_executor/tests.rs`
   的格式差异（本节点未触碰，已恢复原状）。
4. clippy：`cargo clippy -p skiff-runtime-transport -p skiff-deployment --tests`
   对新增文件零 warning/error（其余 warning 均为既有 crate baseline）。
5. 反向搜索：`rg` 证明 `ActivationParticipantBinding`、`ActivationTransactionFrameSink`、
   `activation-transaction-cases`、`activation-state-contract-cases` 只出现在本节点
   tests/ 与 doc/，production（`runtime/transport/src`、`deployment/src`、`router`）无消费者。
6. 写集：仅上表 8 个文件；`build/cargo-target`（worktree 内 target，1.7G）随 worktree 清理。
