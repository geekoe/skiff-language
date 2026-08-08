# Router Rust Migration Batch 4 — A1 Leaf Task

日期：2026-08-02
状态：execution leaf（开发 Agent：`/root/dev_a1`）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，2026-08-01），
  重点 §2.4（actor routing projection contract）、§3.2（stateless
  `ActorMethodCatalogView` / actor owners）、§3.3（immutable `RoutingEpoch` 内
  actor index）、§3.4（identity/fence 类型）、§5.4（C-actor pack 前置）、§7
  E-actor-rust / E-actor-parity。
- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`（DAG 节点 A1：
  A0 已建 `actor_routing`，A1 加 producer）。
- A0 冻结契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`（§2 schema、
  §3 owner、§4 identity generation、§5 exact deployment binding、§6 反例）。
- A0 叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a0-leaf.md`。
- 架构语义事实源：`doc/architecture/actor-model.md`。

本任务文件只补充执行信息，不改变设计语义与 A0 冻结 schema。

## 任务边界

实现 A1：compiler/deployment 侧 actor routing projection producer，消费 A0 冻结
schema（`ActorRoutingProjection` / `ActorRoutingMethod` / `ActorRoutingRef`），位置
`deployment/src/projection/actor_routing.rs`：

- 从 canonical artifact/ABI 信息（typed 生成身份）生成投影；
- 绝不读 File IR / source / executable payload；
- 不修改 A0 冻结的投影 schema、serde 表面与构造不变式；
- 不实现 A2 TS strict consumer / A3 Rust strict reader；
- 不定义 wire frame（C-model-actor）；不做 activation DTO（contracts-activation）；
- 不改 `artifact-model` / `artifact-identity` / compiler 生产代码；
- 不操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## Baseline / worktree

- Repo：`/Users/geek/workspace/skiff`；基线 `main@7683b7c8`（`git rev-parse` 已验证；
  worktree 创建时显式锚定该 commit）。
- 分支：`feat/router-rust-a1`；worktree：`/Users/geek/workspace/wt-a1`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-a1/target`（worktree 内独立 target）。
- 集成 Agent：`/root/router_rust_integration_b4`；不 merge、不 push、不碰集成分支。
- 并行兄弟：wt-contracts-actor / wt-contracts-ws / wt-w-model；`deployment/tests/`
  按前缀隔离，本任务只写 `a1_*`。

## 零 worktree 只读预检证据（main@7683b7c8）

1. compiler 现有 artifact 生成路径：
   - `compiler/driver/authoring/package_publication.rs` 的
     `publish_package_artifact_records`：写入 PackageArtifact 记录（
     `package_local_abi.public_symbols[*].actor` 携带 `PackageActorAbi {
     actor_abi_identity, abi }`）+ File IR 记录 + resource 记录；
   - `compiler/projection/src/package_artifact/actor.rs` 的 `project_actor_abi`：
     把 lowered `ActorDeclarationIr` 的 `actor_abi_identity` 与 `abi`（含
     public method `method_identity`）投影进 PackageArtifact；
   - identity 生成 canonical owner：`artifact-identity/src/actor.rs`
     （`actor_abi_identity` / `actor_method_identity` /
     `actor_implementation_identity`）；`ActorDeclarationIr` 在 lowering 时已生成
     三个 framed identity。
2. deployment 是否已有可从 PackageArtifact 生成投影的输入：
   - `project_service_deployment` 已接收 `&[PackageArtifact]`；
     PackageArtifact 的 actor ABI 事实（`actor_abi_identity` + public method
     identities）与 package ref 事实可读；
   - 但 PackageArtifact 不携带 `actor_implementation_identity`（该事实只存在于
     File IR `ActorDeclarationIr` 与 runtime linked-program），因此单独从
     PackageArtifact 无法生成 A0 冻结的 (abi, implementation, method) 三元组；
   - 结论：A1 在 deployment 侧定义 source-free typed producer input（只含 framed
     identity 字符串），由 compiler 侧调用方（后续集成）从 lowered actor
     declarations 提取事实；producer 自身绝不读 File IR / source / payload。
3. A0 反例边界（§6）：
   - 禁止 source（源码文本 / `sourceSpan` / `sourceAstHash`）；
   - 禁止 File IR 坐标（`modulePath`、`actorName`、`methodName` 作为字段、`unit`、
     `file`、`fileIrIdentity`、`loadedFileIndex`、`codeSlot`）；
   - 禁止 executable payload（`executableIndex`、可执行体、常量/类型表、payload
     bytes）；
   - 禁止 symbol path（`actorTypeIdentity`、`actorSymbol`）；
   - A0 投影结构无对应字段 + serde `deny_unknown_fields` 双重拒绝；A1 producer
     input 同样只携带 framed identity 字符串，不携带上述生成输入。
