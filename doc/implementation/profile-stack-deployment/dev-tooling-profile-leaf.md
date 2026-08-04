# Leaf Task: 阶段 C Rust 侧 profile 语义（dev/tooling-profile）

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`
  （integration/profile-stack @ b447dcc9 已提交），§2/§6/§7/§12，§2 含
  test-runner 例外（config profile 固定 `skiff-test`，激活 target 与 config profile 分离）。
- 直接父节点：阶段 C 工具层任务（父任务派发：config-snapshot-tooling、compiler
  authoring、test-runner 的 activation 语义从 environment 改为 profile）；
  router/runtime/transport/scripts 已按 profile 合流（baseline b447dcc9）。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，HEAD b447dcc9）。
- 本分支：dev/tooling-profile，worktree
  `/Users/geek/workspace/skiff-dev-tooling-profile`。

## 零 worktree 只读预检结论（基线 b447dcc9，Git 对象锚定）

### 1. 基线状态

- `integration/profile-stack` HEAD == b447dcc9，与任务 baseline 一致；共享主 worktree
  main 未受影响；并行 worktree `skiff-profile-stack-integration`（集成 Agent）、
  `skiff-dev-testinfra` / `skiff-integration-testinfra`（无关批次）。本节点写集与它们无重叠。

### 2. 真实入口与关键调用链

1. config-snapshot-tooling：`main.rs`（CLI）→ `ConfigSnapshotProductionInput`
   （同时含 environment/profile）→ `producer.rs`（`validate_activation_environment`）→
   `projection.rs`（`project_runtime_config_snapshot[_with_base]`）→
   `RuntimeConfigSnapshot::new(profile, ...)`；`source.rs` 有本地 `validate_profile`
   （无 200 长度上限）。
2. compiler authoring：`bin/skiff-compiler.rs`（`--environment`）→ `authoring.rs`
   `build_authoring_object`（package，`_environment` 未用）与 `project_runtime_assembly`
   （assembly receipt `"environment"`、`release = environment`、
   `RuntimeAssemblyPointerPath::new(release)` → `pointers/runtime-assemblies/<profile>.json`）。
3. test-runner：`src/main.rs`（`--environment`、`SKIFF_TEST_ENVIRONMENT` 非 live 兜底、
   `validate_environment`）→ `SkiffTestOptions.target_environment` →
   `runtime_execution.rs`（`activation_request_body` 写 v2/`environment`）→
   `runtime_execution/wire.rs`（frame v3、`EnvironmentActivationState`、health/replica
   `environment`）→ `readiness.rs`（environment 校验）→ `canonical_store.rs`
   （`config_snapshot.environment()` 与 target 校验）。
4. test-runner fixture：`bin/package_service_smoke_fixture.rs`
   （`--environment`、`--initialize-environment`、`EnvironmentActivationState`、
   `initialize_environment_activation`）；`package_service_host_fixture.rs`
   （receipt `environment`、`config.{environment}.yml`）；
   `test_service_fixture.rs`（`target_environment` 仅作 snapshot/activation target；
   config 内容由 `test_service_profile.profile_name`（固定 `skiff-test`）选择，§2 例外已成立）。

### 3. 核心 crate 现状（阶段 A/B 已合流，本节点直接消费）

- artifact-model：只有 `validate_activation_profile`（`[A-Za-z0-9._-]{1,200}`，
  拒绝 `.`/`..`）；`ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION == v3`。
- runtime-config-snapshot：`RuntimeConfigSnapshot::new(profile, ...)` / `profile()`；
  record v3。
- deployment：`ProfileActivationState::initial(profile, ...)`、
  `store.initialize_profile_activation`、`PROFILE_ACTIVATION_STATE_SCHEMA_VERSION ==
  skiff-profile-activation-state-v1`。
- artifact-identity：`ProfileActivationStatePath`（`profiles/<profile>/activation.json`）、
  `RuntimeAssemblyPointerPath`（safe_segment 规则与 profile 一致）。
- router/transport：无 `environment` 残留；health counters 为
  `activeRoutingEpoch.active.profile`、`activation.profile`、`repository.profile`；
  frame v4。

## 任务边界与实现决策

### 写集

1. config-snapshot-tooling：CLI 删除 `--environment`，只接收 `--profile`；
   `ConfigSnapshotProductionInput` 删除 environment；`producer.rs` 用
   `validate_activation_profile`；`projection.rs` 参数/消息/`RuntimeConfigSnapshot::new`
   全部改 profile；`source.rs` 的 `validate_profile` 收敛为直接调用
   `skiff_artifact_model::validate_activation_profile`；tests 同步。
2. compiler/driver：`build_authoring_object`/`project_runtime_assembly` 参数改 profile；
   assembly receipt 字段 `"environment"` → `"profile"`；release/pointer key 来自
   profile（`RuntimeAssemblyPointerPath::new(profile)`）；`bin/skiff-compiler.rs`
   `--environment` → `--profile`（package 可选默认 dev，assembly 必填）；bin/authoring
   tests 同步；`package_publication.rs` 注释中 environment 提法改为 profile。
3. test-runner：
   - `main.rs`：`--environment` → `--profile`；`validate_environment` →
     `validate_profile`（委托 artifact-model validator）；`target_environment` →
     `target_profile`；`SkiffTestOptions.target_environment` → `target_profile`。
   - 决策：OS env 键 `SKIFF_TEST_ENVIRONMENT` 保持原名。它是 OS env 通道（§12 反向
     搜索白名单含 OS env）；scripts（已合流、本节点外）在
     `scripts/lib/isolated-test-runtime-instance.mjs` 设置并在 scripts tests 断言该键，
     改名需触碰 scripts，超出本节点边界；不构成旧 CLI/artifact 兼容层。
   - `canonical_store.rs`：`target_profile` + `config_snapshot.profile()`。
   - `bin/package_service_smoke_fixture.rs`：`--environment` → `--profile`、
     `--initialize-environment` → `--initialize-profile`（无外部消费者）、
     `EnvironmentActivationState` → `ProfileActivationState`、
     `initialize_environment_activation` → `initialize_profile_activation`、
     `initialize_empty_environment` → `initialize_empty_profile`、receipt JSON
     `"environment"` → `"profile"`、`RuntimeConfigSnapshot::new(profile, ...)`。
   - `package_service_host_fixture.rs`：environment → profile（receipt JSON、
     `config.{profile}.yml`；该 fixture 的 base 服务是普通服务，config profile == 激活
     profile 符合 §3.4；test service 的固定 `skiff-test` 例外在
     `canonical_package.rs`/`test_service_fixture.rs` 保留）。
   - `test_service_fixture.rs`：`target_environment` → `target_profile`（仅激活 target；
     config 选择仍用 `test_service_profile.profile_name`）。
   - `runtime_execution.rs`：activation request v2/`environment` →
     v3/`profile`；`runtime_execution/wire.rs`：frame v3 → v4、
     `EnvironmentActivationState` → `ProfileActivationState`、health/replica
     `environment` → `profile`；`readiness.rs`：environment → profile。
   - fixtures：`fixtures/http-entry-test-service/run.mjs` 用 `stack.profile`、
     `seedBootstrap({ profile })`、CLI `--profile`。
   - tests 同步：`runtime_execution/tests/{support,wire,readiness,orchestration,batching}`、
     `tests/test_service_flow/{runner_cli_contract,config_snapshot,base_assembly}`、
     `tests/canonical_std_seed_bootstrap.rs`；runner_cli_contract 中 `--profile` 从
     retired 移到 help 必需，`--environment` 移入 retired。

### 强制停止

- 需触碰 router/runtime/transport/scripts/skiff stack 或文档时停止上报；
  `SKIFF_TEST_ENVIRONMENT` 改名的 scripts 协调问题同理。
- 发现设计空洞或需用户决策的语义变化时停止，不自行补设计。

## 自验收矩阵（已回填）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| config-snapshot CLI 只接收 --profile；source 校验与 artifact-model 一致 | `Arguments` 无 environment（main.rs:42）；`validate_profile` 委托 `validate_activation_profile`（source.rs）；`ConfigSnapshotProductionInput` 仅 profile；`RuntimeConfigSnapshot::new(profile,...)`（projection.rs） | `rg environment config-snapshot-tooling` 仅剩 tests.rs 断言拒绝 `--environment` | `cargo test -p skiff-config-snapshot-tooling`：12 passed / 0 failed |
| compiler authoring 用 profile 作 release/pointer key，receipt 为 profile | `project_runtime_assembly`/`_to_store` 参数 profile；receipt `"profile"`（authoring.rs:308）；`RuntimeAssemblyPointerPath::new(profile)`；bin `--profile` | `rg environment compiler/driver` 无残留 | `cargo test -p skiff-compiler --lib authoring` 17 passed + `--bin skiff-compiler` 7 passed + `cargo check --all-targets` PASS |
| test-runner CLI/activation/health 全部 profile；frame v4 | `main.rs` `--profile`/`validate_profile`；`SkiffTestOptions.target_profile`；`activation_request_body` v3/profile；wire frame v4 + `ProfileActivationState`；readiness/canonical_store profile；smoke/host fixture profile | `rg environment test-runner` 仅剩 `SKIFF_TEST_ENVIRONMENT`（OS env，§12 白名单）与 retired `--environment` 负例 | `cargo test -p skiff-test-runner`：96 passed / 0 failed / 2 ignored（lib 75 + main 3 + bootstrap 1 + live HTTP entry 1 + flow 16） |
| §2 例外保留：config profile 固定 skiff-test，激活 target 独立 | `load_test_service_run_config` 用 `test_service_profile.profile_name`（test_service_fixture.rs:172）；`test_service_config_snapshot` 用 target_profile | `config.skiff-test.yml` fixture 保留；`canonical_package.rs::TEST_SERVICE_CONFIG_PROFILE` 未改 | config_snapshot 测试断言 `profile() == "skiff-test"` PASS |
| 写集边界 | 提交 diff 仅限 config-snapshot-tooling、compiler/driver、test-runner + 本叶子文档 | `git status --short` 核对 32 个文件 | — |

## 自验收证据（命令与结果）

```text
cargo check -p skiff-config-snapshot-tooling -p skiff-compiler -p skiff-test-runner --all-targets
  -> Finished dev profile（仅既有 warnings）

cargo test -p skiff-config-snapshot-tooling
  -> lib 9 passed + bin 3 passed + doc 0；0 failed

cargo test -p skiff-compiler --lib authoring
  -> 17 passed；0 failed

cargo test -p skiff-compiler --bin skiff-compiler
  -> 7 passed；0 failed

cargo test -p skiff-test-runner
  -> lib 75 passed / 2 ignored；main 3 passed；bootstrap 1 passed；
     http_entry_test_service 1 passed（真实隔离 router/runtime/Mongo，85s）；
     test_service_flow 16 passed；合计 96 passed / 0 failed
```

说明：`cargo fmt --all` 曾误格式化 6 个 runtime 文件（纯重排、非本节点写集），
已用 `git restore --source=HEAD` 精确恢复；最终写集与上方矩阵一致。
`SKIFF_TEST_ENVIRONMENT` 为 OS env 键（scripts 已合流并在 scripts tests 断言该键），
保持原名，不在旧 CLI/artifact 兼容层范围内。
