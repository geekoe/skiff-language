# Router Rust Migration Batch 7 — W-actor Leaf（Router Rust actor lane）

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_actor`
集成目标：`/root/router_rust_integration_b7`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-7.md`（W-actor 节点：
  `feat/router-rust-w-actor` / `wt-w-actor`，baseline `main@7d8779c4`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  §3.2（actor owner 职责表与 `ActorClaimToken` reserve/commit/abort 唯一
  claim truth）、§3.3（`ActorMethodCatalogView` 只读 immutable `RoutingEpoch`
  actor index）、§3.4（identity/fence 类型）、§5.4（C-actor/C-spawn 条款、
  `callerKind = request | actorInvocation` 决策）、§5.5（`SpawnSubmitRouter`
  sink 不拥有 pending）、§7（E-actor-rust：六 owner sequence、20 帧 corpus、
  10 parent 场景、invocation/control/lease/timer 归零）。
- 冻结契约：`router-rust-migration-c-actor-contract.md`、
  `router-rust-migration-c-model-actor-contract.md`（actor-wire corpus：
  `runtime/transport/testdata/actor-wire/`，20 帧 + 22 场景）、
  `router-rust-migration-c-spawn-contract.md`、
  `router-rust-migration-c-model-spawn-contract.md`（spawn-wire corpus：
  5 帧 + 10 parent 场景；`legacyCut` 无兼容 reader）。
- A0/A3 交付：`deployment/src/projection/actor_routing.rs` +
  `router/src/artifact/`（`ActorRoutingCatalog`、strict reader）。
- W-model-actor-spawn 交付：`runtime/transport/src/protocol/spawn.rs`
  （`SpawnSubmitRequestFrameHeaderV2` / `SpawnCallerKind` / `SpawnTargetKind` /
  canonical codec）、`runtime/transport/src/actor_method.rs`、
  `actor_owner.rs`、`protocol/actor.rs`。
- W-dispatch 交付：`router/src/dispatch/`（`DispatchSubmit`、
  `ActorMethodSpawnControl` / `ActorMethodSpawnDispatch` actor lane seam、
  `RequestDispatcher::spawn_submit`）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `7d8779c4b96c90c4d2d23748112ec1c0328091d7`
   （`git rev-parse main` 已验证；worktree HEAD 即该 commit）。
2. A3 catalog 接口：`ActorRoutingCatalog::from_projection(Arc<ActorRoutingProjection>)`
   一次构造 immutable index；`entries()` / `get(&ActorRoutingMethod)` /
   `methods_for_actor` / `actor_refs` 只读；`RoutingEpoch::actor_catalog()` 暴露
   捕获的 index；catalog view 用 typed full-key 查询，无独立 index/refresh。
3. transport actor codec：`actor.method.*`（`ActorMethodFrame`）、
   `actor.owner.*`（invoke/control/control.ack/failure）与 actor control
   typed DTO 均可用；router 直接依赖 `skiff-runtime-transport`。
4. spawn codec：`decode/encode_spawn_submit_request_frame` 消费
   `callerKind` closed enum；`legacy-no-caller-kind` 帧 decode 拒绝；
   `SpawnSubmitResponseFrameHeader` / `ActorSpawnRuntimeErrorFrameHeader`
   可用。
5. W-dispatch actor lane seam：`ActorMethodSpawnControl` 提供
   `is_active_invocation_parent(caller_request_id)` 与
   `submit_spawn(ActorMethodSpawnDispatch)`；`ActorMethodSpawnDispatch`
   `{ spawn_request_id, caller_request_id, target }` 是 dispatcher→actor
   lane 的转发 seam（dispatch 模块禁止修改，W-actor 只消费）。
6. 设计空洞检查：本叶子不实现 real socket/Mongo 全链（归 E-actor-rust）；
   六 owner 为 router-local synchronous reducer（沿用 dispatch 的
   `Arc<Mutex<Inner>>` 形态）；corpus 场景只断言 owner 语义，不依赖真实
   wire 帧逐条重放。

## 任务范围

在 `router/src/actor/`（新模块，仅 W-actor 写）实现：

1. `ActorMethodCatalogView`：只读显式 `Arc<RoutingEpoch>` 中 A3
   `ActorRoutingCatalog` 的 typed query（`has_method` / `method_for`），
   不读 PackageArtifact/File IR、不持有独立 index/mailbox/refresh。
