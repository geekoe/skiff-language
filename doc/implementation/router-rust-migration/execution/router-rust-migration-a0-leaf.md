# Router Rust Migration Batch 3 — A0 Leaf Task

日期：2026-08-02
状态：execution leaf（开发 Agent：`/root/dev_a0`）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，2026-08-01），
  重点 §2.4（actor routing projection contract）、§3.2（owner 表：stateless
  `ActorMethodCatalogView` / `ActorOwnershipRegistry`）、§3.3（immutable `RoutingEpoch`
  内 actor index）、§3.4（identity/fence newtype）、§5.4（C-actor pack 前置）、§7
  E-actor-rust / E-actor-parity。
- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（PR 0b + A0 +
  contract packs；A0 条款见批次文档 DAG 表与验证 owner 表）。
- 本任务文件只补充执行信息，不改变设计语义。

## 任务边界

冻结 actor routing projection 的 schema / owner / identity generation：

- stable actor ref；
- method admission / implementation identity；
- exact deployment binding；
- 明确不含 source、File IR、executable payload。

非目标：不定义 wire frame（C-model packs）；不实现 A1 producer / A2 TS consumer /
A3 Rust reader；不改 router production 代码；不做 activation DTO（contracts-activation）；
不定义 bootstrap / artifact refs（contracts-bootstrap）。

## Baseline / worktree

- Repo：`/Users/geek/workspace/skiff`；基线 `main@1d442366`（`git rev-parse` 已验证；
  worktree 创建时显式锚定该 commit）。
- 分支：`feat/router-rust-a0`；worktree：`/Users/geek/workspace/wt-a0`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-a0/target`（worktree 内独立 target）。
- 集成 Agent：`/root/router_rust_integration_b3`；不 merge、不 push、不碰集成分支。
- 兄弟节点：wt-contracts-*（contracts-bootstrap/session/activation）、wt-pr0b 并行；
  A0 写入边界不与其重叠（见下）。

## 零 worktree 只读预检证据（main@1d442366）

1. 当前 TS 违规路径确认：
   `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts` 的 `loadActorMethods`
   遍历 `RuntimeAssembly.packageLinkPlan.codeSlots[]` → PackageArtifact 记录 →
   `files[].fileIrIdentity` → FileIr 记录 → `actorDeclarations`，构造
   `RuntimeAssemblyActorMethod`，其中 `declarationOwner` 携带
   `{ unit: {kind:'package', value: codeSlot}, file: {loadedFileIndex|fileIrIdentity},
   actorSymbol }` —— 即 File IR 坐标（code slot / file index / fileIrIdentity /
   source symbol），违反权威设计 §2.4 的 canonical topology 要求。
   消费者：`runtimeAssemblyActorMethodCatalog.ts`（hasMethod /
   declarationOwnerFor）、`actorMethodDispatcher.ts`、`productionActorMethodRouter.ts`、
   `actorGetCreateActivationCoordinator.ts`。
2. identity generation 已有 canonical owner，A0 不重造：
   - `artifact-identity/src/actor.rs`：`actor_abi_identity` / `actor_method_identity` /
     `actor_implementation_identity`（framed `skiff-actor-abi-v1:sha256:<hex>`、
     `skiff-actor-method-v1:sha256:<hex>`、`skiff-actor-implementation-v1:sha256:<hex>`）。
   - 类型 newtype：`artifact-model/src/actor_declaration.rs` 的 `ActorAbiIdentity` /
     `ActorImplementationIdentity` / `ActorMethodIdentity`。
   - 架构语义：`doc/architecture/actor-model.md` §Identity 与注册 / §任期与 Version：
     actor identity = service id + actor 类型 + key 字段类型 + key canonical 编码，
     service version / build id 不进 identity；ABI identity 覆盖 key 字段类型与
     canonical 编码、字段布局、公开成员方法签名和 actor runtime ABI；implementation
     identity 覆盖规范化可执行 IR 及其可达依赖。
3. deployment / artifact-model 无现有 actor routing/projection 类型可复用：
   deployment `projection/` 只投影 service deployment（operation bindings、service
   selectors、package closure），`assembly/` 只解析 RuntimeAssembly；均无 actor
   method catalog / actor ref / actor projection 类型。artifact-model 的
   `actor_declaration.rs` 是 compiler/artifact DTO（含 executable index / File IR
   上下文），不是 routing projection，且属于禁止写入的现有类型。
4. 结论：canonical projection 类型不存在，按任务指示放 deployment crate 新模块
   `projection/actor_routing`；无需新建 workspace crate（不动
   `verify-rust-subjects.mjs`）；不写 artifact-model / router / runtime / contracts-*。

## 冻结决策点（记录于契约文档，A0 授权范围内）

- stable actor ref 形态：`{ service_id, actor_abi_identity }`（actor-model.md 规定
  service id 是 actor identity 的一部分；ABI identity 已 canonical 覆盖 actor
  类型、key 字段类型与 canonical 编码；不引入新的 ActorTypeIdentity hash，避免
  与 ABI identity 重复持有同一事实）。
- method admission / implementation identity：entry 携带
  `actor_abi_identity` + `actor_implementation_identity` + `method_identity`。
- exact deployment binding：`ServiceDeploymentRef` + owning `PackageArtifactRef`
  （同一 (abi, implementation, method) 可能跨包重复，module path 是包内命名空间，
  因此必须用 package build id 精确定位声明 owner）。
- 反例（不得进入投影）：source、File IR、source symbol path、file index /
  fileIrIdentity / code slot、executable index / payload、sourceSpan、module path
  本身（module path 只作为 identity derivation 的输入，不进入投影字段）。
- wire frame 形态不在 A0 范围：由 C-model-actor pack 决定 projection 字段到 wire
  frame 的映射。

## 交付物与写集

1. 契约文档：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`。
2. canonical 类型：`deployment/src/projection/actor_routing.rs`（+ 子模块
   `actor_routing/tests.rs`）；`deployment/src/projection/mod.rs` 增加
   `pub mod actor_routing;`。
3. 测试：投影构造（schema version、identity 前缀、duplicate、排序确定性）、
   serde roundtrip（camelCase / deny_unknown_fields）、反例（File IR 字段被
   deny_unknown_fields 拒绝；类型结构无 source/File IR/payload 字段）。
4. 反向搜索证据：`rg` 证明新类型只被自身模块/测试与契约文档引用，无任何
   A1/A2/A3 或 router TS/Rust consumer 提前消费。

禁止写：router/、runtime/、artifact-model/、contracts-* 模块、AGENTS.md、
scripts README、verify selector graph、verify.yml、`skiff-instance.mjs`；
不操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收（聚焦）

- `cargo check --manifest-path deployment/Cargo.toml`
- `cargo test --manifest-path deployment/Cargo.toml actor_routing --no-fail-fast`
  （或 `cargo test -p skiff-deployment actor_routing --no-fail-fast`）
- `cargo fmt --manifest-path deployment/Cargo.toml -- --check`（仅新文件影响范围）
- 契约文档覆盖 §2.4 全部约束 + 反例；rg 反向搜索无提前 consumer。

## 停止条件

- 若发现设计 §2.4 未覆盖且会改变架构 / 公共契约语义的决策（超出本文件冻结决策
  范围），停止并返回 `TASK_SCOPE_EXPANDED` 附证据。
- 兄弟 ownership 冲突（deployment crate 内 contracts-activation 等）先通知 root。
