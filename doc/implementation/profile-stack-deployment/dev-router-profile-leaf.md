# Leaf Task: 阶段 B Router crate activation 语义 environment → profile（dev/router-profile）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`
  （integration/profile-stack @ 5523aa27 已提交），§2/§4/§6/§7/§12。
- 直接父节点：
  - 设计 §12 评审决议（2026-08-04 用户确认）：wire/schema 版本、health wire、
    Mongo state/audit 索引、durable task `target_profile` 全部随本设计改名；
    阶段 A 验收用限定路径搜索 + 白名单，不允许全局 `rg environment`。
  - 共享 corpus 预检（`dev/shared-corpus-profile`，已并入 ce14b650）：
    `router/src/protocol/*.ts` 是 legacy 非生产代码，唯一消费者是断链的
    `cross-system-fixtures/package-service-ecosystem/verify.mjs`；
    baseline 上 `skiff-router` 无法编译（38 个错误，属于本节点迁移范围）。
- 已合入上游事实（同 baseline ce14b650）：
  - stage A `dev/core-contract-profile`：`skiff-deployment`/`skiff-artifact-model`
    已改为 `ProfileActivationState`/`profile`/`validate_activation_profile`。
  - stage B `dev/runtime-transport-profile`：router→runtime frame v4，
    `activation.profile`。
  - stage C `dev/cli-profile`：scripts/CLI `--profile`。
  - `dev/shared-corpus-profile`：cross-system checkpoint/store corpus 已迁移。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，
  baseline ce14b650）。
- 本分支：dev/router-profile，worktree
  `/Users/geek/workspace/skiff-dev-router-profile`。

## 零 worktree 只读预检结论（ce14b650，Git 对象锚定）

1. 集成分支 HEAD = ce14b650，与任务 baseline 一致；main 在 6d2b13e7，
   无其他并行分支修改 router crate；无 router 相关兄弟 worktree。
2. 上游类型/接口均已 profile 化：
   - `skiff_deployment::activation_state::ProfileActivationState`（字段
     `profile`）、`PrepareInput/CommitInput/AbortInput.profile`、
     `ActivationAuditEvent.profile`；
   - `skiff_artifact_model::validate_activation_profile` =
     `[A-Za-z0-9._-]{1,200}` 且拒绝 `"."`/`".."`；
   - `RuntimeConfigSnapshot::new(profile, ...)`、`snapshot.profile()`；
   - `AssemblyActivationRequest.profile`（v3）、
     `AssemblyActivationControl::*{ profile, .. }`；
   - `RouterBootstrapFrameHeader.activation.profile`（v4）；
   - `TaskExecutionImageRef.target_profile`。
3. Router 是剩余 owner：router/src 中 `environment` 域符号约 322 处、
   router/tests 约 47 个文件；`is_valid_profile` 旧规则
   `[A-Za-z_][A-Za-z0-9_]*` 与 artifact-model 不一致（`prod-us` 将变合法）。
4. 任务可在不改变设计的情况下闭合：全部为 router crate 内机械迁移 +
   明确授权的 verify.mjs 处置；无需触碰 runtime/transport、scripts、文档。

## 写集边界（仅限以下）

1. `router/src/**`：environment 域符号 → profile（config、bootstrap、activation、
   health、session、supervisor、task、telemetry、listener）。
2. `router/tests/**` 与 `router/tests/fixtures/**`、`router/router.example.yml`：
   同步新语义；live probe 的 OS env 变量名同步为 `*_PROFILE`。
3. `cross-system-fixtures/package-service-ecosystem/verify.mjs`：
   legacy 断链 checker。决策：**删除**（理由见下）；不修改其他 cross-system
   fixture。
4. 本叶子任务文件。

禁止修改：`router/README.md`（文档归阶段 E）、其他 crate、scripts、docs。

### verify.mjs 处置决策

删除 `verify.mjs`。依据：
- baseline 三种模式均在模块解析阶段失败（import
  `../../router/src/protocol/*.ts`，该目录自 b9714d7f 起不存在），
  共享 corpus 预检已确证其为 stale legacy checker；
- 其唯一业务负载（checkpoint.json / ecosystem-store-cases.json 断言）已由
  `runtime/transport` activation/wire corpus 与 shared corpus 迁移覆盖；
- 保留一个不再执行任何真实 wire 断言的“最小 checker”会制造假的验证入口，
  不如明确删除；`activationRawCorpus.mjs` 是自包含的 raw case loader，
  保留。

## 关键实现映射（设计 §2/§4/§6.2/§7/§12）

- `RouterConfig.environment` 删除；`profile` 保留必需；
  `is_valid_profile` 收敛为 `validate_activation_profile` 语义。
- E-bootstrap：直接使用 `config.profile` 读取 committed state；
  删除 `EnvironmentMissing` 错误变体（profile 已由 config 校验保证必需）。
- activation repository：`_id == profile`、`state.profile` 唯一索引、
  audit 查询/维护索引键 `profile`；schema
  `skiff-profile-activation-state-v1`。
