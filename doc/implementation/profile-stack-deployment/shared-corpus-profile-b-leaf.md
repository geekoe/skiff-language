# Leaf Task: 阶段 B 共享 corpus/checkpoint 检查点（shared-corpus-profile-b）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`（integration/profile-stack
  @ 5523aa27 已提交），§2/§4/§6/§7/§12。
- 直接父节点：设计 §12 评审决议（2026-08-04 用户确认）——corpus/golden 全量迁移；
  checkpoint.json / ecosystem-store-cases.json 仍含旧 environment/frame-v3。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，基线
  5523aa27）。
- 本分支：dev/shared-corpus-profile，worktree
  `/Users/geek/workspace/skiff-dev-shared-corpus-profile`。

## 任务边界

只迁移 Router/Runtime 并行节点的共同前置 corpus/checkpoint。写集仅限：

1. `cross-system-fixtures/package-service-ecosystem/checkpoint.json`
2. `cross-system-fixtures/package-service-ecosystem/ecosystem-store-cases.json`
3. `cross-system-fixtures/package-service-ecosystem/verify.mjs`
   （仅机械同步断言中的字段名/期望值）

不改 `router/src/protocol/*.ts`、不改 `scripts/**`、不改
`activation-raw-cases.json`（legacy 拒绝语料，保留）。

## 零 worktree 只读预检结论（基线 5523aa27，Git 对象锚定）

### 1. checkpoint.json / ecosystem-store-cases.json 消费者

- `checkpoint.json`：唯一代码消费者为
  `cross-system-fixtures/package-service-ecosystem/verify.mjs`
  （`readJson("checkpoint.json")`，`--self-test` 与
  `--runtime-wire-self-test` 两个模式断言其内容）。`git grep` 未发现任何
  Rust/TS production 测试消费 checkpoint.json。
- `ecosystem-store-cases.json`：唯一代码消费者为
  `verify.mjs --runtime-wire-self-test`（`readJson("ecosystem-store-cases.json")`）。
  未发现 Rust 测试消费；基线树中 `__ecosystem-store` adapter 实现已不存在
  （`compiler/driver/ecosystem_store/` 仅剩 `tests/fixtures.rs`），该 corpus
  描述的是已被取代的 legacy test sidecar。
- 基线 5523aa27 上 `verify.mjs` 不可运行：顶部 import
  `../../router/src/protocol/*.ts`，而 `router/` 在基线已是纯 Rust，
  `router/src/protocol/` 不存在（TS Router 在 b9714d7f
  "cutover-delete TS Router" 已删除，且 b9714d7f 是基线祖先）。所有三种模式
  都会在模块解析阶段失败。结论：checkpoint 与 ecosystem-store-cases 当前没有任何
  可运行的 checker；verify.mjs 是 stale legacy checker。

### 2. router/src/protocol/*.ts 生产/legacy 结论

- **legacy，非生产代码**。基线 5523aa27 的 `router/` 目录为纯 Rust
  （249 个文件，无 `src/protocol/*.ts`），TS Router 已被 b9714d7f 删除；
  `git grep` 在 router/src、scripts、test-runner、runtime、compiler 等
  production 路径未发现对 `router/src/protocol/*.ts` 的引用。
- 唯一残留引用是 `cross-system-fixtures/package-service-ecosystem/verify.mjs`
  的 import，属于 legacy checker 消费。不恢复、不修改这些 TS 文件；其
  retire/rewire 归属 Router 节点或后续清理节点。

### 3. activation-raw-cases.json 的 14 处 environment

- 全部 14 处均为 `target: "state"`、`outcome: "reject"` 的拒绝语料，
  内容是 `"schemaVersion":"skiff-environment-activation-state-v1"`（旧
  activation state schema 名）。
- 设计 §6.2 规定 activation state 使用新命名空间
  `skiff-profile-activation-state-v1`，不保留旧 environment 兼容层；
  因此这些 legacy schema 拒绝语料应原样保留，不迁移。

## 迁移映射（机械）

### checkpoint.json

- `runtimeAssemblyProjection.argv`：`--environment <environment>` →
  `--profile <profile>`。
- `pointerPaths.EnvironmentActivationState` →
  `ProfileActivationState`，值
  `environments/<environment>/activation.json` →
  `profiles/<profile>/activation.json`。
- `activationStateFields.state`：`environment` → `profile`。
- `activationRequest.schemaVersion`：`skiff-assembly-activation-request-v2` →
  `skiff-assembly-activation-request-v3`；`fields` 中 `environment` → `profile`。
- `controlWireFields` 各变体：`environment` → `profile`。
- `runtimeAssemblyFrame.schemaVersion`：`skiff-runtime-frame-v3` →
  `skiff-runtime-frame-v4`。
