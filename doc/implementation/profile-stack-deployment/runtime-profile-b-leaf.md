# Leaf Task: 阶段 B Runtime 侧 profile 语义（runtime-profile-b）

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`
  （integration/profile-stack @ ce14b650 已提交），§2/§3.2/§4/§5/§6.1/§11/§12。
- 直接父节点：设计 §5（Runtime 语义）、§12（评审决议，2026-08-04 用户确认）；
  runtime/transport v4/profile wire 已合入基线（`RouterBootstrapActivationFrameHeader.profile`、
  `AssemblyActivationControl.profile`、`RuntimeConfigSnapshot.profile()`）。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，HEAD ce14b650）。
- 本分支：dev/runtime-profile，worktree
  `/Users/geek/workspace/skiff-dev-runtime-profile`。

## 零 worktree 只读预检结论（基线 ce14b650，Git 对象锚定）

### 1. 基线状态

- `integration/profile-stack` HEAD == ce14b650，与任务 baseline 一致。
- 共享主 worktree（main）未受影响；并行 worktree：`skiff-dev-router-profile`
  （router 节点）、`skiff-dev-testinfra` / `skiff-integration-testinfra`（无关批次）。
  本节点写集与它们无重叠。

### 2. 真实入口与关键调用链

1. `runtime/driver/main.rs` → `RuntimeFileConfig::load`（runtime.yml）→
   `RuntimeProductionConfig`（当前含 environment）→ `RuntimeHost::new_production`。
2. router.bootstrap frame → `decode_connection_bootstrap`（router_session.rs）→
   `RuntimeHost::recover_durable_committed(activation.environment, ...)`（lifecycle.rs）
   → 与 `RuntimeHost.environment` 校验 → `AssemblyAdmissionController::recover_committed`
   → `resolve_started_exact_candidate` → `validate_snapshot_environment` →
   `build_started_candidate` → `ActiveAssemblyContextSet::from_candidate`（物化 ConfigView）。
3. assembly activation control → `apply_bootstrapped_assembly_activation_control` /
   `apply_cancellable_...`（assembly_admission.rs:887/914）→ `activation_control_environment`
   与 `trusted_environment` 校验 → `apply_activation_control_inner`（解构
   `AssemblyActivationControl::{Prepare,...}` 的 environment 字段）→ prepare/commit →
   同一 `resolve_started_exact_candidate` 路径。
4. service-db 加密域：`ActiveAssemblyContextSet` → `DbProviderBuildInput.environment`
   （runtime/capability-context，非本节点写集，字段名保持）→
   `MongoServiceDbProviderFactory::runtime_from_input` → `ServiceDbRuntime::new_with_config`
   → `ServiceDbMetadata::from_runtime_program_db_with_encryption(..., storage_environment, ...)`
   → `DbEncryptedFieldContext.storage_environment` → KDF info 与 AEAD AAD
   （encryption.rs `derive_field_key` / `field_aad`）。

### 3. 基线编译事实（本节点待迁移的中间态）

- artifact-model 阶段 A 已改名：`AssemblyActivationControl` 各变体字段为 `profile`；
  `validate_activation_environment` 已删除，只有 `validate_activation_profile`；
  runtime-config-snapshot 只有 `profile()`（无 `environment()`）。
- runtime/host、runtime/driver、runtime/tests、runtime/service-db（storage_identity）
  仍引用已删除/已改名的 environment 符号，基线这些 crate 无法编译；这正是本节点写集。
- runtime/transport 已全量迁移（不改）；router、test-runner、scripts、cross-system-fixtures
  不改。

## 任务边界与实现决策

### 写集（按 §5/§12）

1. `runtime/driver/config.rs`：`RuntimeFileConfig` 删除 `environment`；runtime.yml 只保留
   router / runtime-home / serviceDb.encryption.keyringFile / http.egress.proxy；
   `environment` 键由 `deny_unknown_fields` 拒绝（无兼容层）。
2. `runtime/driver/main.rs`：`RuntimeProductionConfig` 不再携带 environment/profile；
   profile 只来自连接级 bootstrap。
3. `runtime/host/src/host/runtime_host.rs` + `lifecycle.rs` + `router_session.rs` +
   `router_session/activation.rs`：`RuntimeHost` 删除 `environment` 字段；
   新增 `frozen_profile: OnceLock<String>`：
   - 测试侧 `RuntimeConfig.profile` 构造时冻结（test-only 表面，保持既有测试语义）；
   - 生产侧 `RuntimeProductionConfig` 无 profile，首次 router.bootstrap 时
     `freeze_bootstrap_profile` 冻结；后续 bootstrap/activation control 与冻结值不一致
     fail closed（重连报错，不静默重载）。
4. 快照校验：`validate_snapshot_environment` → `validate_snapshot_profile`；
   `resolve_started_exact_candidate` 在物化 ConfigView（`materialize_snapshot_config`）之前
   校验 `snapshot.profile() == bootstrap profile`，不一致 fail closed。
5. `runtime/host` admission 内部 `AssemblyTransition`/`CommittedAssembly`/`AssemblyCandidateStage`
   相关 environment 字段与消息全部改为 profile（跨重连 committed profile 校验保留）。
6. `runtime/service-db`：加密/storage 域标识 `storage_environment` → `storage_profile`，
   `MigrationTargetContext.environment` → `profile`，`ServiceDbRuntime::new/new_with_config`
   与 `service_storage_database_name` 参数改为 profile；KDF/AAD 使用 bootstrap profile 值。
   migration_tool 的 CLI 面（`operator.environment`、receipt 等）不动，属后续工具层节点。
7. `runtime/tests` 共享 corpus 消费者（h_registration_cut_corpus、w_model_bootstrap_wire_consumer、
   w_model_registration_consumer 等）与 runtime/host、runtime/driver、runtime/service-db 的
   test/fixture 同步到 profile；cross-system-fixtures 不改。

### 强制停止

- 需触碰 runtime/transport、router、test-runner、scripts、文档或共享 corpus 时停止上报。
- 发现 design 空洞或需用户决策的语义变化时停止，不自行补设计。

## 自验收矩阵（已回填）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| runtime.yml 不再读 environment | `RuntimeFileConfig` 无 environment 字段；`deny_unknown_fields` 拒绝 `environment` 键 | `rg environment runtime/driver` 仅 config/tests.rs 断言拒绝 | `cargo test -p runtime` config::tests 全绿 |
| RuntimeHost 无 environment 字段/生命周期校验 | `RuntimeHost.frozen_profile: OnceLock<String>`；`freeze_bootstrap_profile`；`trusted_profile` | `rg trusted_environment runtime/host/src/host` 无残留；生产路径仅剩 `DbProviderBuildInput.environment`（capability-context 跨 crate 字段，非本节点） | host 生产 lib `cargo check -p skiff-runtime-host --lib` 通过 |
| 首次 bootstrap 冻结 profile，后续不一致 fail closed | `freeze_bootstrap_profile` get_or_init 冻结 + lifecycle/activation 双向校验 | foreign-profile 测试保留（test-only） | 用例：`activation_rejects_profile_other_than_runtime_frozen_domain_before_resolution`、activation_prepare foreign-profile |
| 物化 ConfigView 前校验 snapshot.profile == bootstrap.profile | `validate_snapshot_profile` 在 `build_started_candidate`（materialize）之前调用 | `rg validate_snapshot_environment\|snapshot\.environment runtime/host` 无残留 | recovery `cold_recovery_rejects_config_snapshot_from_another_profile_before_config_views`（test 已同步，host 套件待依赖节点合流后运行） |
| 加密域/KDF/AAD 用 bootstrap profile 值 | `DbEncryptedFieldContext.storage_profile` 贯穿 metadata/mapping/encryption | `rg storage_environment runtime/service-db` 无残留（仅 migration_tool CLI 面保留 environment，属后续工具层节点） | `cargo test -p skiff-runtime-service-db`：145 passed / 0 failed |
| runtime/tests 消费已迁移 corpus | consumers 断言 `activation.profile`，schema 断言 v4 | `rg activation\.environment\|frame-v3 runtime/tests` 无残留 | `cargo test -p runtime` 全 corpus 消费者通过 |
| 写集边界 | 提交 diff 仅限 runtime + 本叶子文档 | `git diff --stat` 核对 | — |

## 自验收证据（命令与结果）

1. `cargo test -p skiff-runtime-service-db`（隔离 CARGO_TARGET_DIR=worktree/target）：
   145 passed / 0 failed / 6 ignored。
2. `cargo check -p skiff-runtime-host --lib`：通过（14 个既有 dead-code 警告，与本节点无关）。
3. `cargo test -p runtime`：完整套件中除 1 个既有环境敏感栈压力测试外全部通过
   （lib 118 + 全部 runtime/tests corpus 消费者）。

### 已知阻塞（不属于本节点写集）

- `cargo test -p skiff-runtime-host --tests` 被基线已损坏的越界 crate 阻断：
  `skiff-config-snapshot-tooling`（producer.rs/projection.rs 仍引用已删除的
  `validate_activation_environment` / `snapshot.environment()`，3 errors），
  其后再到 `skiff-test-runner`（`EnvironmentActivationState` 等未迁移）。两者均为
  后续节点（工具层/test-runner）范围；host 单元/集成测试需其合流后运行。
- `runtime_program_non_tail_recursion_128_layers_fit_diet_stack`（eval 栈压力测试，
  48 MiB debug 栈）在本机 rustc 1.88.0 下进程级栈溢出 abort；本节点 diff 不涉及
  eval 路径，且 `SKIFF_NON_TAIL_DEPTH_STACK_KIB=262144` 时同一测试通过，归类为
  既有环境敏感失败，非本节点回归。

## 交接

完成后提交到 dev/runtime-profile，报告给 `skiff_integration`：
branch、worktree 路径、commit/tree、实际写集、自验收证据与命令、剩余风险。
