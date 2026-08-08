# Router Rust Migration Batch 3 — contracts-bootstrap Leaf

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_contracts_bootstrap`
集成目标：`/root/router_rust_integration_b3`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（当前在
  `integration/router-rust-migration-batch-3` 分支，基线 main 尚未包含；本叶子按路径引用，
  集成合流后可用）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，重点 §2.2、§3.3、
  §3.4、§5.3、§5.4、§7 E-bootstrap；冲突时以权威设计为准）。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-2.md`（M0 + C-net，已合入
  main@1d442366）；M0 决策记录：`doc/implementation/router-rust-migration/contracts/router-rust-migration-m0-decisions.md`。
- 仓库约定：`AGENTS.md`（skiff repo）、`/Users/geek/workspace/AGENTS.md`（workspace，git 外）。
- Baseline：`main@1d442366e63e17085c4a4ab0d306627c5f494e3a`（`git rev-parse main` 已验证，
  `git rev-parse 1d442366` 一致）。

## 任务范围

contract pack（bootstrap 链）：

1. C-model-bootstrap-wire：冻结 Router→Runtime bootstrap assembly/config refs
   （`RuntimeAssemblyRef`/`ConfigSnapshotRef` 形态、strict artifact inputs、direction、
   payload presence）。
2. C-model-artifact：冻结 compiler/Router/Runtime artifact model 消费边界（artifact
   identity、strict reader boundary、§2.2 三类 model 分类消费面）。
3. C-bootstrap：冻结 repository read port + durable-to-shared projection + strict loader +
   初始 `ActiveRoutingEpochStore` 发布契约（§3.3、§7 E-bootstrap 范围：committed 只读、
   pending fail closed、missing/malformed/identity mismatch 负例）。

非目标：不实现 W-bootstrap/W-artifact/M-pack consumer；不写 skiff-router production；不定义
actor projection（A0 独占）；不定义 activation DTO（contracts-activation 独占）。

## 零 worktree 只读预检结论（锚定 main@1d442366）

### 已存在的类型（本叶子只引用并冻结 corpus，不写 production）

| 类型 / 表面 | 现有 owner | 证据 |
| --- | --- | --- |
| `RuntimeAssemblyRef { assembly_identity }` | `skiff-artifact-model`（`src/runtime_assembly.rs`） | Deserialize 时调用 `validate_runtime_assembly_identity`；`deny_unknown_fields` |
| `RuntimeConfigSnapshotRef { snapshot_id }` | `skiff-artifact-model`（`src/runtime_config_snapshot.rs`） | `skiff-runtime-config-snapshot-v1:<32 lowercase hex>`；Deserialize 时调用 `validate_runtime_config_snapshot_ref` |
| `RouterBootstrapFrameHeader` / `RouterBootstrapActivationFrameHeader` | `skiff-runtime-transport`（`src/protocol/session.rs`） | `decode_router_bootstrap_frame_header`：schemaVersion/type/artifactsPath/serviceDb/http/activation 严格校验 |
| bootstrap wire 共享 corpus | `cross-system-fixtures/package-service-ecosystem/runtime-bootstrap-wire.json` | transport `protocol/tests.rs` `router_bootstrap_shared_corpus_has_strict_parity`（1 accept / 16 reject） |
| frame family 规则 | `skiff-runtime-transport`（`src/protocol.rs`） | Session family `Either` direction、`Empty` payload presence（M0-D3 冻结） |
| `CommittedActivation` / `PendingActivation` / `EnvironmentActivationState` | `skiff-deployment`（`src/storage/activation.rs`） | `read_environment_activation` 校验 committed refs 指向真实 assembly 记录；`recovery_action` |
| `CanonicalArtifactStore::read_runtime_assembly` | `skiff-deployment`（`src/storage/records.rs`） | strict reader：path/declared/computed identity + canonical bytes 四重校验 |
| `RuntimeConfigSnapshotStore` / `RuntimeConfigSnapshotResolver` | `skiff-runtime-config-snapshot`（`src/store.rs` / `src/resolver.rs`） | strict read：canonical JSON、大小上限、id/path mismatch 拒绝；resolver trait 已存在 |
| identity 计算/校验 | `skiff-artifact-identity`（`src/runtime_assembly.rs`） | `runtime_assembly_identity` / `runtime_assembly_ref` / `validate_runtime_assembly_identity` |
| record 路径 | `skiff-artifact-identity`（`src/ecosystem_paths.rs`） | `RuntimeAssemblyRecordPath`、`EnvironmentActivationStatePath` |

