# Leaf Task: task-control crate（durable task dispatch 共享契约检查点）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（TaskStore 是 task
  identity / state / `due_at` / attempt / lease / 取消结果 / terminal outcome
  的权威 owner；所有 transition 都是对当前 state / lease 的 conditional write；
  store authority time 上 CAS；TaskId-idempotent create；terminal settlement
  幂等且拒绝同 lease 冲突 outcome）。
- 用户面契约：`doc/reference/dispatch.md`（`TaskStatus` / `TaskCancelResult`
  的公开 kind 拼写）。
- 直接父节点：批次 `task-control-integration`，父文档
  `doc/implementation/task-control-batch.md`（集成 Agent `/root/task_control_integration`）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`。
- baseline：`25e430f5967c704106994e609f281797dbe6c42b`（main HEAD，工作区 clean）。
- worktree：`/Users/geek/workspace/skiff-task-store`，branch `task-store`。

## 任务合同摘要

新建 workspace crate `task-control`（包名 `skiff-task-control`），实现 durable
task dispatch 核心的 TaskStore 权威 owner 共享检查点，供后续 scheduler
（`/root/task_scheduler`）消费：

1. canonical model：`TaskId` / `AttemptId` / `LeaseId` opaque newtype；
   `TaskState`（scheduled / ready / leased / succeeded / failed /
   platform-failed / canceled）；`TaskLease` / `TaskTerminal` / `TaskRecord`；
   `TaskStatus` / `TaskCancelResult` 公开 kind 枚举（拼写与 reference 一致）。
2. `TaskStore` trait：durable create（TaskId-idempotent、同 TaskId 不同
   canonical record 冲突拒绝）、conditional claim、renew、settlement（幂等、
   拒绝 stale/冲突）、cancel、lease expiry recovery、due scan、status 查询
   （retention 过期返回 expired）。可替换存储（in-memory fake + Mongo adapter）。
3. Mongo adapter：全部 conditional write/CAS；`(state, due_at)` 索引与 TaskId
   unique（`_id`）；store authority time（`$expr` + `$$NOW` 服务端判定，不依赖
   客户端时钟）；短时不可用归类为暂时性错误。
4. 测试矩阵第 5–14 条：in-memory fake 契约测试 + Mongo live probe（按仓库
   live-gate 惯例 `#[ignore]` + env var，不默认在 CI 展开）。

## 预检结论（只读，baseline 25e430f5）

- crate 命名：现有 workspace crate 均为 `skiff-` 前缀（`skiff-artifact-model`、
  `skiff-deployment`、`skiff-router`）；目录名 `task-control/`，包名
  `skiff-task-control` 一致。
- 身份类型对接（git grep 锚定 baseline）：
  - `DetachedCallTarget::Function` 对接 `skiff_artifact_model::PackageCallableId`
    （现有 canonical callable identity）。
  - `DetachedCallTarget::ActorMethod` 的 `ActorIdentity` 对接
    `skiff_deployment::projection::actor_routing::ActorRoutingRef`
    （`service_id` + `actor_abi_identity`，稳定 actor ref）；implementation /
    method 复用 `ActorImplementationIdentity` / `ActorMethodIdentity`。
  - `TaskExecutionImageRef` 复用 `RuntimeAssemblyRef` /
    `RuntimeConfigSnapshotRef` / `ServiceDeploymentRef`，另含
    `target_environment` 与 `package_version` 显式版本事实。
  - `RecoverablePayload` 定义为 opaque bytes newtype（存储层不解释 payload，
    不依赖 runtime execution）；`ActorActivationSnapshot` 含 key / create input
    / expected type plan（复用 `skiff_artifact_model::RecoverableExpectedTypePlan`）。
- Mongo 访问模式：参考 `router/src/activation/repository.rs`
  （DTO + pure reducer + Mongo adapter）；router 配置 `service_db.mongo_url`、
  `mongodb = "3.2"`（lock 3.6.0）；bson 2.x `DateTime::from_millis` / `Binary`。
- live-gate 惯例：`#[ignore = "..."]` + harness env var，`--ignored` 显式运行；
  参考 `router/tests/activation_mongo_probe.rs`。
- 新增 workspace crate 必须归入恰好一个 subject：按 AGENTS.md 要求登记
  `scripts/lib/verify-rust-subjects.mjs` 的 `router` subject（该 crate 将被
  router 消费）。

## 写集（planned）

- `task-control/Cargo.toml`、`task-control/src/*.rs`、`task-control/tests/*.rs`。
- 根 `Cargo.toml`：members 增加 `"task-control"`。
- `scripts/lib/verify-rust-subjects.mjs`：router subject 增加
  `rustPackage('task-control', 'skiff-task-control')`（AGENTS.md 要求的机械闭合）。
- `scripts/tests/verify-taxonomy.test.mjs`：router subject 硬编码 deepEqual 同步
  新包条目（AGENTS.md subject 注册的测试 fixture 机械闭合）。
- 本叶子文件 `doc/implementation/task-control-store-leaf.md`。
- `Cargo.lock` 随 cargo 更新（只新增 `skiff-task-control` 条目与既有依赖引用）。

