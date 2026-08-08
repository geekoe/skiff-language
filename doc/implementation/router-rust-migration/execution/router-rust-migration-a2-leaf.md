# Router Rust Migration Batch 8 — A2 Leaf Task（TS actor routing projection hard cut）

日期：2026-08-02
状态：execution leaf（开发 Agent：`/root/dev_a2`；一次性有界会话）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  §2.4（actor routing projection contract）、§3.2（stateless
  `ActorMethodCatalogView`）、§3.3（immutable `RoutingEpoch` actor index）、
  §7（E-actor-rust / E-actor-parity）。
- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-8.md`（A2 节点：
  `feat/router-rust-a2` / `wt-a2`，baseline `origin/main@d228b613`）。
- A0 冻结契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`
  （投影 schema、反例 §6、owner §3）。
- A1 交付：`deployment/src/projection/actor_routing.rs`（producer +
  `project_actor_routing`）+ `deployment/tests/a1_actor_routing_producer_corpus.rs`
  + `deployment/tests/fixtures/a1-actor-routing-producer-corpus.json`。
- A3 交付：`router/src/artifact/actor_routing.rs`（strict reader 校验链）+ 共享
  corpus `deployment/tests/fixtures/a3-actor-routing/corpus.json`。
- C-model-actor 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-actor-contract.md`
  §3（catalog view 查询 key 不含 declarationOwner / File IR 坐标；wire
  declarationOwner 是 wire 级声明归属事实，交叉校验归 W-actor invocation
  admission）。
- C-actor 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-actor-contract.md`
  §3.2（`CommitFenceFacts` 携带 `declaration_owner`；owner fence 从 wire 捕获）。

## 零 worktree 只读预检证据（origin/main@d228b613）

1. 基线锚定：`git rev-parse origin/main` = `d228b613eafeba5e2275bf830f5770f21b931e81`；
   本地 main（40fac3b6，未 push）与基线分叉，本任务一律以 origin/main 为基线。
2. canonical projection 记录路径已由 A1/A3 合流确认：
   - `router/src/bootstrap/assembly.rs`：
     `ACTOR_ROUTING_PROJECTION_RECORD_PATH = "records/actor-routing/current.json"`；
   - `scripts/check-router-bootstrap-live.mjs` 在 compiler artifact root 写
     `records/actor-routing/current.json`；
   - A3 共享 corpus `deployment/tests/fixtures/a3-actor-routing/corpus.json`
     按该目录物化记录。
   → 结论：canonical 位置 = `records/actor-routing/current.json`，与任务预期一致，
   不需要 TASK_SCOPE_EXPANDED。
3. TS 违规路径（本次要硬切的现状）：
   `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts::loadActorMethods`
   遍历 `RuntimeAssembly.packageLinkPlan.codeSlots[]` → PackageArtifact 记录 →
   `files[].fileIrIdentity` → FileIr 记录 → `actorDeclarations`，用
   `abi.actorName` 构造 `RuntimeAssemblyActorMethod.declarationOwner`
   （`{unit:{kind:'package',value:codeSlot}, file:{kind:'loadedFileIndex',value},
   actorSymbol}`）。违反 §2.4 canonical topology。
   消费者：`runtimeAssemblyActorMethodCatalog.ts`（hasMethod /
   declarationOwnerFor）、`actorMethodDispatcher.ts`、
   `productionActorMethodRouter.ts`（evictIdleOwner 用 catalog 的
   declarationOwnerFor 构造 idleEvict 帧）。
