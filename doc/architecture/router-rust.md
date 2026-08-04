# Router（Rust）Architecture

状态：authoritative contract。Router Rust 迁移实施完成后，本文是唯一长期
Router 架构参考；`doc/implementation/router-rust-migration-plan.md` 已置
`complete`，不充当第二份架构规范。

## 本文负责 / 不负责

本文负责：

- `skiff-router` 作为唯一生产 Router 的长期 owner/contract 汇总；
- 进程拓扑、state owner、wire/artifact/durable model 归属、named gates 与
  验证入口、rollback 策略。

本文不负责：

- 用户可见语言语义、service API 和配置字段（归 `doc/reference/`）；
- 部署脚本 CLI 拼写、端口具体值（归 `scripts/README.md` 与部署配置）；
- Runtime 进程内部的 execution model（归 runtime 相关 architecture 文档）。

## 1. 进程拓扑

Router 是独立 Rust binary，与 Runtime 保持两个独立进程，不共享进程内
mutable state：

```text
client / ingress
  -> skiff-router public HTTP + client WebSocket
  -> skiff-router runtime/control listener
       <- runtime actively connects over /runtime

skiff-router
  -> shared artifact filesystem (read-only routing records)
  -> MongoDB (activation state + audit only)
```

- router 负责 service HTTP、control HTTP 和 runtime WebSocket；runtime 主动
  连接 router 并注册当前 loaded service。
- artifacts 是不可变 build record 和 release pointer；router 只读路由记录，
  不拥有 artifact 可变状态。
- MongoDB 只保存 durable activation state 与 audit；router 不启动 runtime、
  Mongo 或 telemetry。
- release-mode HTTP 必须带 `X-Skiff-Service` / `X-Skiff-Version` selector；
  缺 selector、release 不存在或 runtime 未注册一律 fail closed。
- canonical control 契约是 `/__skiff/activate-assembly`（发布新的 active
  assembly）与 `/__router/health`（health/loop-risk projection）；stale
  `/__skiff/reload-artifacts` 不作为 control reload。

## 2. State owners

Owner 按单一 invariant 命名并唯一拥有对应状态；禁止 `RouterCore` / 全局
mutable state / 万能 coordinator。任何 owner 合并必须先写出共同 invariant 和
sequence test。

| Owner | 唯一拥有 | 明确不拥有 |
| --- | --- | --- |
| `ActiveRoutingEpochStore` | 当前 immutable routing epoch 的原子 publication | pending activation、session eligibility cache、pin map |
| `RuntimeRegistrationDirectory` | live `RuntimeSessionEpoch`、registered assembly tuple、capability index、socket handle、replica/epoch 双索引 | active/draining 副本、capacity/pending、health history |
| `RuntimeHealthLedger` | current/retained health observation | routing eligibility、socket ownership |
| `RuntimeAdmissionPool` | per-session capacity permits、selection cursor/policy | session truth、request pending、active routing epoch |
| `RequestDispatcher` | ordinary unary/stream 与 derived task correlation、terminal、reservation token | actor-method invocation、peer WS correlation、socket |
| `ClientConnectionIndex` | logical client connection、business identity replacement、`ClientSocketGeneration` | Runtime generation pin、broker pending |
| `RuntimeGenerationPinLedger` | Runtime generation acquire/release pending/cache/session attachment | client business index、peer RPC correlation |
| `WebSocketRequestBroker` | peer request/response correlation、deadline、tombstone、captured socket generation | ordinary dispatcher pending、connection replacement policy |
| stateless `ActorMethodCatalogView` | 对显式 `Arc<RoutingEpoch>` 中 actor index 的 typed query | 独立 index、mailbox、refresh/publication、actor live state |
| `ActorOwnershipRegistry` | actor identity、incarnation、current owner fence、authoritative claim reservation/commit | activation request correlation、invocation correlation、timer |
| `ActorActivationRequestBroker` | get-or-create operation dedup、activation request/ACK correlation | actor key 上的 claim truth、invocation、lease scheduling |
| `ActorInvocationRelay` | actor method invocation/return/error/cancel correlation | owner registry mutation、owner-control ACK |
| `ActorOwnerControlBroker` | claim/renew/evict 等 owner-control correlation | method invocation、idle timing |
| `ActorLeaseExpiryScheduler` | lease/idle deadline scheduling 和 eviction trigger | actor registry truth、control correlation |
| `ActivationStateRepository` | durable DTO/revision/audit、Mongo indexes、read/CAS/retry | coordinator transaction、routing epoch |
| `ActivationCoordinator` | durable activation transaction lifecycle 和 live/recovery participant binding | active epoch storage、session mutation、socket write |
| pre-auth/per-session/per-client task | `RuntimeConnectionEpoch`、physical socket halves、bounded ingress/outbound queue、abort handle | logical routing/pending maps |
| `HealthAggregator` | owner-published read-only snapshots | 反向修改任一 owner |
| `RouterSupervisor` | config、construction、listener/task join、shutdown | 所有上述业务 mutable state |

