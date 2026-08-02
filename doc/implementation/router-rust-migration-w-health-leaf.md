# Router Rust Migration Batch 12 — W-health Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_health`
集成目标：`/root/router_rust_integration_b12`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-12.md`
  （health projection 节点；baseline `origin/main@ea8616bc`；
  批次文档由集成 Agent 在 `integration/router-rust-migration-batch-12`
  分支维护，本 worktree 锚定 origin/main）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（owner 表：`HealthAggregator` 只聚合 owner 发布的只读快照，
  反向不修改任何 owner）、§10（Concurrency/Sequence/Health：每个 owner 发布
  health snapshot、loop-risk required fields、missing/nonzero self-test、
  live fixture）、§8（external `runtime-live` 与两个 loop-risk selector
  消费同一 Rust health）。
- W-session leaf（`router-rust-migration-w-session-leaf.md`）：
  `RuntimeHealthLedger` 保留按 session 的 observation（map），供 health
  projection 消费；本叶子是该投影的唯一生产 consumer。
- 冻结消费契约（本叶子只消费，不重写）：
  `test-runner/src/runtime_execution/wire.rs` / `readiness.rs`、
  `scripts/lib/loop-risk-health.mjs`、`scripts/lib/isolated-test-runtime-instance.mjs`、
  `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`、
  `scripts/lib/skiff-source-test-suite.mjs`。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`（主 worktree 只读；主 worktree 在本地
  `main` 并行线上，与本批无关）。
- 精确 baseline：`origin/main@ea8616bc`（`git rev-parse ea8616bc` =
  `ea8616bcddc707f864170988c758c19d2930b09d`，已核对；本 worktree HEAD 即该
  commit）。
- 分支 / worktree：`feat/router-rust-health` /
  `/Users/geek/workspace/wt-health`（基线即上述 commit）。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-health/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论（锚定 ea8616bc）

1. baseline 锚定成功；`router/` 无 TS 源码（Batch 11 已 cutover-delete），
   `router/tests/` 无 `health_*` 前缀文件，`router/src/health/` 不存在。
2. `/__router/health` 当前是 listener.rs 的占位空 200；TS parity 参考为
   `git show baebf720:router/src/router/assemblyControlPlane.ts`
   （loopRisk detail 形状：`observedAt` / `router.dispatcher.pendingUnary|
   pendingStream` / `router.httpStream.backpressureWaiters|backpressureCancels`
   / `runtimes[].runtimeId|connected|fresh|counters`，counters 五字段
   `outboundRequestsPending|outboundStreamLeasesActive|
   streamRuntimeStreamsActive|flagBackedCancelWaitersActive|spawnedTasksActive`）。
3. 生产消费者现状：
   - test-runner `wire.rs::decode_health_snapshot_inner` 对根对象 exact
     object（ok/activeAssembly/pendingActivation/capabilityConnections/
     replicas，deny unknown）——§10 计数面进入默认投影必须先更新该解码器
     （批次文档明示允许“先更新 corpus/测试再实现”）。
   - `replicas`/`capabilityConnections` 解码器要求 `registeredAt` 字符串；
     Rust session 模块不保留注册时间戳，本投影省略该字段并同步放宽解码器。
   - JS 消费者（isolated runtime、package-service smoke oracle、
     skiff-source-test-suite）只读 ok/activeAssembly/pendingActivation/
     replicas/capabilityConnections 的 TS 字段语义，忽略未知键。
4. owner 现有 read-only snapshot 面（均可由 health 模块只读聚合）：
   - `ActiveRoutingEpochStore::health()` / `RoutingEpoch::ingress_projection()`
     （ingressCount）。
   - `RouterBootstrapAssembly::health()`（reader fail-closed + loader）。
   - `SessionLayer::health_snapshot()` / `directory_lock()` /
     `dispatch_capabilities_snapshot()` / `has_frame_writer()` /
     `health()`（`RuntimeHealthLedger`）。
   - `RequestDispatcher::health()` / `permit_ledger()`（per-session in-flight）。
   - `WebSocketLane`（pub `index`/`ledger`/`broker` 各自 `snapshot()`）。
   - `ActivationCoordinatorHandle::health()`、
     `ActivationStateRepository::health()`。
   - `ActorComponents` 六 owner 各自 `health()` + `SpawnSubmitRouter::health()`。
   - `PendingHttpRouter::pending_count()` / `overflow_terminal_count()`。