4. 现有 actor 测试 baseline：
   - `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`：
     fixture 走 `packageLinkPlan` + PackageArtifact（`accepts only PackageArtifact
     v10...` 等用例），无 projection 记录；
   - `router/tests/compilerGeneratedManifestCompatibility.test.ts`：
     current-scope fixture 断言 `loaded.actorMethods` 的
     `declarationOwner.actorSymbol === 'Counter'`（File IR 来源）；
   - `router/tests/runtime-assembly-actor-catalog.test.ts`：catalog 按
     declarationOwner JSON + (abi, impl, method) 匹配；
   - `router/tests/helpers/actorRoutingHarness.ts`：fake catalog 提供
     `declarationOwnerFor`；`actorBootstrap()` 不带 declarationOwner；
   - `router/tests/helpers/compilerArtifacts.ts`：compiler artifact root
     未写 projection 记录。
5. 设计空洞与收敛方向（本叶子决策点，不改设计语义）：
   - runtime 侧 `ActorInstanceFence` 包含 `declaration_owner`，idleEvict 的
     `control_instance_fence` 用完整 fence 相等匹配（
     `runtime/eval/src/actor_instance.rs::exact_session_handle` 比较
     `handle.fence == *fence`，含 declaration_owner）。因此 router 发起 idleEvict
     必须携带与激活时完全相同的 wire declarationOwner，不能由 projection 派生
     （projection 按 A0 §6 明确不含任何 File IR 坐标 / symbol path）。
   - C-actor §3.2 / Rust W-actor `types.rs` 的 `CommitFenceFacts` /
     `ActorOwnerFence` 已冻结该模型：declarationOwner 在 claim commit 时从 wire
     捕获进 owner fence，catalog view 不作为 admission key。
   - 因此 A2 在移除 catalog 的 File IR 来源时，把 TS owner fence 对齐到同一模型：
     `ActorBootstrapInput` / `ActorRegistryEntry` / `ActorOwnerFence` 增加
     `declarationOwner`，由 `actor.getOrCreate.request` / `actor.replace.request`
     wire header 捕获；`evictIdleOwner` 改用 `fence.declarationOwner`，删除
     catalog 的 `declarationOwnerFor` 选项。投影本身仍不含该字段。

## 任务边界

TS Router 硬切只读 canonical actor routing projection：

- `loadActorMethods` 改为只读 `records/actor-routing/current.json`，strict reader
  校验链与 A3 对齐：escape-proof 路径 → 有界读取（16 MiB）→ duplicate-key-free
  strict JSON → 精确 schemaVersion → typed 表面校验（deny unknown fields +
  identity 前缀 + serviceId 一致 + 排序 + 唯一）→ canonical JSON bytes 相等。
- actor method catalog 不再读 File IR / source / payload；admission key 与 Rust
  `CatalogQuery` 对齐：`{serviceId, actorAbiIdentity, actorImplementationIdentity,
  methodIdentity}`，不含 declarationOwner。
- 同步更新 TS 测试：legacy File IR 用例改负例；actor 全链 baseline
  （compiler-generated manifest）改为 projection 驱动。
- 为 E-actor-parity 提供 TS 侧 differential baseline：TS 消费 A3 共享 corpus
  （同一 record bytes），断言与 Rust strict reader / catalog 语义一致。

## 设计决策（本叶子授权范围内）

### D1：projection reader 独立 TS 模块

`router/src/router/actorRoutingProjection.ts`（新文件，A2 独占）：

- 常量：`ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION`、
  `ACTOR_ROUTING_PROJECTION_RECORD_PATH = "records/actor-routing/current.json"`、
  `MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES = 16 MiB`（与 A3 对齐）。
- `decodeActorRoutingProjectionRecord(bytes)`：完整 strict 校验链，失败抛
  typed Error（错误词汇与 A3 `ActorRoutingProjectionError` 对齐：schema /
  malformed / non-canonical / invalid / missing / too-large）。
- `canonicalJsonBytes(value)`：与 `skiff-canonical-json` 的
  `canonical_json_bytes` 输出一致（递归 key 排序、整数归一、serde_json 风格
  转义）；projection 值域无非整数浮点，遇到即 fail closed。
- `sortActorRoutingMethods`：按 A0 完整 typed key（actor.serviceId →
  actor.actorAbiIdentity → actorImplementationIdentity → methodIdentity →
  deployment → package）字典序。