### 不存在、只允许契约定义的表面

- `CommittedActivationBootstrapReader` read port（W-bootstrap 实现）。
- durable→shared 投影 reducer（W-bootstrap 实现）。
- `RoutingEpoch` / `ActiveRoutingEpochStore` / 初始发布 port（W-bootstrap 实现；actor
  routing projection 字段归 A0，本叶子只引用不定义）。
- bootstrap strict loader 组合（W-bootstrap 实现）。

### 兄弟 ownership 核对

- contracts-session：独占 handshake/registration/session corpus；本叶子只写
  `runtime/transport/testdata/router-rust-bootstrap-wire-corpus.json` 与
  `runtime/transport/tests/bootstrap_wire_corpus.rs`（bootstrap-wire 相关），不触碰
  registration/health/session corpus。
- contracts-activation：独占 activation-state DTO/corpus；本叶子只读 `EnvironmentActivationState`
  现有 production 类型，不写 deployment production 代码。
- A0：独占 actor routing projection schema/类型；本叶子不定义。
- PR 0b：独占 skiff-router production / `scripts/skiff-instance.mjs` / verify 注册表；本叶子不写。

## 写集（全部在 worktree `/Users/geek/workspace/wt-contracts-bootstrap`）

契约文档（`doc/implementation/`）：

1. `router-rust-migration-contracts-bootstrap-leaf.md`（本文件）。
2. `router-rust-migration-c-model-bootstrap-wire-contract.md`。
3. `router-rust-migration-c-model-artifact-contract.md`。
4. `router-rust-migration-c-bootstrap-contract.md`。

Corpus fixture 与其测试（不写 production）：

5. `runtime/transport/testdata/router-rust-bootstrap-wire-corpus.json` +
   `runtime/transport/tests/bootstrap_wire_corpus.rs`。
6. `deployment/tests/fixtures/bootstrap-artifact-corpus.json` +
   `deployment/tests/bootstrap_artifact_reader_corpus.rs`。
7. `deployment/tests/fixtures/bootstrap-chain-corpus.json` +
   `deployment/tests/bootstrap_chain_corpus.rs`。
8. `runtime-config-snapshot/tests/fixtures/bootstrap-snapshot-corpus.json` +
   `runtime-config-snapshot/tests/bootstrap_snapshot_reader_corpus.rs`。

禁止写：skiff-router production、deployment/artifact-model production 类型、runtime/transport
production 模块结构、`router/` 其他文件、AGENTS.md、scripts README、verify 注册表/selector
graph/verify.yml、`scripts/skiff-instance.mjs`。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| C-model-bootstrap-wire corpus 测试 | `cargo test --package skiff-runtime-transport --test bootstrap_wire_corpus` |
| C-model-artifact corpus 测试（assembly reader） | `cargo test --package skiff-deployment --test bootstrap_artifact_reader_corpus` |
| C-bootstrap corpus 测试（repository/projection/epoch） | `cargo test --package skiff-deployment --test bootstrap_chain_corpus` |
| C-model-artifact corpus 测试（snapshot reader） | `cargo test --package skiff-runtime-config-snapshot --test bootstrap_snapshot_reader_corpus` |
| 既有 crate 测试未回归 | `cargo test -p skiff-runtime-transport -p skiff-deployment -p skiff-runtime-config-snapshot`（聚焦） |
| 契约文档覆盖 §5.4 必填项 | 三份 contract 文档各含 owner/invariant、typed I/O、capacity、queue full、terminal、health、fake seam、真实边界 probe |
| 无 production consumer 提前依赖 | `rg` 反向搜索：无 skiff-router/deployment/transport production 引用 corpus 或契约类型（见下） |
| 无新 workspace crate / Cargo.toml 改动 | `git status` 写集审计；Cargo.lock 不变 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