5. 预检发现的最小缺口与处理（均记录为本叶子 seam，不阻塞；owner 行为不变）：
   a. `RuntimeHealthLedger` 保留 per-session observation map 但只暴露计数；
      loopRisk `runtimes` 与 replica `lastHealthAt/healthCounters` 必须读到
      per-session observation。处理：给 ledger 增加一个只读
      `observations_snapshot()`（additive getter，不改 owner 语义；
      W-session leaf 本就声明该数据供 health projection 消费）。
   b. session 无注册时间戳：`registeredAt` 省略，test-runner 解码器放宽为
      optional（消费者契约更新，符合批次流程）。
   c. WS ledger 不发布 per-session pin 计数：replica
      `connectionPinCount/connectionReleaseAckCount` 输出 0（稳态真值；
      聚合计数在 `counters.generationLeases`）。
   d. Rust HTTP 网关无 backpressure waiter/active-writer 计数：
      loopRisk `httpStream.backpressureWaiters` 输出 0（字段齐全；
      `backpressureCancels` 为真实计数）。
   e. dispatcher health 不暴露 admission cursor：`counters.admission` 省略
      cursor（permits/counters 为真实值）。
   f. session consumer mailbox occupancy 不发布：`counters.mailboxes` 仅暴露
      coordinator mailbox（occupancy/capacity/saturation 为真实值）。
   g. session outbound writer queue occupancy 不发布：
      `counters.writerQueues` 暴露 WS observed write bytes 聚合与
      slow-client 计数（broker outbound pending 在 `counters.broker`）。
6. 无阻塞性设计空洞；可执行。supervisor 是唯一装配点，`/__router/health`
   路由需要把 `HealthAggregator` 传入 runtime/control listener：本叶子按
   批次“唯一 wiring owner”边界对 `supervisor/mod.rs` 做最小 additive wiring
   （构造 aggregator、注册 HTTP health source、透传 listener），并记录于
   写集。

## 任务目标

1. `router/src/health/`：`HealthAggregator`（只读聚合各 owner snapshot；
   不反向修改任何 owner）+ 生产 `/__router/health` JSON 投影：
   - 基础投影保持 TS 兼容 shape：
     `ok` / `activeAssembly{environment,generation,assemblyIdentity,
     configSnapshotId,ingressCount}` / `pendingActivation`（null 或冻结
     PendingActivation shape）/ `capabilityConnections` / `replicas`
     （TS 字段语义：replicaId、environment、generation、assemblyIdentity、
     configSnapshotId、state∈healthy|draining|disconnected、connected、
     inFlightCount、connectionPinCount、connectionReleaseAckCount、
     可选 lastHealthAt/healthCounters；本实现省略 registeredAt，见预检 5b）；
   - §10 计数面：新增顶层 `counters` 对象（19+1 个 section，见下方契约），
     聚合各 owner 发布的只读字段；
   - `?detail=loop-risk` 时追加 `loopRisk`（TS parity 形状，字段齐全）。
2. loop-risk evaluator / live harness 与 external `runtime-live` 消费同一
   Rust health：基础形状不动（除已记录的最小消费者更新），loopRisk 形状与
   TS `AssemblyControlPlane` 一致。
3. 各 owner counter 在 success/error/disconnect/saturation/shutdown 后归零
   的 health 输出层断言（稳态全零 fixture、真实 socket 注册→断开→归零、
   错误 terminal→归零；单调累计计数不“归零”，只断言 occupancy/pending 面）。
4. 更新受影响消费者测试：test-runner health 解码（counters optional +
   registeredAt optional + corpus 更新）、loop-risk self-test（全量 canonical
   shape + missing/nonzero 负例）、live 相关 fixture 若需。

## 写入边界

可写：

- `router/src/health/`（本节点独占）；
- `router/src/lib.rs`（additive：`pub mod health;` + re-export）；
- `router/src/session/health.rs`（仅 additive 只读 `observations_snapshot()`，
  预检 5a 的最小 seam）；
- `router/src/listener.rs`（`/__router/health` 路由 + 控制 listener 的
  aggregator 透传；本批唯一 wiring owner）；
- `router/src/supervisor/mod.rs`（仅 health wiring：构造 aggregator、
  HTTP health source、透传；见预检 6）；
- `router/tests/`（`health_*` 前缀：`health_common/`、`health_projection.rs`、
  `health_http.rs`）；
- `test-runner/src/runtime_execution/wire.rs` 与
  `test-runner/src/runtime_execution/tests/{support.rs,wire.rs}`
  （消费者契约更新，先于生产实现提交）；
- `scripts/check-loop-risk-health.mjs`、`scripts/tests/loop-risk-health.test.mjs`
  （self-test / 消费面）；
- 相关 fixtures/tests/doc、本叶子文件。

禁止：