2. `ActorOwnershipRegistry`：actor identity、incarnation、current owner
   fence、claim reserve/commit/abort 唯一 claim truth，签发
   `ActorClaimToken`；renew/release/expire 只由 registry 修改 owner truth。
3. `ActorActivationRequestBroker`：get-or-create dedup、activation
   request/ACK correlation（持 token 执行；commit/abort 回 registry；
   broker 不另存 claim truth）。
4. `ActorInvocationRelay`：method invocation/return/error/cancel 相关性，
   exact-fence settle、duplicate 拒绝、owner/caller disconnect、deadline
   terminal；不改 registry、不处理 owner-control ACK。
5. `ActorOwnerControlBroker`：claim/renew/evict 等 owner-control 相关性
   （requestId + runtimeId + operation + connection exact）、timeout、
   late-ACK tombstone。
6. `ActorLeaseExpiryScheduler`：lease/idle deadline 调度与 eviction
   trigger（bounded retry 3、exhausted fail-closed 上报）。
7. spawn consumer：stateless `SpawnSubmitRouter`，按 exact parent kind
   （`SpawnCallerKind::Request` | `ActorInvocation`）选择 resolver；
   sink 不拥有 pending；accepted spawn 与 parent 生命周期分离；
   `ActorMethodSpawnControl` seam 由 actor lane 提供
   （`is_active_invocation_parent` 查询 relay pending、
   `submit_spawn` 转交 invocation/actor-method spawn 执行 path）。

## 写入边界

可写（仅本 worktree）：

- `router/src/actor/`（新模块：catalog / ownership / activation_broker /
  invocation / control / lease / spawn / health / types / mod）。
- `router/src/lib.rs`（additive：`pub mod actor;` + re-export）。
- `router/tests/`（仅 `actor_*` 前缀新文件）。
- 本叶子任务文件与相关 doc。

禁止：

- `run_router` / `main.rs` / `listener.rs`；`router/src/activation/`、
  `ws/`、`routing/`、`dispatch/`、`session/`、`bootstrap/`、
  `artifact/`；runtime crate；`runtime/transport/src`；deployment；
  AGENTS.md；scripts README；verify 文件；`skiff-instance.mjs`；
  `Cargo.toml` / `Cargo.lock`。
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 实现要点

- 沿用 `RequestDispatcher` 的 owner 形态：synchronous reducer，
  `Arc<Mutex<Inner>>`，方法不跨 `.await` 持锁。
- 健康快照按 C-actor §7 字段：catalog 捕获/命中/未命中；ownership
  fences/reservations/commit/abort/conflict/expired/released；activation
  dedup/lineage-conflict/pending/tombstone/timeout；invocation
  pending/settled/rejected/terminal/tombstone；control pending/timeout/
  late-ack/wrong-correlation；lease sweep/expired/idle/eviction
  pending/acked/retried/exhausted。
- 常量：activation deadline 30s、control ACK deadline 10s、owner lease
  TTL 30s、idle TTL 30s、spawned actor method deadline 300s / lease 330s、
  eviction retry 上限 3、activation claims / control pending 上限 4096、
  late/settled tombstone 上限 1024、invocation relay 共享
  `runtime.maxConcurrency` 预算（构造注入）。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| 六 owner sequence 测试 | `cargo test -p skiff-router --test actor_catalog_view --test actor_ownership_registry --test actor_activation_broker --test actor_invocation_relay --test actor_owner_control --test actor_lease_scheduler` |
| C-actor corpus（20 帧）经真实 codec | `actor_wire_corpus` 测试（roundtrip byte-exact + 场景名冻结） |
| C-spawn corpus（10 parent 场景）经真实 router | `actor_spawn_router` 测试（真实 codec 帧 + resolver/router 语义，含 collision/parent terminal/replacement） |
| 归零断言 | success/error/disconnect/saturation/shutdown 后 invocation pending、control pending、activation claims、lease timers/evictions 全部归零 |
| router-rust subject 不回归 | `node scripts/verify.mjs --only router-rust`（不跑全量 verify） |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-actor/target`（不与其他
worktree 共享）。

## 交接

完成后向 `/root/router_rust_integration_b7` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