关键不变式：

- active routing 只有 `ActiveRoutingEpochStore` 一个 authority：immutable
  `RoutingEpoch`（profile、assembly generation/identity、config snapshot
  id、ingress/deployment/actor projection）原子 `Arc` 发布；admission 捕获
  完整 epoch，不允许混合新旧 epoch。
- `RuntimeRegistrationDirectory` 使用 `current_by_replica` 与
  `sessions_by_epoch` 两张 exact index；replacement 先标记并 cancel old epoch
  再安装 new epoch，old close barrier 只能删除 old session 记录。
- 候选资格由 stateless `RuntimeCandidateQuery` 统一投影（epoch + exact
  registered tuple + capability + cancellation）；heartbeat freshness 不参与
  admission/activation。pending 持有 routing epoch、registered session lease
  和 permit，terminal 时一次释放。
- identity/fence 使用独立 newtype：`AssemblyGeneration`、
  `RuntimeConnectionEpoch`、`RuntimeSessionEpoch`、`ClientSocketGeneration`、
  `RuntimeGenerationLeaseId`、`ActorIncarnationFence`、`ActivationId` /
  `ActivationParticipantBinding`；禁止通用 fence 或裸字符串跨 domain 传递。
- runtime handshake 固定为：accept → router.bootstrap →
  runtime.capabilities → bind `RuntimeSessionEpoch` →
  assembly.activation:Register → runtime.registered ACK → runtime.health；
  `assembly.activation:Register` 的 state owner 仍是
  `RuntimeRegistrationDirectory`，不是 `ActivationCoordinator`。
- runtime disconnect 是 cancellation + barrier：installed component manifest
  静态声明 session-keyed state 和 terminal sink；全部 ACK 后删除 exact
  session，超时/槽位失效时 Router fail-stop。durable activation state 才在
  restart 后做 persistence reconciliation。
- client socket 生命周期有独立 finalizer：先撤销 business index，再触发
  client cancellation、broker detach、pin release、dispatcher terminal 和
  writer close；old finalizer 不得删除 replacement generation。
- concurrent first-owner 的 authoritative transition 只在
  `ActorOwnershipRegistry`：原子 reserve actor key 并签发
  `ActorClaimToken`，broker 持 token 执行 operation/dedup，commit/abort 必须
  带 token 回 registry；broker 不各存一份 claim truth。

## 3. Model ownership（wire / artifact / durable）

| 类别 | canonical owner | consumer |
| --- | --- | --- |
| Router↔Runtime wire model | `skiff-runtime-transport` 及必要的低层 request contract | Router、Runtime |
| compiler/Router/Runtime artifact model | `skiff-artifact-model`、`skiff-artifact-identity`、`skiff-deployment` strict reader | compiler/deployment/Router/Runtime |
| Router/platform durable activation model | canonical deployment/persistence crate 中的 DTO/pure reducer；Mongo adapter Router-owned | Router/deployment tooling；不是 Runtime wire contract |

边界契约：

- `skiff-router` 不得直接或传递依赖宽 `skiff-runtime-model`、runtime-host、
  eval 或 request execution；Runtime 不依赖 Router。
- Runtime 只消费 activation prepare/commit/abort wire projection，不消费
  Mongo durable record。
- actor catalog 只读 canonical routing projection，不读
  PackageArtifact/File IR。
- 第三类 durable model 即使物理位于 shared crate，也不得扩大 Runtime 或
  Router 的依赖面。

## 4. Named gates 与验证入口

verify registry（`scripts/lib/verify-rust-subjects.mjs` /
`verify-selector-graph.mjs` / `verify-plan.mjs`）：