### D2：`RuntimeAssemblyActorMethod` 改为 A0 entry 形态

```ts
interface RuntimeAssemblyActorMethod {
  actor: { serviceId: string; actorAbiIdentity: string };
  actorImplementationIdentity: string;
  methodIdentity: string;
  deployment: RuntimeAssemblyDeploymentRef;      // ServiceDeploymentRef 同形
  package: { packageId; packageVersion; packageBuildId; packageLocalAbiIdentity };
}
```

不再有 `declarationOwner`（unit/file/actorSymbol 全部来自 File IR，A0 §6 反例）。

### D3：catalog admission key 与 Rust `CatalogQuery` 对齐

`ActorMethodCatalog.hasMethod` 输入改为
`{serviceId, actorAbiIdentity, actorImplementationIdentity, methodIdentity}`；
`actorMethodDispatcher` 从 `header.actorRef.serviceId` 取值；
`RuntimeAssemblyActorMethodCatalog` 删除 `declarationOwnerFor`。

### D4：wire declarationOwner 由 owner fence 捕获（C-actor 对齐）

- `ActorBootstrapInput` / `ActorRegistryEntry` / `ActorOwnerFence`（以及派生
  `ActorIdleEvictionFence`）增加 `declarationOwner: ActorDeclarationOwnerFrameHeader`
  （wire 类型来自 `../protocol/actorMethodProtocol.js`；incoming 帧已在协议边界
  校验）。
- `actorGetCreateActivationCoordinator` 与 `actorSpawnRuntimeControl`（
  `actor.replace.request` / getOrCreate）从 wire header 捕获。
- `productionActorMethodRouter.evictIdleOwner` 用 `fence.declarationOwner`；
  `declarationOwnerFor` 选项删除。
- `cloneEntry` / `cloneOwnerFence` / `ownerFence()` 深拷贝该字段。
- `sameActorOwnerFence` 语义不变（owner 相关性仍按 actorKey+epoch+impl+owner
  lease 判定；同 entry 派生的 fence 恒共享同一 declarationOwner）。

### D5：测试 baseline

- loader fixture 默认写空 projection 记录；legacy PackageArtifact/File IR 用例改
  负例（无 projection 记录 → 失败；projection 存在时即使 package/file-ir 记录
  缺失/损坏也成功，证明生产路径不再读 File IR）。
- `compilerArtifacts.ts` 两个 helper 都写 projection 记录：websocket fixture 写
  空投影；current-scope fixture 从 compiler 产出的 PackageArtifact/File IR 提取
  framed identity（test-side A1 producer 角色，禁止进生产）合成单 entry 投影，
  断言 `loaded.actorMethods` 为 A0 形态。
- 新增 `actor-routing-projection-reader.test.ts`：消费 A3 共享 corpus
  （`deployment/tests/fixtures/a3-actor-routing/corpus.json`），valid 记录全部
  decode 成功，负例按 fail class（schema/malformed/non-canonical/invalid/
  missing）断言；并断言 catalog 在 single/multi entry 上的 hit/miss 与 Rust
  `ActorRoutingCatalog` 语义一致（E-actor-parity 的 TS 侧 baseline）。

## 写集（仅本 worktree）

生产（router TS，batch-8 并行 ownership：router TS 仅 A2）：

- `router/src/router/actorRoutingProjection.ts`（新）
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
- `router/src/router/runtimeAssemblySnapshot.ts`
- `router/src/router/runtimeAssemblyActorMethodCatalog.ts`
- `router/src/router/actorMethodDispatcher.ts`
- `router/src/router/productionActorMethodRouter.ts`
- `router/src/actor/registryStore.ts`
- `router/src/actor/inMemoryRegistryStore.ts`
- `router/src/router/actorGetCreateActivationCoordinator.ts`
- `router/src/router/actorSpawnRuntimeControl.ts`
- `router/src/router/actorRuntimeDisconnectController.ts`

