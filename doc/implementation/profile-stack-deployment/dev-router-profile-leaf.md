# Leaf Task: 阶段 B Router crate activation 语义 environment → profile（dev/router-profile）

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
