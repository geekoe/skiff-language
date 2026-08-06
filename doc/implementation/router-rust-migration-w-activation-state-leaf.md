# Router Rust Migration W-activation-state Leaf Task

日期：2026-08-02

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。
节点：W-activation-state-repository（Router-owned durable activation state，一次性有界会话）
Agent：`/root/dev_w_activation_state`
集成目标：`/root/router_rust_integration_b4`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-4.md`
  （W-activation-state 节点、写边界、验证 owner、退出检查点）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）
  - §2.2 第三类 model：Router/platform durable activation model，Mongo adapter Router-owned，
    Runtime 不消费 durable record；
  - §3.2 owner 表：`ActivationStateRepository` 唯一拥有 durable DTO/revision/audit、Mongo indexes、
    read/CAS/retry；不拥有 coordinator transaction、routing epoch；
  - §4.1/§4.2 durable authoritative 语义（commit CAS 发出后 outcome 由 durable state 决定；
    cold recovery 由 durable pending 驱动）；
  - §5.3 `C-router-activation-state` → `W-activation-state-repository` → `P-activation-state`；
  - §8 `router-activation-mongo-live` / `router-live:activation-mongo`（P-activation-state slice 起）。
- 契约文档（frozen，batch 3 已合入基线）：
  `doc/implementation/router-rust-migration-c-router-activation-state.md`、
  `doc/implementation/router-rust-migration-c-model-activation.md`、
  `doc/implementation/router-rust-migration-c-activation-coordinator.md`、
  `doc/implementation/router-rust-migration-contracts-activation-leaf.md`。
- 仓库：`/Users/geek/workspace/skiff`
- Baseline：`main@7683b7c8`（`git rev-parse 7683b7c8` =
  `7683b7c8007a374ae07cb62c7723ced62929100b`，已核对）
- Worktree：`/Users/geek/workspace/wt-w-activation-state`，branch
  `feat/router-rust-w-activation-state`

## 零 worktree 只读预检结论

1. 基线锚定成功：`main` = `7683b7c8`。主 worktree 已被集成 Agent 移到
   `integration/router-rust-migration-batch-4`（仅多批次文档），不影响本节点；兄弟 worktree
   `wt-contracts-ws` 与本节点无文件重叠。
2. durable DTO v2 已存在且为冻结形态：`deployment/src/storage/activation.rs` 的
   `EnvironmentActivationState`（v2）/`CommittedActivation`/`PendingActivation`，
   canonical JSON strict parse、`validate()`、`recovery_action`、文件 adapter
   `CanonicalArtifactStore` 的 read/prepare/abort/commit CAS 与幂等 replay 均已实现并有测试。
3. TS 侧现状（迁移事实，不作为实现目标）：`router/src/router/assemblyActivationStateStore.ts`
   （port）、`assemblyActivationStateReducer.ts`（reducer）、
   `mongoAssemblyActivationStateStore.ts`（Mongo store：state 文档 `_id`/revision/state +
   事务内 audit + 幂等 replay + audit 失败回滚 + 重试不重复 audit 的 TS 测试）。Rust 实现按
   冻结契约采用 contract 形状（collection `activation_state`/`activation_audit`、
   `operation`/`outcome` audit event），不复制 TS 字段形态。
4. 缺失项（本节点补）：适配器无关的 pure reducer 与 audit event DTO（canonical 位置缺失，
   补到 `deployment/src/activation-state/`，与 A1 的 `projection` 模块不重叠）；Router-owned
   Mongo repository adapter、retry policy、health（`router/src/activation/`）；临时 Mongo
   replica set 探针 harness（仓库既有 `scripts/lib/*live-harness*` 约定）。
5. Mongo driver：workspace `Cargo.lock` 已有 `mongodb 3.6.0`（`runtime/service-db`、
   `runtime/host` 消费，满足 `mongodb = "3.2"`）；本节点把 `mongodb` 加入 `skiff-router`
   直接依赖，不新引入版本。`skiff-deployment` 依赖 closure（artifact-identity/model、
   canonical-json、fs2、sha2）不包含宽 Runtime execution model，符合 §2.3。
6. 临时 Mongo 约定：`scripts/lib/local-port-lease.mjs`（`leaseLocalPorts`/`assertPortsClosed`，
   45000-45999 动态端口租约）、`scripts/lib/mongosh-json-command.mjs`（mongosh JSON probe）、
   `scripts/lib/encrypted-storage-live-instance-resources.mjs`（临时目录 + 端口租约 +
   FORBIDDEN_PORTS：27017、4000-4007、44000-44999）。本节点按同一约定自建临时 mongod
   replica set（独立 dbPath/port，用后清理），不触碰 stable Mongo/instance/PM2/4004-4007。
7. 任务可在不改公共契约、不改冻结 DTO 字段、不动 main.rs/listener.rs 的前提下闭合。
   无设计空洞；不返回 TASK_SCOPE_EXPANDED / TASK_NOT_EXECUTABLE。

## 交付物（写集）

| 文件 | 内容 |
| --- | --- |
| `doc/implementation/router-rust-migration-w-activation-state-leaf.md` | 本文件（叶子任务） |
| `deployment/src/activation-state/mod.rs` | activation-state 模块入口：pure reducer/audit/error 导出 + DTO 再导出（DTO 本体仍留在冻结的 `storage/activation.rs`，不移动不改变字段） |
| `deployment/src/activation-state/reducer.rs` | 适配器无关 pure reducer：`prepare`/`abort`/`commit` 过渡函数、typed inputs、幂等 replay、CAS 语义、canonical replica 集合规范化 |
| `deployment/src/activation-state/audit.rs` | `ActivationAuditEvent`（frozen §6 形状：event_id/environment/activation_id/operation/expected+candidate generation/outcome/participants/timestamp）+ event_id 去重键派生 |
| `deployment/src/activation-state/error.rs` | 适配器无关 `ActivationStateError`（CasMismatch / InvalidRecord） |
| `deployment/src/lib.rs` | 仅加 `pub mod activation_state;`（additive，机械合并） |
| `deployment/tests/activation_reducer_contract.rs` | 复用 `activation-state-contract-cases.json` 驱动 pure reducer 的 corpus 测试 + reducer 专属序列（幂等、overflow、participant 规范化、错误分类）+ 与文件 adapter 同序列 conformance |
| `router/Cargo.toml` | 增加 `skiff-deployment`、`mongodb`、`async-trait`、`thiserror`（均已在 Cargo.lock） |
| `router/src/lib.rs` | 仅加 `pub mod activation;`（additive，机械合并） |
| `router/src/activation/mod.rs` | Router-owned activation state repository 模块入口 |
| `router/src/activation/error.rs` | `RepositoryError`（CasMismatch / InvalidRecord / Transient / Closed）+ driver 错误分类 |
| `router/src/activation/repository.rs` | `ActivationStateRepository` port（read/prepare/commit/abort/append_audit + initialize）+ `MongoActivationStateRepository`：transaction 内 read→reducer→CAS filter update→audit insert→commit；CAS/audit 失败回滚；读缺失→CasMismatch；幂等 replay 不写不审 |
| `router/src/activation/retry.rs` | 有界指数退避 retry（仅基础设施瞬态错误；CasMismatch/InvalidRecord 不重试；deadline/attempt/next backoff 计入 health） |
| `router/src/activation/health.rs` | repository health 快照（durable revision、last outcome、retry、audit、driver/shutdown） |
| `router/src/activation/index.rs` | Mongo index 管理（`activation_state.state.environment` unique；`activation_audit` 查询键 + 维护键） |
| `router/tests/activation_mongo_probe.rs` | P-activation-state 真实边界探针（`#[ignore]`，由 harness 注入 `SKIFF_ACTIVATION_MONGO_URL`/`SKIFF_ACTIVATION_MONGO_DB`）：CAS 冲突、retry 不重复 audit、audit 失败回滚、重启后 committed/pending 读取一致 |
| `scripts/lib/activation-state-live-harness.mjs` | 临时 mongod replica set harness（mktemp dbPath + 45000-45999 端口租约 + mongosh rs.initiate + 用后 SIGTERM/清理/端口回收断言），遵循 `encrypted-storage-live-*` 约定 |
| `scripts/run-router-activation-mongo-probe.mjs` | P-activation-state 探针 runner（自建 harness → `cargo test -p skiff-router --test activation_mongo_probe -- --ignored` → finally 清理） |

禁止写：`router/src/session`、`router/src/artifact`、`deployment/src/projection`、
`runtime/transport`、verify 注册表/selector graph、`verify.yml`、`AGENTS.md`、scripts README、
`skiff-instance.mjs`；不操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。
`deployment/src/storage/activation.rs` 保持冻结，本节点不改字段与文件 adapter 行为。

## 契约冻结要点（本节点实现映射）

- DTO：复用 `deployment::storage::activation` 冻结类型，不新增字段；schema version
  精确 `skiff-environment-activation-state-v2`；strict parse（deny unknown fields）+ `validate()`。
- Revision/CAS：revision = `(committed.generation, pending.activation_id | ∅)`；Mongo adapter
  在事务内 read 后由 pure reducer 判定 CAS 语义，update filter 用派生 tuple 构造
  （prepare：generation + pending 槽位；commit：generation + pending tuple 全等；
  abort：generation + pending.activation_id）；并发写冲突/匹配 0 → `CasMismatch`，不重试。
- 幂等性：完全相同的 mutation tuple 重放返回当前 state，不写 state、不追加 audit。
- Retry：只对 driver/连接/超时/写冲突等瞬态错误有界指数退避；`CasMismatch`/`InvalidRecord`
  不重试；retry 状态计入 health。
- Audit：每个成功 mutation 在**同一事务**内 append `activation_audit`；
  `event_id` = sha256(`(environment, activation_id, operation, expected, candidate)` framed)；
  audit 写失败 → 整个 mutation 回滚；retry 因幂等 replay + `_id` 去重不产生重复 audit；
  audit 不含 Mongo URL/secret/业务 payload。
- Index/Driver：collection `activation_state`/`activation_audit`；连接串来自 strict final
  Router config 的 `serviceDb.mongoUrl`（本节点只消费构造入参，不读 ambient env）；driver
  有连接超时、退避与 `close()`（shutdown 最后关闭 Mongo，关闭后操作返回 `Closed`）。
- Read-only port：`read` 只返回 committed+pending 全量 DTO；缺失 → `CasMismatch`
  （"state does not exist"），environment 不匹配 → `InvalidRecord`。reference existence
  check 属于文件 adapter/coordinator loader 边界（Mongo adapter 做 lexical validate；
  写入前 ref 存在性由 coordinator 的 blocking loader 保证）。
- 本节点不实现 coordinator、bootstrap reader、main.rs/listener.rs 装配；`append_audit`
  作为 port 方法提供（frozen §10），Mongo 实现按 `event_id` 幂等。

## 自验收矩阵

| 项 | 命令/断言 |
| --- | --- |
| reducer 单元/序列 | `cargo test -p skiff-deployment --test activation_reducer_contract`（corpus 6 case + reducer 专属序列 + conformance） |
| repository 单元/序列 | `cargo test -p skiff-router`（retry 策略、CAS filter、错误分类、health、audit 去重） |
| 现有 deployment/router 回归 | `cargo test -p skiff-deployment -p skiff-router`（冻结测试不回归） |
| 临时 Mongo probe | `node scripts/run-router-activation-mongo-probe.mjs`：CAS 冲突、retry 不重复 audit、audit 失败回滚、重连读取一致，全部通过且进程/目录/端口清理 |
| rustfmt/clippy | 触碰 Rust 文件 `cargo fmt --all --check`（新增文件）；`cargo clippy -p skiff-deployment -p skiff-router --tests`（触碰 crate 无新增 error） |
| 写集边界 | `git status` 仅含上表文件；`rg` 反向搜索证明未触碰禁止目录 |
| 退出 | 提交到 `feat/router-rust-w-activation-state`，交接 `/root/router_rust_integration_b4` 并通知 root |

## 停止条件

- 需要改公共契约（DTO schema、audit shape、CAS 语义、wire schema）：停止上报，不自行扩展。
- 与兄弟节点文件重叠：先通知 root。
- 设计空洞：返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 执行结果（提交前自验收填写）

（2026-08-02 提交前填写，全部通过）

1. `cargo test -p skiff-deployment -p skiff-router`：
   deployment 84 passed（lib，含新 activation_state 17 项）+ 4 passed
   （`activation_reducer_contract`，复用 batch 3 的 6-case corpus + 文件 adapter
   conformance + missing-assembly 边界）+ 既有 7 项；router 29 passed（lib，含
   retry/index/error/health/repository 单元）+ 2 passed（既有）+ 5 passed
   （`activation_repository_contract` port 级 sequence）+ 既有集成 20 项；
   `activation_mongo_probe` 为 1 ignored（live，harness 驱动）。
2. P-activation-state 临时 Mongo replica set probe：
   `node scripts/run-router-activation-mongo-probe.mjs` 通过。自建 mongod
   （45000-45999 租约端口 + mktemp dbPath + mongosh `rs.initiate`），断言：
   stale-generation CAS 拒绝；并发相同 prepare 收敛且仅 1 条 audit（retry 不重复）；
   不同 pending 槽位 CAS 拒绝；预置冲突 `event_id` 使 audit append 失败 → 整个
   mutation 回滚（bounded retry 后 Transient，state 无 pending、audit 无新增）；
   abort→重新 prepare→commit；新 driver 实例读取 committed/pending 一致；
   `close()` 后读失败、新实例仍可用；三个索引（state environment unique、
   audit query key unique、audit maintenance）就位。mongod/临时目录/端口租约
   全部清理，`assertPortsClosed` 通过，未触碰 stable Mongo（27017）。
3. rustfmt：全部新增 Rust 文件 `rustfmt --edition 2021 --check` 通过。
4. clippy：`cargo clippy -p skiff-deployment -p skiff-router --tests` 对本节点
   文件零 warning/error（剩余警告均为既有 crate baseline：artifact-model、
   runtime-request-contract、runtime-transport 等）。
5. 反向搜索：`MongoActivationStateRepository`/`activation_state`/
   `ActivationStateRepository` 不出现在 `runtime/transport/src`、
   `deployment/src/projection`、`router/src/session`、`router/src/artifact`；
   audit event id 与新 repository 无本节点外 consumer。
6. 写集：`git status` 仅含叶子文件所列条目（含 Cargo.lock 5 行 additive
   skiff-router 依赖，无新 crate）；`deployment/src/storage/activation.rs`
   未触碰。Cargo.lock 中 `mongodb 3.6.0` 为既有锁定版本。
7. 审计去重键说明（契约内实现决策）：audit dedup key 为
   `(environment, activation_id, operation, expected, candidate)`，因此同一
   activation_id 的 prepare→abort→重新 prepare 会命中同一 event_id 并拒绝重复
   audit（与 TS `auditIdentity` 语义一致）；coordinator 应使用每次 transaction
   唯一的 activation_id。