测试 / fixtures：

- `router/tests/helpers/actorRoutingHarness.ts`
- `router/tests/helpers/compilerArtifacts.ts`
- `router/tests/runtime-assembly-actor-catalog.test.ts`
- `router/tests/actor-routing-projection-reader.test.ts`（新）
- `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`
- `router/tests/compilerGeneratedManifestCompatibility.test.ts`
- 其余 `router/tests/*.test.ts` 中受 `ActorBootstrapInput` 类型影响的 getOrCreate
  / replace 调用点（tsc 驱动，机械补 `declarationOwner`）
- 本叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a2-leaf.md`

禁止：`router/src/artifact/`（Rust）、`deployment/`、runtime crate、
`runtime/transport/src`、scripts verify/CI、AGENTS.md、`skiff-instance.mjs`、
A0/A1/A3 交付与契约文档；不操作 stable instance / Mongo / PM2 / 4004-4007；
不跑全量 `pnpm verify`。

## 自验收矩阵（提交前执行）

| 项 | 命令 / 断言 |
| --- | --- |
| router TS 测试全绿 | `pnpm --filter @skiff/router test` |
| type-check | `pnpm --filter @skiff/router type-check` |
| manifest 全链 baseline | `pnpm --filter @skiff/router test:manifest-compatibility`（含于 test，单独跑一遍） |
| projection corpus differential | `router/tests/actor-routing-projection-reader.test.ts` 通过 |
| File IR 负例 | `rg` 在 `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts` 与 `runtimeAssemblyActorMethodCatalog.ts` 中无 `fileIrIdentity` / `actorDeclarations` / `methodImplementations` / `actorName` / `codeSlots`（actor 读取路径） |
| 写集干净 | `git status` 仅本叶子写集；`git diff origin/main...HEAD` 聚焦 |

## 停止条件

- canonical projection 记录路径与 A1 交付不一致 → 停止返回 `TASK_SCOPE_EXPANDED`
  附证据，不写兼容 reader（预检已确认一致，未触发）。
- 发现 A0/设计未覆盖且改变公共契约语义的决策 → 停止返回
  `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`（declarationOwner wire 捕获按
  C-actor §3.2 现有冻结模型收敛，不属于新语义）。

## 交接

完成后提交到 `feat/router-rust-a2`（不 push），直接向
`/root/router_rust_integration_b8` 交接（commit SHA、worktree 路径、自验收
矩阵、A1 对齐点），并通知 root。

## 执行结果（提交前自验收）

- `pnpm --filter @skiff/router type-check`：通过（src + tests）。
- `pnpm --filter @skiff/router test`：69 files / 977 tests 全绿。
- `pnpm --filter @skiff/router test:manifest-compatibility`：3/3 通过（含
  current-scope 全链 actorMethods 为 A0 形态、无 projection 记录负例）。
- A3 共享 corpus differential：`actor-routing-projection-reader.test.ts` 覆盖
  全部非 missing 记录（23 项），TS strict reader 的 failure class 与 Rust
  `ActorRoutingProjectionStore::load` 一致（schema/malformed/non-canonical/
  invalid），single-entry 记录 exact A0 surface。
- rg 负例：`filesystemRuntimeAssemblySnapshotLoader.ts` /
  `runtimeAssemblyActorMethodCatalog.ts` / `actorRoutingProjection.ts` 中
  `fileIrIdentity` / `actorDeclarations` / `methodImplementations` /
  `actorName` / `codeSlots` / `PackageArtifact` 读取路径零命中（仅文档注释中的
  否定表述与 `declarationOwner` 的 wire 语义说明）。
- 写集：仅 router TS src/tests、测试 fixtures/helpers 与本叶子文档；未触碰
  router/src/artifact、deployment、runtime、scripts verify/CI、AGENTS.md。