- Rust subject `router` 是 `skiff-router` 的唯一 owner（leaf
  `router-contracts`，task `router:contracts`，自动生成唯一 Cargo test
  leaf）；manual `router` selector 只展开 Rust leaves：
  `router-contracts` + `router-rust-process-smoke`（task
  `router-rust:process-smoke`）。
- 每个 Rust workspace package 必须归入恰好一个 subject；新增 crate 漏 owner
  或同一 name 双 owner 都会让 registry integrity / transition test 失败。
- 已删除 TypeScript Router builder/leaf；`pnpm test`、默认 verify 与 CI 不再
  展开任何 TS Router 任务。

常用入口：

```bash
node scripts/verify.mjs --only router
node scripts/verify.mjs --only router-rust-process-smoke
cargo test -p skiff-router
```

live/manual gates（tier `live/manual`，默认 verify、`pnpm test`、Cargo
workspace 和 CI 都不展开）：

| Selector | 内容 |
| --- | --- |
| `router-rust-bootstrap-live` | compiler artifact、committed reader、initial epoch |
| `router-rust-session-live` | real Runtime bootstrap/register/reconnect/shutdown；无 unary |
| `router-rust-dispatch-live` | fake ingress → admission/pending → real Runtime |
| `router-rust-http-live` | real HTTP → Router → Runtime |
| `router-rust-ws-live` | real WS/broker/generation |
| `router-rust-actor-live` | two-replica actor chain |
| `router-activation-mongo-live` | reducer/CAS/retry/audit failure + temporary Mongo |
| `router-activation-full-chain-live` | real Router+Mongo+compiler artifact+Runtime commit/re-register/new request |
| `router-chat-full-chain-live` | pinned Skiff/internals/packages commits + Agine chat smoke |
| `router-clean-host-live` | Linux binary/PM2，无 pnpm/tsx/router node_modules |

其他验证契约：

- `loop-risk-health-live` / `loop-risk-stress-live` 必须通过
  `--loop-risk-config` 或 `SKIFF_LOOP_RISK_CONFIG` 传同一份 canonical JSON
  config，health URL 精确指向 `/__router/health?detail=loop-risk`。
- `runtime-live` 必须显式提供 runtime config、router reload URL 和 artifact
  root，不读取通用 env 也不猜 stable 4001。
- `db-encrypted-storage-live` 保留业务回归，但不用于证明 Router 无 Node
  依赖；该证明只归 `router-clean-host-live`。
- cross-repo chat full chain 在 `internals/agine` 运行
  `npm run e2e:chat-smoke`，并记录 Skiff/internals/skiff-packages commit 与
  service artifact identities。
- task 未实现且未出现在 `verify --list`、workflow 未实际调用前，不得标记
  gate 已建立。

## 5. Rollback 策略

- 同一 release 只启动一个 Router；不做双 Router production canary。
- 回滚对象是上一完整 repository/build release（immutable binary + config +
  artifacts），不是补丁式 hotfix。
- 演练顺序：stop admission → shutdown current Router → 确认 PID/listener
  退出 → 启动目标 immutable process → Runtime reconnect exact committed
  tuple → activation/readiness → 开放 admission → HTTP/WS/actor/chat smoke。
- 若必须改变 durable state / wire / artifact，先拆成独立 generation
  checkpoint，不在同一 release 内混用变更与回滚。
- build/instance/deploy/PM2 全部管理 Rust binary：`build-runtime-stack` 用
  Cargo-metadata source closure 生成 `build/runtime-stack/bin/skiff-router`；
  instance build/up 构建并安装 binary；deploy 上传 binary 并由 PM2 直接执行
  `--config`；clean host 只提供 binary/config/artifacts，PATH 不含
  pnpm/tsx。

## 6. 完成契约

- 唯一生产 Router 是 Rust `skiff-router` binary；TypeScript Router
  source/tests/package/lockfile/dist/CI/remote install 已全部删除。
- Router↔Runtime wire 和 artifact DTO/identity/reader 只有 canonical owner，
  无 TS/local mirror。
- active routing 只有 `ActiveRoutingEpochStore` 一个 authority；ordinary
  pending、WS broker pending、client connection、Runtime generation lease、
  actor ownership/claim/invocation/control/lease 和 activation transaction
  各有独立 owner。
- 所有 owner counters 在 success/error/disconnect/saturation/shutdown 后归零。
- 长期 owner/contract 以本文为准；`doc/implementation/router-rust-migration-plan.md`
  已置 `complete`，不再作为架构规范。