- `ecosystemStoreAdapter.operations`：environment → profile
  （`ensureEnvironmentBootstrap` → `ensureProfileBootstrap`、
  `readEnvironment` → `readProfile`、`prepareEnvironment` → `prepareProfile`、
  `abortEnvironment` → `abortProfile`、`commitEnvironment` → `commitProfile`）。

### ecosystem-store-cases.json

- workflow 请求字段 `environment` → `profile`，operation 名按上述映射同步；
- invalidRequests 中的 operation/字段名同步（`latestEnvironment` →
  `latestProfile`、`readEnvironment` → `readProfile` 等），保持拒绝语义不变。

### verify.mjs

- 仅同步断言中引用的字段名/期望值：request keys、argv、activationRequest
  fields、pointerPaths 键与值、workflow operation 列表。
- 不触碰已删除 TS 模块的 import 行及其解码器符号名（属 legacy checker
  结构，不是字段名同步；恢复/删除属 Router 节点范围）。

## 自验收

1. `node cross-system-fixtures/package-service-ecosystem/verify.mjs
   <--self-test|--combined-probe|--runtime-wire-self-test>`：
   记录其在模块解析阶段失败的精确证据（导入已删除 TS 模块），不视为本节点
   回归。
2. 结构性校验：两个 JSON 均保持合法 JSON，且
   `ecosystem-store-cases.workflow[].operation` 与
   `checkpoint.ecosystemStoreAdapter.operations` 一致；无残留旧字段
   （仅允许 activation-raw-cases.json 保留 legacy 拒绝语料）。
3. 聚焦 Rust corpus gate（证明共享 corpus 状态一致，这些 corpus 不消费本节点
   两个 JSON）：`runtime/transport` activation/wire 相关测试与 `router`
   activation coordinator corpus 测试。完整 router/runtime 套件留给各自节点。
4. `git diff --check` 与 `git status` 写集核对。

## 交接

完成后提交到 dev/shared-corpus-profile，报告给 `skiff_integration`：
写集（3 个文件）、预检结论、自验收证据、router/src/protocol legacy 结论。

## 结果与证据（提交前记录）

### 写集

- `checkpoint.json`：argv `--profile <profile>`；pointerPaths 键
  `ProfileActivationState`、值 `profiles/<profile>/activation.json`；
  activationStateFields.state、activationRequest fields、
  controlWireFields 全部 `environment` → `profile`；activation request
  schema `v2` → `v3`；runtime frame schema `v3` → `v4`；
  ecosystemStoreAdapter.operations 全部改为 profile 命名。
- `ecosystem-store-cases.json`：workflow/invalidRequests 的 operation 名与
  `environment` 字段全部 `environment` → `profile`（`readRouterSnapshot`
  请求原样无 profile 字段）。
- `verify.mjs`：仅同步 request keys、argv、activationRequest fields、
  pointerPaths 键/值、store workflow operation 列表；未触碰已删除 TS 模块
  的 import/解码器符号名。

### 结构校验

- 两个 JSON `JSON.parse` 通过；checkpoint 与 ecosystem-store-cases 全文不含
  `environment`/`Environment`（大小写均无）。
- `ecosystem-store-cases.workflow[].operation` 序列与
  `checkpoint.ecosystemStoreAdapter.operations` 的排列一致
  （workflow 为带重复操作的执行序列，checkpoint 为去重操作清单，两者语义一致）；
  argv 一致。
- `git diff --check` 通过；`git status` 写集仅 3 个 corpus 文件 + 本叶子文档。

### verify.mjs 可运行性证据

三种模式 `--self-test` / `--combined-probe` / `--runtime-wire-self-test`
均在模块解析阶段失败：

```text
ERR_MODULE_NOT_FOUND: Cannot find module
'.../router/src/protocol/assemblyActivationProtocol.ts'
imported from .../verify.mjs
```

该失败在迁移前已存在（基线 5523aa27 无 `router/src/protocol/`），不是本节点
回归。结论：verify.mjs 是引用已删除 TS Router 模块的 stale legacy checker，
retire/rewire 由 Router 节点或后续清理节点负责。

### 聚焦 Rust corpus gate

- `cargo test -p skiff-runtime-transport --test activation_transaction_corpus`
  （隔离 CARGO_TARGET_DIR）：2 passed / 0 failed。该测试消费
  activation-transaction-cases.json / runtime-wire.json / control-wire.json，
  与 checkpoint 新语义一致。
- `cargo test -p skiff-router --test activation_coordinator_corpus`：
  基线 5523aa27 的 `skiff-router` crate 本身无法编译（38 个错误，
  分布在 router/src/task、session、bootstrap、activation、telemetry、
  test_dispatch 等约 15 个文件；典型：`TaskExecutionImageRef` 无
  `target_environment` 字段、`EnvironmentActivationState` 未迁移等）。
  这是 Router 并行节点的待迁移范围，与本节点写集无关；完整 router 套件
  留给 Router 节点。
