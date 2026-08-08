# Router Rust Migration Batch 10 — A1-compiler Leaf Task

日期：2026-08-03
状态：execution leaf（开发 Agent：`/root/dev_a1_compiler`；一次性有界会话）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §2.4（actor routing projection contract / A1 compiler producer）、§7
  E-actor-parity（A2 已硬切 canonical projection）。
- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-10.md`（A1-compiler
  节点条款、验证 owner、写边界）。
- A0 冻结契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`
  （schema / owner / identity generation / 反例 §6）。
- A1 叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a1-leaf.md`（producer
  输入形态、compiler 侧调用方缺口、冻结决策点）。
- A2 叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a2-leaf.md`
  （`records/actor-routing/current.json` 是 TS strict reader 的唯一路径；
  loadActorMethods 消费 A0 形态 entry，create 不入 catalog）。
- A3 叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a3-leaf.md`（strict
  reader 校验链；`ActorRoutingProjectionRef { record_path }` 是临时 seam，
  canonical 记录路径推导归 A1 producer 输出面）。
- E-activation 叶子（历史）：pending 用例改为 recovery 语义后，
  `bootstrap-chain-corpus.json` 标签更新归 deployment owner（本节点执行）。

## 零 worktree 只读预检证据（origin/main@edc111f8）

1. 基线锚定：`git rev-parse origin/main` =
   `edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`；共享主 worktree 在本地
   `main`（adbcd1b4，用户并行线），本节点一律只读 git 对象，全部实际工作在
   worktree `/Users/geek/workspace/wt-a1-compiler` 内进行。
2. A1 producer 已合入基线：`deployment/src/projection/actor_routing.rs`
   含 `ActorRoutingProducerInput` /
   `ActorRoutingPackageInput` / `ActorRoutingActorInput` /
   `project_actor_routing`（schema
   `skiff-actor-routing-producer-input-v1`；只接受 framed identity；
   每 public method 展开一个 entry；冻结构造不变式由
   `ActorRoutingProjection::new` 统一排序/查重/校验）。
3. compiler publish 路径（compiler crate 生产代码，基线现状）：
   - package：`compiler/driver/authoring.rs::build_authoring_object` →
     `build_package_after_platform_context_guard` →
     `publish_package_artifact_records_to_store` 写 PackageArtifact / File IR /
     resource / schema 记录；service 包随后写 ServiceContract /
     ServiceDeployment 与指针；
   - assembly：`project_runtime_assembly` →
     `project_runtime_assembly_to_store` 读 deployments/contracts/packages →
     `resolve_runtime_assembly` → `store.write_runtime_assembly`；
   - 两条路径均不写 `records/actor-routing/current.json`。
4. A1 leaf 记录的调用方缺口确认：`PackageArtifact` 只携带
   `actor_abi_identity` + public method identities，不携带
   `actor_implementation_identity`；后者只在 File IR
   `FileIrUnit.actor_declarations[*].actor_implementation_identity`
   （`ActorDeclarationIr`）中存在。compiler 侧调用方必须从 lowered
   declarations 提取事实；producer 自身保持 source-free（A0 §6 反例）。
5. A2/A3 期望的读取路径一致：`router/src/bootstrap/assembly.rs` 与
   `router/src/router/actorRoutingProjection.ts` 均固定
   `records/actor-routing/current.json`；scripts harness 同样手工写该路径
   （本节点不修改 scripts）。A3 reader 要求 canonical JSON bytes 完全相等
   （`skiff_canonical_json::canonical_json_bytes`）。
6. storage 现状：`CanonicalArtifactStore` 有 `read_file_ir`，但没有投影记录
   写入 API；`write_immutable` 是 identity-addressed 语义，不能覆盖
   "current" 记录；`with_exclusive_pointer_lock` / `replace_locked` 提供
   原子替换原语（pointer 模式）。
7. corpus 现状：`deployment/tests/fixtures/bootstrap-chain-corpus.json`
   `pending-present` 的 `bootstrapOutcome` 仍为 legacy `failClosedPending`；
   E-activation（基线）已把 reader 改为 `CommittedWithPending`（committed
   先发布 + recovery 安装），router 消费测试
   `router/tests/bootstrap_reader.rs` 用特殊分支断言 `committedWithPending`
   并注释"fixture 标签由 deployment owner 更新"。
8. 无并行冲突：`feat/router-rust-ws-only-routing`、
   `feat/router-rust-rollback-final` 均停在 origin/main；
   `integration/router-rust-migration-batch-10` 只追加批次文档。

## 设计决策（本叶子授权范围内；不改变 A0/A1 冻结语义）

### D1：A1 producer 输出面 = 记录路径常量 + store 写入器

在 `skiff-deployment::projection::actor_routing` 增加
`ACTOR_ROUTING_PROJECTION_RECORD_PATH = "records/actor-routing/current.json"`
（A3 leaf D2 的 canonical 推导收口；router/src 与 TS 的既有同名常量因写边界
禁止触碰而保持字符串一致，本叶子在交接点记录该 seam）。
`CanonicalArtifactStore::write_actor_routing_projection` 以 canonical JSON
bytes + 原子替换（exclusive pointer lock + rename）写入该相对路径，与
A3/A2 strict reader 的 canonical bytes 校验链同源。

### D2：compiler 侧调用方只提取 framed identity

新增 `compiler/driver/authoring/actor_routing.rs`：

- 对每个 deployment，取其 `implementation` + `package_bindings[*].package`
  对应的 PackageArtifact（按 `package_build_id` 精确定位，缺失/身份不符 fail
  closed），逐 File IR 记录读取 `FileIrUnit.actor_declarations`；
- 每个 actor 提取 `actor_abi_identity` / `actor_implementation_identity` /
  `method_implementations.keys()`（public method identities）；create-only
  actor（无 public method）直接跳过，不进入 producer input（A1 producer
  要求 actor 至少一个 method，create 不是 catalog entry，A2 语义一致）；
- 组装 `ActorRoutingProducerInput` 调 `project_actor_routing`；producer 输入
  仍然只含 framed identity 字符串，不携带 modulePath / actorName /
  methodName / executable 坐标 / payload。

### D3：多 deployment assembly 按冻结构造器合并

`ActorRoutingProducerInput` 是单 deployment 形态（A1 冻结）。assembly 有
多个 root deployment 时，对每个 deployment 各自投影，再把全部 method
entries 交给冻结的 `ActorRoutingProjection::new` 统一排序/查重/校验（保持
A0 immutable epoch 一次构造语义，不新增合并 schema）。空 assembly 合法
（methods 为空）。

### D4：写入选点

- package publish（`build_package_after_platform_context_guard`）：service
  包在 ServiceDeployment 生成/写入后写该 deployment 的投影（实现包 +
  closure 包）；非 service 包写空投影；
- assembly publish（`project_runtime_assembly_to_store`）：RuntimeAssembly
  写入后按全部 root deployments 投影合并写入。

### D5：corpus pendingPresent 标签

`failClosedPending` → `recoverPending`（recovery 语义；对应
`ActivationRecoveryAction` 词族与 E-activation 的
`CommittedWithPending` 消费语义）。同步更新
`deployment/tests/bootstrap_chain_corpus.rs` 与
`router/tests/bootstrap_reader.rs` 的断言，使 corpus 标签成为消费测试的
单一事实源。

## 交付物与写集

生产（仅本节点 ownership）：

- `deployment/src/projection/actor_routing.rs`（只加记录路径常量）
- `deployment/src/storage/records.rs`（`write_actor_routing_projection`）
- `compiler/driver/authoring/actor_routing.rs`（新，compiler 侧调用方）
- `compiler/driver/authoring.rs`（接 package/assembly publish 路径）

测试 / fixtures：

- `deployment/tests/fixtures/bootstrap-chain-corpus.json`
- `deployment/tests/bootstrap_chain_corpus.rs`
- `deployment/tests/a1_actor_routing_producer_corpus.rs`（记录写入器测试）
- `router/tests/bootstrap_reader.rs`（corpus 标签消费断言）
- `compiler/tests/actor_routing_projection_publish.rs`（新，真实 compiler
  产物测试）
- `compiler/Cargo.toml`（注册新 integration test）
- 本叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a1-compiler-leaf.md`