- health wire：`activeAssembly.profile`、`replicas[].profile`。
- telemetry：`producer_id = router:{profile}`。
- session：`RegisteredAssemblyTuple.profile`、
  `RouterBootstrapActivationFrameHeader.profile`。
- task：消费 `target_profile`。
- 路由 epoch/snapshot：`validate_activation_profile`、`snapshot.profile()`。

## 自验收

1. `cargo test -p skiff-router`（隔离 CARGO_TARGET_DIR，
   `/Users/geek/workspace/.cargo-targets/router-profile`，磁盘预算 ≤12G）。
2. 限定路径反向搜索：`router/src`、`router/tests`、`router/tests/fixtures`、
   `router/router.example.yml` 中无 environment/Environment 域残留
   （白名单：`std::env`、`env::var`、`accessKeyIdEnv`/`accessKeySecretEnv`
   配置键、无 `environment` 子串的 OS env 名）。
3. `git diff --check`、写集核对（仅 router 范围 + verify.mjs + 本文件）。

## 交接

完成后提交到 dev/router-profile，报告给 `skiff_integration`：
分支、worktree、implementation commit/tree、实际写集、自验收矩阵
（设计/任务条款 | 代码证据 | 反向搜索证据 | 测试）、verify.mjs 处置决策。

## 结果与证据（提交前记录）

### 写集

- `router/src/**`：environment 域符号全量迁移到 profile：
  config 删除 `environment` 字段/键、`is_valid_profile` 委托
  `skiff_artifact_model::validate_activation_profile`；bootstrap/supervisor
  删除 `EnvironmentMissing`，E-bootstrap 直接使用 `config.profile`；
  activation repository `_id == profile`、索引 `state.profile`/audit
  `profile`；health wire `activeAssembly.profile`/`replicas[].profile`；
  telemetry `router:{profile}`；session bootstrap/demux/identity/handshake
  使用 `profile`；task admission/sink 消费 `target_profile`；
  epoch/snapshot 校验 `profile`。
- `router/tests/**` 与 fixtures：同步新语义；`invalid-environment.yml`
  重命名为 `unsupported-environment.yml`（拒绝未知键断言），
  `invalid-profile.yml` 改为 `profile: "."`（拒绝 `.` 断言），
  两个 corpus 期望同步；live probe 的 OS env 名改为 `*_PROFILE`，
  router.yml 使用 `profile: <live.profile>`，runtime.yml 不再写
  environment/profile；遗留 `skiff-runtime-frame-v3` 硬编码全部同步为 v4。
- `router/router.example.yml`：删除 `environment: dev`。
- `cross-system-fixtures/package-service-ecosystem/verify.mjs`：删除
  （legacy 断链 checker；决策见上）。
- 本叶子任务文件。

未修改：`router/README.md`（文档归阶段 E）、其他 crate、scripts、
其他 cross-system fixture。

### 反向搜索（限定路径）

`router/src`、`router/tests`、`router/tests/fixtures`、
`router/router.example.yml` 中仅剩两处白名单用途：
1. `invalid/unsupported-environment.yml` + corpus.json 同名条目：
   负例断言 `environment` 顶层键被拒绝；
2. `bootstrap_live_probe.rs` 的 `"skiff-environment-activation-state-v1"`
   legacy schema 种子：负例断言旧命名空间 fail closed。

### 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| config 删除 environment、profile 必需、validator 收敛 | `router/src/config/mod.rs`（TOP_LEVEL_KEYS/RouterConfig/is_valid_profile） | 无 environment 域残留；unsupported-environment 负例 | `cargo test -p skiff-router`（config_corpus 4/4） |
| E-bootstrap 按 config.profile 读取 committed state | `router/src/bootstrap/assembly.rs`（`config.profile.clone()`） | 无 EnvironmentMissing | bootstrap_production_wiring 6/6、bootstrap_* 全过 |
| repository `_id=profile`、唯一索引、audit 索引 | `router/src/activation/{repository,index}.rs` | 索引键/名均为 profile | activation_repository_contract 5/5、activation_mongo_probe（ignored live） |
| health wire `profile` | `router/src/health/wire.rs` | 无 environment 字段 | health_http 3/3、health_projection 5/5 |
| telemetry `router:{profile}` | `router/src/telemetry.rs` | 无环境残留 | task_telemetry 5/5 |
| session 消费 v4 activation.profile | `router/src/session/{bootstrap,demux,identity,handshake}.rs` | frame v3 硬编码已清零 | session_* 全过、w_model_* 全过 |
| task 消费 target_profile | `router/src/task/{admission,sink}.rs` | `target_environment` 清零 | task_* 全过 |
| tests/fixture 同步 | router/tests + fixtures diff | 限定路径搜索仅白名单 | 完整套件 |
| verify.mjs legacy 断链处置 | 文件删除 | 无仓库内引用 | 不适用 |

### 完整套件

`CARGO_TARGET_DIR=/Users/geek/workspace/.cargo-targets/router-profile
cargo test -p skiff-router`：全 target 0 failed；lib 69、bin 2、
全部集成测试套件通过；`#[ignore]` 的 live probe（需外部 Mongo/harness）
不在默认运行范围。`cargo check --all-targets` 通过；`git diff --check` 通过。