4. TS 现状语义对齐：`router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
   的 `loadActorMethods` 只遍历 `actor.methodImplementations`（public methods），
   create 不作为 method catalog 条目；A1 producer 每个 entry 对应一个 public
   method identity。
5. A0 交付现状：`deployment/src/projection/actor_routing.rs` 已冻结
   `ActorRoutingProjection` / `ActorRoutingMethod` / `ActorRoutingRef`、
   `ActorRoutingProjection::new` 构造校验（schema version / framed identity /
   serviceId 一致 / 排序 / duplicate）与 serde 表面（camelCase +
   `deny_unknown_fields`）。本任务只增量 producer 输入/函数/错误变体，不改冻结
   schema 语义。

## 冻结决策点（A1 授权范围内，不改 A0 schema）

- producer input schema version 固定为 `skiff-actor-routing-producer-input-v1`，
  构造/反序列化时校验，fail closed（独立于投影 schemaVersion）。
- `ActorRoutingProducerInput { schema_version, deployment:
  ServiceDeploymentRef, packages: Vec<ActorRoutingPackageInput> }`。
- `ActorRoutingPackageInput { package: PackageArtifactRef, actors:
  Vec<ActorRoutingActorInput> }`。
- `ActorRoutingActorInput { actor_abi_identity: ActorAbiIdentity,
  actor_implementation_identity: ActorImplementationIdentity, methods:
  Vec<ActorMethodIdentity> }` —— 只含 framed identity，无 module path / actor
  name / method name / executable 坐标。
- 每个 public method identity 展开为一个 `ActorRoutingMethod` entry（
  `actor.service_id = deployment.service_id`，deployment/package binding 原样
  复制），最终由冻结的 `ActorRoutingProjection::new` 统一排序、查重与校验。
- producer input 校验（fail closed）：schema version 精确匹配；actor 至少一个
  method；同一 actor 内 method identity 唯一；同一 package 内 actor 记录
  （abi + implementation）唯一；packages / actors 列表可为空（空 methods 投影
  合法）。
- 错误变体（additive）：`ProducerUnsupportedSchemaVersion` /
  `ProducerActorWithoutMethods` / `ProducerDuplicateActorMethod` /
  `ProducerDuplicateActor`；A0 错误语义不变。

## 交付物与写集

1. 本叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a1-leaf.md`。
2. producer：`deployment/src/projection/actor_routing.rs`（输入类型 +
   `project_actor_routing` + 输入校验 + 错误变体）。
3. 单测：`deployment/src/projection/actor_routing/tests/producer.rs`（既有
   `actor_routing/tests.rs` 增加 `mod producer;`）。
4. corpus：`deployment/tests/a1_actor_routing_producer_corpus.rs` +
   `deployment/tests/fixtures/a1-actor-routing-producer-corpus.json`。

禁止写：`deployment/src/activation-state`（W-activation-state）、`skiff-router`
（`router/`）、`runtime/transport`、verify 注册表、AGENTS.md、scripts README、
verify.yml、`skiff-instance.mjs`、`artifact-model` / `artifact-identity` / compiler
生产代码。

## 自验收（聚焦）

- `cargo fmt --manifest-path deployment/Cargo.toml -- --check`
- `cargo check --manifest-path deployment/Cargo.toml`
- `cargo test --manifest-path deployment/Cargo.toml actor_routing --no-fail-fast`
  （A0 冻结 10 项 + A1 producer 单测）
- `cargo test --manifest-path deployment/Cargo.toml --test a1_actor_routing_producer_corpus`
- rg 负例：producer 实现与新增单测中不出现
  `FileIrUnit` / `FileIrRef` / `modulePath` / `module_path` / `actorName` /
  `actor_name` / `methodName` / `method_name` / `sourceSpan` / `sourceAstHash` /
  `payload` / `executableIndex` / `codeSlot` / `loadedFileIndex` /
  `fileIrIdentity` / `actorSymbol` / `actorTypeIdentity`（A0 反例测试字符串除外）。

## 停止条件

- 若发现设计 §2.4 / A0 未覆盖且会改变架构或公共契约语义的决策（例如投影 schema
  变化、需要修改 artifact-model / artifact-identity / compiler 生产代码），停止并
  返回 `TASK_SCOPE_EXPANDED` 附证据。
- 兄弟 ownership 冲突（deployment crate 内 contracts-* / W-model 等）先通知 root。

## 交接

完成后提交到 `feat/router-rust-a1`（不 push），直接向
`/root/router_rust_integration_b4` 交接并通知 root。
