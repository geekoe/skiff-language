# Router Rust Migration Batch 4 — contracts-actor Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_contracts_actor`
集成目标：`/root/router_rust_integration_b4`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-4.md`
  （contracts-actor 节点：`feat/router-rust-contracts-actor` / `wt-contracts-actor`，
  baseline `main@7683b7c8`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，
  2026-08-01），重点 §3.2（actor owner 职责表与 `ActorClaimToken`
  reserve/commit/abort 唯一 claim truth）、§3.3（`ActorMethodCatalogView` 只读
  immutable `RoutingEpoch` actor index）、§3.4（identity/fence 不可互换，
  `ActorIncarnationFence`）、§5.3（`C-model-actor → W-model-actor → M-actor`、
  `C-model-spawn → W-model-spawn → M-spawn → H-spawn-parent-cut`）、§5.4
  （contract pack 必填项；C-actor/C-spawn 条款；`callerKind =
  request | actorInvocation` 决策）、§5.5（`ActorFrameSink` /
  `SpawnSubmitRouter` 不拥有 pending）、§7（E-actor-rust / E-actor-parity）。
- A0 契约：`doc/implementation/router-rust-migration-a0-contract.md`（frozen；
  `ActorRoutingProjection` / `ActorRoutingMethod` / `ActorRoutingRef`，consumer
  合入冻结后只读投影，不得回退读取 PackageArtifact/File IR）。
- 同链已冻结契约（batch 3）：`router-rust-migration-c-model-registration-contract.md`、
  `router-rust-migration-c-session-contract.md`、`router-rust-migration-c-model-activation.md`、
  `router-rust-migration-c-model-artifact-contract.md`。

冲突时以权威设计为准；本叶子只冻结契约与 corpus，不写 production。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@7683b7c8` = `7683b7c8007a374ae07cb62c7723ced62929100b`
  （`git rev-parse main` 已验证；worktree HEAD 即该 commit）。
- 分支 / worktree：`feat/router-rust-contracts-actor` /
  `/Users/geek/workspace/wt-contracts-actor`（基线即上述 commit）。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-contracts-actor/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `7683b7c8…`，与批次文档一致。
2. A0 类型已合入：`skiff-deployment::projection::actor_routing`
   （`ActorRoutingProjection` / `ActorRoutingMethod` / `ActorRoutingRef`，
   camelCase + `deny_unknown_fields`，schemaVersion
   `skiff-actor-routing-projection-v1`）；identity newtype owner 为
   `skiff-artifact-model`（`ActorAbiIdentity` / `ActorImplementationIdentity` /
   `ActorMethodIdentity` / `ServiceDeploymentRef` / `PackageArtifactRef`）。
3. transport actor wire 现状（`runtime/transport/src`）：
   - `protocol/actor.rs`：`actor.getOrCreate.request/response/error`、
     `actor.replace.*`、`actor.find.*`、`actor.remove.*`（控制族，
     `ActorGetOrCreateRequestFrameHeader` 等，含 `ActorKeyFrameMetadata` /
     `ActorRefFrameMetadata` / `ActivationIdentityFrameMetadata`）；
   - `actor_method.rs`：`actor.method.invoke/return/error/cancel`
     （`ActorMethodFrame` codec，invoke 有 payload，其余 payload 必须为空；
     校验 identity 前缀、deadline、cancellationCorrelation、testCase 配对）；
   - `actor_owner.rs`：`actor.owner.invoke`、`actor.owner.control`、
     `actor.owner.control.ack`、`actor.owner.failure`（fence/routeAuthority/
     transition/bootstrap/deadline/evictionRequestId 校验；control operations:
     `MarkUpgrading | Discard | Activate | ActivateInitial | IdleEvict`）；
   - `protocol/spawn.rs`：`spawn.submit.request/response/error`，当前 request
     只有 `targetKind` + `callerRequestId`，无 `callerKind`（见下）。