禁止写：`router/src/`、runtime crate、`runtime/transport/src`、router TS、
AGENTS.md、scripts README、verify 注册表/文件、`skiff-instance.mjs`；
不操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 断言 |
| --- | --- |
| deployment 聚焦 | `cargo test -p skiff-deployment --test a1_actor_routing_producer_corpus --test bootstrap_chain_corpus --no-fail-fast` |
| compiler 新测试 | `cargo test -p skiff-compiler --test actor_routing_projection_publish --no-fail-fast`（真实产物自动含记录；A3 等价 reader 可消费） |
| compiler publish 回归 | `cargo test -p skiff-compiler --lib authoring --no-fail-fast`（authoring 单测）+ `cargo test -p skiff-compiler --test generated_service_deployment --no-fail-fast` |
| router corpus 消费 | `cargo test -p skiff-router --test bootstrap_reader --no-fail-fast` |
| 相关包整体 | `cargo test -p skiff-deployment -p skiff-compiler -p skiff-router --no-fail-fast` |
| 格式 | `cargo fmt --all -- --check`（聚焦触碰文件） |
| rg 负例 | `deployment/src/projection/actor_routing.rs` producer 输入/投影不含 File IR / source / payload 字段（A0 反例测试字符串除外） |

## 停止条件

- 发现 A0/A1/权威设计未覆盖且改变公共契约语义的决策（例如投影 schema 变化、
  需要改 artifact-model / artifact-identity、需要改 router/src 才能收口记录
  路径）→ 返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE` 附证据。
- 兄弟 ownership 冲突（router/src、runtime crate、scripts verify 等）先通知
  root，不静默改写。

## 交接

完成后提交到 `feat/router-rust-a1-compiler`（不 push），直接向
`/root/router_rust_integration_b10` 交接（commit SHA、worktree 路径、自验收
矩阵、A3 ref seam 对齐点），并通知 root。

## 执行结果（提交前自验收）

- 实现 commit：`412b228b`（branch `feat/router-rust-a1-compiler`，worktree
  `/Users/geek/workspace/wt-a1-compiler`，基于 origin/main@edc111f8，未 push）。
- `cargo test -p skiff-deployment --no-fail-fast`：全绿（95 lib +
  a1 producer corpus 5 + a3 corpus 5 + activation/boot corpus 等）。
- `cargo test -p skiff-compiler --no-fail-fast`：全绿（lib 41、bin 7、
  新 `actor_routing_projection_publish` 2、`generated_service_deployment` 12
  及其余全部 integration suites）。
- `cargo test -p skiff-router --no-fail-fast`：全绿（含
  `bootstrap_reader` 5，corpus 标签改为消费 `recoverPending`）。
- 新 compiler 集成测试证据：真实 std + 两个 service 包（含 actor）经
  `build_authoring_object` 发布后 `records/actor-routing/current.json` 自动
  生成：package publish 记录与 store 内 File IR 提取的 public method 集合
  完全一致（create-only actor 被排除、deployment/package binding 精确）；
  后续 package publish 原子替换 current 记录；assembly publish 合并两个
  deployment 的 entries；空 assembly 产出合法空投影；每条记录均通过
  A3 等价 reader 链（canonical bytes 相等 + typed `deny_unknown_fields`
  解码）。
- deployment `write_actor_routing_projection` 测试：canonical 路径
  `records/actor-routing/current.json`、canonical bytes roundtrip、
  再次发布原子替换。
- rustfmt：全部触碰 Rust 文件 `--check` 通过；clippy 三 crate
  `--all-targets` exit 0（仅存量 advisory，新代码零诊断）。
- rg 负例：`deployment/src/projection/actor_routing.rs` 与
  `deployment/tests/a1_actor_routing_producer_corpus.rs` 中 File IR /
  source / payload 标识符仅存在于否定表述注释。

### A3 ref seam 对齐点（交接信息）

- `ActorRoutingProjectionRef { record_path }` 的 canonical 推导现由 A1
  producer 输出面收口：`skiff_deployment::projection::actor_routing::
  ACTOR_ROUTING_PROJECTION_RECORD_PATH = "records/actor-routing/current.json"`，
  compiler publish 自动写入；router/src 与 TS 的既有同名常量因本节点写边界
  禁止触碰而保持字符串一致（机械合并时无需改，后续若允许可引用
  deployment 常量去重）。
- scripts harness 仍手工写空投影覆盖 compiler 产物（本节点禁止写 scripts）；
  E-actor-parity / differential 节点如需真实 actor entries 应改由 compiler
  产物提供（该改动归 scripts 侧 owner）。