## 禁止

- 不改 runtime-transport wire（`task.submit.request` 等保持现状）。
- 不改 compiler / runtime / router 既有 `task.*` wire sink 模块
  （`router/src/actor/task.rs`、`task_sink.rs` 等）。
- 不改 `doc/reference/`、`doc/architecture/` 与 `doc/implementation/**` 既有文件
  （本叶子文件为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree。

## 自验收矩阵

实际写集（commit 后与交接报告一致）：

```text
Cargo.lock
Cargo.toml
doc/implementation/task-control-store-leaf.md
scripts/lib/verify-rust-subjects.mjs
scripts/tests/verify-taxonomy.test.mjs
task-control/Cargo.toml
task-control/src/{lib,clock,error,memory,model,mongo,reducer,retry,store}.rs
task-control/tests/{memory_contract,mongo_probe}.rs
task-control/tests/support/{mod,contract,fixtures}.rs
```

覆盖范围：契约矩阵第 5–14 条为完整覆盖（fake 确定性 + Mongo live 同一 runner）；
未跑完整 `pnpm verify`（按任务约定只做聚焦验证）。

### 自验收矩阵（设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令）

| 条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| canonical model：TaskId/AttemptId/LeaseId/ServiceOwner opaque newtype；TaskState 七态；TaskLease/TaskTerminal/TaskRecord 字段所有权；TaskExecutionImageRef 五要素；DetachedCallTarget function/actorMethod 对接既有身份类型 | `task-control/src/model.rs`（TaskState 七态 + `as_str`；TaskRecord 字段与权威文档一致；TaskStatusKind/TaskCancelResultKind 与 reference 拼写一致，`model::tests::status_kinds_match_reference_spelling`） | `rg -n "TaskState" task-control/src` 仅 crate 内部；`git diff --name-only` 无 doc/reference、doc/architecture、runtime/transport、router 既有 task 模块 | `cargo test -p skiff-task-control`（19 unit + 3 memory contract + 1 ignored probe） |
| TaskStore trait：create（TaskId-idempotent、冲突拒绝）、claim CAS、renew、settlement（幂等/冲突拒绝）、cancel、lease expiry recovery、due scan、status（retention→expired） | `task-control/src/store.rs`（trait + input/outcome 类型）；`task-control/src/reducer.rs` 纯 transition；`task-control/src/memory.rs` fake | `rg -n "DuplicateTaskId\|AlreadySettled\|AlreadyStarted\|Expired" task-control/src/store.rs` 覆盖全部结果 kind | `cargo test -p skiff-task-control --test memory_contract`（contract matrix 全过） |
| Mongo adapter：全部 conditional write/CAS；`(state, dueAt)` 索引 + `_id` unique；store authority time（`$$NOW`）；短时不可用→Transient | `task-control/src/mongo.rs`（claim/settle/renew/cancel/recover 全部 `find_one_and_update` CAS filter；`task_state_due_at_index`；`is_duplicate_key`；`map_driver_error`→Transient）；`mongo::tests::record_document_round_trips_all_authority_fields` | `rg -n '\$\$NOW' task-control/src/mongo.rs` 出现在 due/expiry/settle/scan/status 的所有 authority 判定；无客户端 `SystemTime` 参与 CAS | `SKIFF_TASK_CONTROL_MONGO_URL=... cargo test -p skiff-task-control --test mongo_probe -- --ignored`（真实 rs0，PASS 11.55s） |
| 测试矩阵 5–14：create 幂等/冲突；claim CAS；expiry vs settlement 竞争；renew/stale；cancel/claim 双向；terminal 幂等/冲突；due visibility/回拨；非法 transition；永久→platform-failed | `task-control/tests/support/contract.rs`（11 个场景，与 `tests/support/fixtures.rs` 共用）；fake 注入时钟 `tests/support/mod.rs`（FakeClock/TestTime） | `rg -n "run_contract" task-control/tests` 两个入口（memory_contract、mongo_probe）同一 runner | 见上两行 |
| 错误分类：永久 → platform-failed 判定入口；暂时性 → 可重试 | `model::PlatformErrorClass`（`terminal_outcome`）；`error::TaskStoreError::is_retryable`；`retry::TaskRetryPolicy`；memory `fail_next_transient` 注入测试 | `rg -n "PlatformFailed" task-control/src` 收敛路径；`rg -n "Transient" task-control/src` 分类/重试 | `cargo test -p skiff-task-control retry::` + `memory_contract::transient_store_failures_are_retryable` |
| 新 workspace crate 必须归入恰好一个 subject | `scripts/lib/verify-rust-subjects.mjs` router subject + `scripts/tests/verify-taxonomy.test.mjs` 同步 | `git diff --name-only` 无其它 scripts 文件 | `node scripts/tests/verify-taxonomy.test.mjs`（9/9 PASS） |
| 不改 wire / router task sink / 既有文档 / 不 push | 写集见上，全部新增或注册性一行修改 | `git diff --name-only \| grep -E 'runtime/transport\|router/src/actor/task\|doc/reference\|doc/architecture'` 为空（exit 1） | `git diff --check` 通过 |