4. 当前 TS Router 语义（`router/src/actor/`、`router/src/router/actor*`、
   `runtimeDispatcher.ts` / `productionActorMethodRouter.ts`）：
   - `ActorManager`/`InMemoryActorRegistryStore`：getOrCreate/replace/find/remove、
     acquireOwnerLease（lease + epoch fence）、markOwnerLive、renewOwnerLease、
     releaseOwnerLease、expireOwnerLeases、idleOwnerCandidates、
     requestIdleOwnerEviction / acknowledgeIdleOwnerEviction、admitActorMethod、
     accept/finish actor execution、activeExecutionsForRuntime、evictIdle；
   - `ActorGetCreateActivationCoordinator`：按 actor logical key 的
     get-or-create dedup（claims map），test capability lineage 冲突拒绝，
     pending ACK 按 requestId + connection + operation 关联，late ACK tombstone
     上限 1024；claim truth 目前在 registry store（epoch/fence/lease），
     尚无显式 `ActorClaimToken` 类型；
   - `ActorMethodDispatcher`：catalog.hasMethod → admitActorMethod → dispatchToOwner
     / activateInitial / markOwnerUpgrading / discardOldInstance / activateTarget；
     同一 invocationId 重复 settle 拒绝，owner/caller 精确 fence；
   - `ActorOwnerLeaseIdleController`：sweep = expireOwnerLeases +
     idleOwnerCandidates + requestIdleOwnerEviction + transport.sendIdleEviction；
     acknowledgeEviction 走 registry；renewOwner 走 lease TTL；
   - `ActorRuntimeDisconnectController`：owner runtime 断开时释放 owner lease、
     fail 关联 invocation（当前 pending 由 productionActorMethodRouter 收敛）；
   - spawn 现状（`runtimeDispatcher.requireSpawnParent`）：同一
     `callerRequestId` 字符串先在 request pending 查找、再查
     `activeActorInvocationParent`，两路都存在即“ambiguous”拒绝——这正是设计
     §5.3 要删除的“靠字符串前缀猜测/两路猜测”fallback；目标为显式
     `callerKind = request | actorInvocation` typed parent namespace。