- `runtime/` crate（含 `runtime/transport/src`）、`deployment/`、
  `router/src/session/` 除上述 additive getter 外的任何修改、
  AGENTS.md、scripts README、verify selector graph / verify-live-registry /
  verify-live-plan / verify-selector-graph、`scripts/skiff-instance.mjs`、
  CI workflow（release-ci 节点）；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 计数面契约（`counters` 顶层对象，camelCase）

section 集合（test-runner 解码器按此 exact set 校验）：

| section | 来源 owner | 字段 |
| --- | --- | --- |
| `activeRoutingEpoch` | `ActiveRoutingEpochStore` | `publishCount`, `active{environment,generation,assemblyIdentity,configSnapshotId}` |
| `bootstrap` | `RouterBootstrapAssembly` | `reader{missing,malformed,identityMismatch,pending,repository}` |
| `blockingLoader` | `BlockingLoader` | `concurrency,occupancy,queued,saturated,deadlineAborts,shutdownRefusals,shutdown` |
| `sessions` | `SessionLayer` | `preAuthConnections,preAuthRefused,registeredSessions,pendingSessions,cancelledSessions,barrierPending,consumerPermitsHeld,liveSessionTasks` |
| `capabilities` | session capability bindings | `connections` |
| `health` | `RuntimeHealthLedger` | `observations,observedTotal,healthBeforeAck` |
| `barrier` | session directory | `pending,permitsHeld,failStop` |
| `admission` | `RequestDispatcher` | `permitsHeld,releases,queueFullRejects,revalidateFailures,reselects,noCandidateRejects,duplicateRequestIdRejects` |
| `requestPending` | dispatcher + `PendingHttpRouter` | `unary,stream,derivedSpawn,httpPending,httpOverflowTerminals,stopped` |
| `terminal` | dispatcher | `bySource`（11 类 terminal source，恒全键） |
| `clientConnections` | `ClientConnectionIndex` | `connectionCount,openConnections,finalizerPending,finalizerCount,finalizerFailures,slowClientCount` |
| `generationLeases` | `RuntimeGenerationPinLedger` | `pinsAcquired,pinsPendingRelease,cachedAcquireCount,releaseAcks,releaseFailures,runtimeClosed` |
| `broker` | `WebSocketRequestBroker` | `generationCount,outboundPending,inboundPending,outboundTombstones,inboundTombstones,timerCount,protocolViolations,runtimeDisconnectDetached` |
| `actor` | 六个 actor owner | `catalog,ownership,activation,invocation,control,lease,spawn`（各自全部健康字段） |
| `activation` | coordinator + repository | coordinator 全字段（含 mailbox）+ `repository{environment,committedGeneration,pendingActivationId,lastOutcome,lastOutcomeOperation,retry,audit,driver}` |
| `http` | `HttpGatewayServer` | 13 个计数器 |
| `mailboxes` | coordinator（唯一发布者） | `coordinator{occupancy,capacity,saturation}` |
| `writerQueues` | WS index | `wsSlowClientCount,wsObservedWriteBytesTotal` |
| `spawnedTasks` | session + actor spawn | `liveSessionTasks,actorSpawnCapacityInUse,actorSpawnAccepted,actorSpawnRejected` |
| `shutdown` | 各 owner | `sessionFailStop,coordinatorShutdown,repositoryDriverClosed,repositoryDriverShutdownResidue,dispatcherStopped,wsFailStopReason` |

`counters` 不含 Mongo URL、secret、业务 payload、完整 query URL。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| 真实 `/__router/health` 响应 shape 兼容 | `cargo test -p skiff-router --test health_http`：TS 基础 shape、`counters` section 全集、`?detail=loop-risk` 字段齐全、405 语义 |
| 计数归零断言（health 输出层） | `health_http`：注册→health frame→replicas/observations 非零→disconnect→归零；错误 terminal→归零；`health_projection`：全零 golden JSON + 非零渲染对照 |
| loop-risk self-test + 隔离实例 live | `node scripts/check-loop-risk-health.mjs --self-test` 通过；`cargo test -p skiff-router --test health_http` 内嵌真实 socket loopRisk 投影 |
| 全量 router Rust 测试 | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-router` |
| 消费者测试 | `cargo test -p skiff-test-runner`（health 解码）+ `node scripts/tests/loop-risk-health.test.mjs`（或对应 test runner） |
| 聚焦 verify | `node scripts/verify.mjs --only router,router-rust-process-smoke` |
| 格式/clippy | `cargo fmt --all --check`；`cargo clippy -p skiff-router --all-targets`（新增文件零 warning/error） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b12` 报告 branch、worktree、
implementation commit/tree、写集、自验收矩阵与记录在案的 seam
（registeredAt 省略、pin per-session 0、backpressureWaiters 0、cursor 省略、
mailbox/writer-queue 部分面），并通知 root（父 Agent）。