5. 既有 contract corpus 约定（batch 3）：
   `runtime/transport/testdata/registration-handshake/`（frames.json +
   scenarios/*.json）+ `runtime/transport/tests/registration_handshake_corpus.rs`
   （byte-exact decode/re-encode + 参考状态机）；`session_directory_contract.rs`
   （参考模型测试）；`activation_transaction_corpus.rs`（事务语义 corpus）。
   本叶子沿用同一形态与 `include_str!` fixture 约定；不新增 Cargo 依赖，
   不修改 `Cargo.toml`/`Cargo.lock`。

## 任务目标（contract packs：actor + spawn）

1. **C-model-actor**：冻结 actor wire 模型（actor_method / actor_owner /
   actor control 现有 wire）、`ActorMethodCatalogView` 只读 A0 projection
   消费边界、`ActorOwnershipRegistry`（`ActorClaimToken` reserve/commit/abort，
   claim truth 唯一 owner）、`ActorActivationRequestBroker`（get-or-create
   dedup）、`ActorInvocationRelay`、`ActorOwnerControlBroker`、
   `ActorLeaseExpiryScheduler` 的 typed 边界与 byte-exact actor wire corpus。
2. **C-model-spawn**：冻结 spawn wire 模型：显式 closed enum
   `callerKind = request | actorInvocation`（不可碰撞 typed parent
   namespace），删除靠字符串前缀/两路猜测的 fallback；`FunctionSpawnParentResolver`
   / `ActorSpawnParentResolver` / stateless `SpawnSubmitRouter`（sink 不拥有
   pending）；`H-spawn-parent-cut` 前置（旧 shape 删除、无兼容 reader）。
3. **C-actor**：§5.4 handler 级契约：catalog view + ownership + activation
   request + invocation + control + lease ports；capacity、queue full、
   timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、
   real boundary probe。
4. **C-spawn**：§5.4 handler 级契约：两个 resolver + stateless
   `SpawnSubmitRouter`；collision、parent terminal、replacement 竞态；
   `H-spawn-parent-cut` 后解锁，本 pack 冻结 cut 前置与 corpus。

## 交付清单

- 契约文档四份（`doc/implementation/`）：
  - `router-rust-migration-c-model-actor-contract.md`
  - `router-rust-migration-c-model-spawn-contract.md`
  - `router-rust-migration-c-actor-contract.md`
  - `router-rust-migration-c-spawn-contract.md`
- 本叶子执行文件（本文件）。
- Corpus fixture + 测试（`runtime/transport/testdata/` + `runtime/transport/tests/`）：
  - `actor-wire/frames.json`（actor 族 frame 目录：完整二进制 hex + typed
    header + decodeAs + direction + payload presence）；
  - `actor-wire/scenarios/*.json`（ownership claim、activation dedup、
    invocation relay、owner control、lease expiry 语义序列）；
  - `actor-routing-projection.json`（A0 projection 正负例 fixture，
    File IR/source/payload 反例）；
  - `spawn-wire/frames.json`（spawn 族 frame 目录：新 `callerKind` shape +
    legacy old-shape 反例）；
  - `spawn-wire/scenarios/*.json`（parent resolution / collision / terminal /
    replacement 语义序列）；
  - `tests/actor_wire_corpus.rs`（byte-exact + 参考状态机）；
  - `tests/actor_owner_contract.rs`（ownership/activation/invocation/control/
    lease 参考模型）；
  - `tests/actor_catalog_view_contract.rs`（A0 projection fixture + catalog
    view 参考模型；测试内 mirror struct 严格对照 A0 schema）；
  - `tests/spawn_wire_corpus.rs`（spawn wire corpus + resolver/router 参考
    模型）。

## 写入边界

可写：

- `doc/implementation/router-rust-migration-contracts-actor-leaf.md` 与四份
  契约文档。
- `runtime/transport/testdata/actor-wire/`、`runtime/transport/testdata/
  spawn-wire/`、`runtime/transport/testdata/actor-routing-projection.json`
  （corpus fixtures）。
- `runtime/transport/tests/` 下四个测试文件（corpus/参考模型，test-only）。

禁止：

- `skiff-router` production、`runtime/transport/src` production、
  `deployment/` production（含 `src/projection/`，A0 已冻结不动）、
  artifact-model/artifact-identity production。
- `Cargo.toml` / `Cargo.lock`、AGENTS.md、scripts README、verify
  注册表/selector graph/verify.yml、`skiff-instance.mjs`。
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
  `pnpm verify`；不 push。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| corpus 测试通过（含负例） | `CARGO_TARGET_DIR=<worktree>/target cargo test --package skiff-runtime-transport --test actor_wire_corpus --test actor_owner_contract --test actor_catalog_view_contract --test spawn_wire_corpus` |
| 现有 transport 测试不回归 | `cargo test --package skiff-runtime-transport`（聚焦运行） |
| 契约文档覆盖 §5.4 必填项 | 四份文档均含 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、real boundary probe |
| `callerKind` 决策冻结 | C-model-spawn/C-spawn 文档 + spawn corpus 断言 `request | actorInvocation` closed enum、old-shape 无兼容 reader、无字符串前缀 fallback |
| 无 production consumer 提前依赖 | `rg -n "contracts-actor|c-model-actor|c-actor-contract|actor-wire|spawn-wire|ActorClaimToken|callerKind" --glob '!doc/**' --glob '!runtime/transport/tests/**' --glob '!runtime/transport/testdata/**'`（无命中） |
| baseline/写集干净 | `git status` 仅上述新增文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b4` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
