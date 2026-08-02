# Skiff Router Rust Migration Plan

日期：2026-08-01

状态：complete（实施完成，长期架构见 `doc/architecture/router-rust.md`）

本文是迁移实施计划，不是长期架构规范。实施已完成，长期架构见
`doc/architecture/router-rust.md`；本文不充当第二份架构规范。原目标是把
`skiff/router` 从 TypeScript 迁移为独立 Rust binary，让 Router 与 Runtime
直接复用 canonical Rust artifact/wire model，同时保持两个独立进程和现有可观察语义。

本计划的组织原则：

1. 先确定跨进程 shared model 的 owner、envelope 和 identity，再写业务 handler。
2. 不建立 `RouterCore` 或换名后的 domain 万能容器；state owner 按单一 invariant 划分。
3. 不等待“所有接口冻结”才并行。接口按 lane 形成 contract pack，某 lane 冻结后立即解锁对应实现。
4. 第一个实施 PR 先建立 TS/Rust process-selection seam、空 Rust package 和可选择的测试入口；此后每条 lane
   持续进入同一 binary 和 named verification task。
5. capability gate 是依赖 DAG，不是 Phase 1→6 的串行开发流程；生产仍在全部 gate 通过后一次 hard cut。

Skiff 尚未发布，不需要为退役的 manifest、config、endpoint、wire 或 artifact generation 保留兼容 reader、
alias、fallback 或双写。迁移期间 TS/Rust 可以在完全隔离的 namespace 中作为测试目标并存，但同一 environment、
Mongo activation namespace 和 Runtime connection set 始终只能有一个 Router owner。

## 1. 决策与边界

迁移应执行，主要收益为：

- Router/Runtime binary frame DTO、direction validation 和 codec 收敛到 canonical Rust owner。
- Router 直接消费 canonical artifact model、identity 和 strict reader，删除 TS mirror/hash/raw JSON reader。
- 用明确 state owner、typed epoch/lease、bounded mailbox 和唯一 terminal path替代 Node event loop 隐式串行性。
- shared model 与 domain ports 解耦，让 Runtime session、HTTP、activation、WS、actor、tooling 并行开发。
- build、instance、verify、deploy 从空 skeleton 开始持续集成，避免切换期首次发现生命周期问题。

Non-goals：

- 不合并 Router/Runtime，不共享进程内 mutable state。
- 不引入 Node↔Rust proxy、FFI、sidecar、CLI-per-request 或临时业务协议。
- 不新增 autoscaling、retry、service-to-service remote boundary 或负载算法。
- 不迁 telemetry，不让 Router decode 业务 schema/type/payload。
- 不借迁移 redesign HTTP/WebSocket/actor external semantics。
- 不建立全局 mutable state、无类型 event bus、万能 coordinator/manager/registry。
- 不做两个 Router 共享 Mongo/Runtime 的 production canary 或长期双实现。

性能提升不是正确性前提。吞吐、延迟、CPU、RSS 需要基线和回归 gate，但不能驱动语义变化。

## 2. 当前事实与迁移前置项

### 2.1 当前生产 owner 不是一个 Core

`router/src/router/server.ts` 当前分别装配 active snapshot、assembly/runtime registries、runtime endpoint、
activation coordinator、request dispatcher、WebSocket broker/generation lifecycle、actor controllers 和 gateways。
生产 import graph 约 73 个 TS 文件、32,287 行；Router tests 约 57 个文件、777 项。

canonical gateway architecture 已明确：

- ordinary request pending 属于 `RuntimeDispatcher`；
- WebSocket peer-RPC correlation 属于 `WebSocketRequestBroker`；
- 两者不能共用无类型 pending map；
- Runtime registry 不处理 HTTP/WS external protocol。

Rust 迁移必须保持这些事实 owner，不能因为 Rust actor/task 模型而把它们装进一个串行 loop。

### 2.2 三类 model 必须分开

| 类别 | canonical owner/目标 | consumer |
| --- | --- | --- |
| Router↔Runtime wire model | `skiff-runtime-transport` 及必要的低层 request contract | Router、Runtime |
| compiler/Router/Runtime artifact model | `skiff-artifact-model`、`skiff-artifact-identity`、`skiff-deployment` strict reader | compiler/deployment/Router/Runtime |
| Router/platform durable activation model | canonical deployment/persistence crate中的 DTO/pure reducer；Mongo adapter Router-owned | Router/deployment tooling；不是 Runtime wire contract |

第三类即使物理位于 shared crate，也不能以“Router/Runtime shared model”名义扩大 Runtime 或 Router 的依赖面。
Runtime 只消费 activation prepare/commit/abort wire projection，不消费 Mongo durable record。

### 2.3 Shared Cargo closure 必须先收窄

当前 `skiff-runtime-transport` 传递依赖 `skiff-runtime-model`，后者暴露 request heap、runtime value graph、resource
等宽 execution surface。Rust Router 的目标依赖 closure 不应因此包含 Runtime host/model internals。

M0 必须通过 `cargo metadata` 做出显式决策并实施：

- 把 transport 真正需要的 opaque wire/service-error facts 下沉到 transport/request-contract 低层 crate；
- `skiff-router` 不得直接或传递依赖宽 `skiff-runtime-model`、runtime-host、eval 或 request execution；
- shared model 的 verify owner 仍由对应 canonical Rust subject 持有；Router subject增加 consumer gate，不复制 owner。

若无法在不引入宽 Runtime execution model 的情况下构建 Router consumer，停止 handler 开发，先修 crate boundary。

### 2.4 Actor routing projection contract

当前 TS Router 扫描 `PackageArtifact/File IR` 构造 actor method catalog，违反 canonical topology。必须定义最小
actor routing projection：stable actor ref、method admission/implementation identity、exact deployment binding；
不含 source、File IR 或 executable payload。

这项工作按 lane 拆开，而不是阻塞整个 skeleton：

```text
A0 freeze projection schema/owner/identity generation
  ├─ A1 compiler/deployment producer
  ├─ A2 TS Router strict consumer + production hard cut
  └─ A3 Rust strict reader/consumer
```

A1/A2/A3 可并行；E-actor-rust要求A1/A3，E-actor-parity和最终cutover再要求A2。A0可与transport M0同时
设计，但schema/identity
owner 必须在各 consumer 编码前冻结。

### 2.5 TS baseline cleanup 分成前置与并行项

以下是 skeleton listener/config freeze 的硬前置 C0：

- 把 `?detail=loop-risk` projection 移到 production `AssemblyControlPlane`，更新 evaluator/self-test/live baseline；
- 将 canonical control contract 统一到 `/__skiff/activate-assembly`；生产/tooling/tests 移除 stale reload URL；
- 更新 repo/workspace `AGENTS.md`、`scripts/README.md`、local instance checks 和 config surface。

C0包含独立`C-config`：冻结唯一Router config schema、defaults、relative-path resolution、secret redaction、
unknown-key policy和golden invalid corpus。先让TS parser与所有renderer hard-cut消费该contract，再让Rust消费同一
corpus。当前renderer输出但TS不声明/消费的字段（包括`ecosystemStoreCliPath`）必须删除或迁到真实owner，不能靠
TS忽略unknown key而让Rust保留。instance supervisor的迁移期`router.implementation`属于instance config，
不混入Router process config。

production-unreachable legacy manifest/http/control/telemetry 实现与 tests 的删除、test inventory 和 exception ledger
可以与早期 skeleton 并行，但必须在相应 Rust capability freeze 和 cutover 前完成。

Checkpoint A、H-registration-cut和H-spawn-parent-cut完成后冻结artifact/wire/Mongo/control、actor authority、
HTTP/WS external semantics。阻断迁移的 bug fix
先更新 canonical contract/corpus，再修改所有消费者。

## 3. Target Topology and State Owners

### 3.1 外部拓扑不变

```text
client / ingress
  -> skiff-router public HTTP + client WebSocket
  -> skiff-router runtime/control listener
       <- runtime actively connects over /runtime

skiff-router
  -> shared artifact filesystem (read-only routing records)
  -> MongoDB (activation state + audit only)
```

Rust Router 不启动 Runtime、Mongo 或 telemetry。

### 3.2 Owner 以 invariant 命名

以下是职责合同，不要求每项恰好一个文件，但任何合并都必须先写出共同 invariant 和 sequence test：

| Owner | 唯一拥有 | 明确不拥有 |
| --- | --- | --- |
| `ActiveRoutingEpochStore` | 当前 immutable routing epoch 的原子 publication | pending activation、session eligibility cache、pin map |
| `RuntimeRegistrationDirectory` | live `RuntimeSessionEpoch`、registered assembly tuple、capability index、socket handle、replica/epoch 双索引 | active/draining 副本、capacity/pending、health history |
| `RuntimeHealthLedger` | current/retained health observation | routing eligibility、socket ownership |
| `RuntimeAdmissionPool` | per-session capacity permits、selection cursor/policy | session truth、request pending、active routing epoch |
| `RequestDispatcher` | ordinary unary/stream 与 derived function-spawn correlation、terminal、reservation token | actor-method invocation、peer WS correlation、socket |
| `ClientConnectionIndex` | logical client connection、business identity replacement、`ClientSocketGeneration` | Runtime generation pin、broker pending |
| `RuntimeGenerationPinLedger` | Runtime generation acquire/release pending/cache/session attachment | client business index、peer RPC correlation |
| `WebSocketRequestBroker` | peer request/response correlation、deadline、tombstone、captured socket generation | ordinary dispatcher pending、connection replacement policy |
| stateless `ActorMethodCatalogView` | 对显式 `Arc<RoutingEpoch>` 中 actor index 的 typed query | 独立 index、mailbox、refresh/publication、actor live state |
| `ActorOwnershipRegistry` | actor identity、incarnation、current owner fence、authoritative claim reservation/commit | activation request correlation、invocation correlation、timer |
| `ActorActivationRequestBroker` | get-or-create operation dedup、activation request/ACK correlation | actor key上的claim truth、invocation、lease scheduling |
| `ActorInvocationRelay` | actor method invocation/return/error/cancel correlation | owner registry mutation、owner-control ACK |
| `ActorOwnerControlBroker` | claim/renew/evict 等 owner-control correlation | method invocation、idle timing |
| `ActorLeaseExpiryScheduler` | lease/idle deadline scheduling和eviction trigger | actor registry truth、control correlation |
| `ActivationStateRepository` | durable DTO/revision/audit、Mongo indexes、read/CAS/retry | coordinator transaction、routing epoch |
| `ActivationCoordinator` | durable activation transaction lifecycle和live/recovery participant binding | active epoch storage、session mutation、socket write |
| pre-auth/per-session/per-client task | `RuntimeConnectionEpoch`、physical socket halves、bounded ingress/outbound queue、abort handle | logical routing/pending maps |
| `HealthAggregator` | owner-published read-only snapshots | 反向修改任一 owner |
| `RouterSupervisor` | config、construction、listener/task join、shutdown | 所有上述业务 mutable state |

`Manager`、`Coordinator`、`Registry` 等名称必须在模块 contract 中补充 invariant；“都与 actor/WS/runtime 有关”
不是合并理由。禁止生产类型 `RouterCore`、`CoreState`、`CoreCommand`。

`RequestDispatcher` 的 function spawn 仅指 ordinary request 派生的 function-spawn correlation；actor-method spawn
归 `ActorInvocationRelay`/actor authority path。

concurrent first-owner的authoritative transition只在`ActorOwnershipRegistry`：它原子reserve actor key并签发
`ActorClaimToken`，`ActorActivationRequestBroker`持token执行operation/dedup，`ActorOwnerControlBroker`只关联具体
activate/renew/evict wire request，最终commit/abort必须带token回registry。broker不能各存一份claim truth。

`RuntimeRegistrationDirectory` 使用两张 exact index：

```text
current_by_replica: ReplicaId -> RuntimeSessionEpoch
sessions_by_epoch: RuntimeSessionEpoch -> SessionRecord
```

replacement先标记并cancel old epoch，再安装new epoch为current；old close barrier只能删除
`sessions_by_epoch[old]`，不能删除`current_by_replica[new]`。必须测试old disconnect/new registration/selection并发。

`RuntimeRegistrationTransition`定义同一physical session在activation commit后的re-register：捕获current routing
epoch，验证register exact tuple，原子更新`sessions_by_epoch[session].registered_tuple`和registration revision；
`current_by_replica`仍指同一session。candidate query只读取一个完整revision。stale tuple关闭exact session，exact
duplicate幂等，new-generation-before-epoch-swap拒绝；测试register update与admission/epoch swap并发。

physical accept时replica identity尚未知，先创建
`RuntimeConnectionEpoch { opaque_connection_id, generation }`，它拥有socket/cancellation/bootstrap writer和握手
deadline。未绑定connection只允许capabilities/handshake family，其它frame关闭connection。capabilities验证并固定
replica identity后才绑定`RuntimeSessionEpoch`；assembly register成功后session才进入routable directory。
pre-auth connections有独立总量上限和bootstrap/capabilities/register timeout，测试before-capability、
capability-to-register和replacement期间close。

### 3.3 Active routing 只有一个 authority

`ActiveRoutingEpochStore` 使用原子 `Arc` replacement 发布：

```text
RoutingEpoch {
  environment,
  assembly_generation,
  assembly_identity,
  config_snapshot_id,
  immutable ingress/deployment/actor routing projection
}
```

`RuntimeRegistrationDirectory` 只保存 session 自己注册的 exact tuple，不保存 active/draining 状态。候选资格由
stateless `RuntimeCandidateQuery` 统一投影：读取captured routing epoch和directory的exact registered tuple/
capability，拒绝cancelled session；为保持当前语义，heartbeat freshness不参与admission/activation，
`RuntimeHealthLedger`只服务health projection。新 admission：

1. 捕获一次当前 `Arc<RoutingEpoch>`；
2. 通过同一个 `RuntimeCandidateQuery` 按该 epoch 的 exact deployment查询candidates；
3. candidate query返回
   `RegisteredSessionLease { session_epoch, registration_revision, exact_registered_tuple, cancellation }`；
4. `RuntimeAdmissionPool`从leases中选择并reserve permit；
5. enqueue前原子revalidate session epoch、registration revision、exact tuple和cancellation，失败释放permit并
   重选或fail closed；
6. pending持有routing epoch、registered session lease和permit，terminal时一次释放。

old request/WS 通过持有旧 `Arc<RoutingEpoch>`/typed lease 延续，不需要全局 pin map。actor method index由artifact
loader在构造candidate epoch时一次构造，并属于immutable epoch；`ActorMethodCatalogView`只查询caller显式捕获的
epoch lease，不存在独立refresh。health 中 active/draining 为
active epoch 与 registered session tuple 的派生 projection，不形成第二事实源。

### 3.4 Identity/fence 类型不可互换

至少定义独立 newtype：

- `AssemblyGeneration` / `RoutingEpochId`；
- `RuntimeConnectionEpoch { opaque_connection_id, generation }`；
- `RuntimeSessionEpoch { replica_id, connection_generation }`；
- `ClientSocketGeneration { connection_id, generation }`；
- `RuntimeGenerationLeaseId`；
- `ActorIncarnationFence`；
- `ActivationId` / `ActivationParticipantBinding`。

禁止通用 `ConnectionGenerationFence` 或裸字符串跨 domain 传递后重新猜 owner。

### 3.5 真实 Runtime handshake 合同

`M-registration`先冻结并实现byte-exact sequence corpus；随后`H-registration-cut`才允许current TS Router/Runtime
同时消费并硬切：

```text
accept RuntimeConnectionEpoch
-> Router sends router.bootstrap
-> Runtime sends runtime.capabilities
-> bind RuntimeSessionEpoch / acquire installed-consumer permits
-> Runtime sends assembly.activation:Register
-> RuntimeRegistrationTransition validates committed epoch and publishes routable revision
-> Router sends runtime.registered ACK
-> Runtime starts runtime.health
```

`assembly.activation:Register`属于activation envelope的registration variant，但state owner仍是
`RuntimeRegistrationDirectory`；stateless envelope sink解码后委托`RegistrationFrameSink`，不让
`ActivationCoordinator`拥有session。`H-registration-cut`删除inbound legacy `runtime.register`，
`runtime.registered`只作为成功assembly Register的ACK。wrong order、identity change、duplicate/stale register和
ACK丢失都有严格terminal；health不能在ACK前被当作registered session observation。

### 3.6 Runtime disconnect 是 cancellation + barrier，不是尽力 fanout

每个 Runtime session 建立独立 cancellation token、abort handle和静态注册的 lifecycle consumer set。所有基于 session
建立的 request、broker、generation、actor、activation pending捕获同一个 session cancellation token。

每个installed component通过descriptor声明是否持有session-keyed state及其terminal sink。当前进程的
`RouterComponents`在启动时生成静态consumer manifest；session在发布进directory前只为本次composition中实际
安装并声明session state的consumers取得owned terminal permit，任一失败则拒绝registration。新增capability时
原子合入component + manifest + tests并通过进程重启生效，不在运行中动态扩manifest；未安装/无状态sink不占permit。

final composition包括`RuntimeAdmissionPool`、`RuntimeHealthLedger`、`RequestDispatcher`、
`RuntimeGenerationPinLedger`、`WebSocketRequestBroker`、已安装actor session owners和`ActivationCoordinator`。
checker验证任一installed session-keyed component都在manifest，E-cutover另验证最终全集。Runtime session总数有
固定/operator-configured上限，permit绑定session epoch并在ACK后释放。health retained history不得持有permit/
socket/eligibility；cancellation watcher独立于ordinary mailbox dequeue。

close protocol：

1. session task 原子触发 cancellation token；pending 即使 mailbox 饱和也能观察 session terminal；
2. 通过为 terminal 预留的 mailbox slot 向已注册 consumer 发送
   `RuntimeSessionClosed(RuntimeSessionEpoch)`；
3. consumer exact-fence 幂等清理并 ACK；
4. close barrier 等待全部 ACK，随后 directory 删除 exact session、writer drain/drop permits；
5. delivery/ACK 超时或 reserved slot 失效时 Router process fail-stop。必须测试同时关闭最大允许session数；进程
   重启会清除全部 ephemeral state；不能
   声称重启补偿一次漏掉的 ephemeral disconnect；
6. 只有 durable activation state在 restart 后执行 persistence reconciliation。

sequence/saturation test必须覆盖所有 consumer pending 最终归零。writer queue 满时通过独立 abort handle关闭
socket，不等待队列接受 close frame。

### 3.7 Client socket lifecycle 是独立 finalization protocol

`ClientSocketGeneration`有自己的cancellation token、installed-consumer manifest和幂等finalizer，不能假设
Runtime disconnect协议自动覆盖client状态：

1. exact socket generation标记closing，先从business/current index撤销，new generation可独立安装；
2. 原子触发client cancellation；broker即使mailbox饱和也能观察terminal；
3. `WebSocketRequestBroker` detach exact generation：terminal outbound、cancel inbound dispatcher requests、
   安装bounded tombstones并ACK；
4. `RuntimeGenerationPinLedger` release exact generation，`RequestDispatcher` terminal matching inbound，writer
   close/drain并分别ACK；
5. finalization barrier完成后删除old generation record；old finalizer不得删除replacement generation。

peer close、business replacement、slow-client overflow、Runtime pin loss和shutdown都进入同一finalizer。release
timeout按current canonical contract完成client terminal并把exact Runtime session视为protocol-unavailable/关闭，
不得静默保留pin；C-client-lifecycle必须冻结最终行为。测试覆盖replacement/peer close/runtime disconnect/
shutdown四向竞态、broker saturation和captured writer stale write。

### 3.8 Boundedness

- 每个 owner 只串行化自己的 invariant，不建立进程级 command loop。
- 不跨 `.await` 持有 owner state mutex；优先 owner task + bounded typed mailbox或纯 reducer。
- domain mailbox 明确 control/terminal reserved capacity、data capacity、公平策略和 full/closed terminal。
- writer queue 同时受 frame/byte permit限制；owner只 non-blocking reserve/`try_send`。
- outbound permit reserve 后随 queued item转移给 writer；completion/drop恰好释放一次，disconnect只close+drain。
- Runtime reader不无限等待 data mailbox；per-session ingress frame/byte budget超限则 abort exact session。
- deadline在 dequeue/admission/dispatch 前重检。
- business payload 为 immutable opaque bytes/lexical slice。
- sync `CanonicalArtifactStore` 只经有 semaphore/timeout/shutdown/health 的 bounded `spawn_blocking` pool调用。

## 4. Activation Protocol

### 4.1 Live transaction

Durable `PendingActivation` 保留当前 replica IDs，不写 ephemeral session epoch。live transaction另捕获
`ActivationParticipantBinding { replica_id, RuntimeSessionEpoch }`。

1. coordinator读取 current durable revision和当前 active epoch。
2. blocking loader加载/验证 candidate `RoutingEpoch`。
3. 通过`RuntimeCandidateQuery`冻结与current epoch exact matching的replica IDs及其
   `RegisteredSessionLease`；
   “healthy”不另行引入heartbeat eligibility语义，并立即校验所有cancellation/current-by-replica binding。
4. durable prepare CAS前再次revalidate session epoch、registration revision、exact tuple和cancellation；失败不写
   pending。成功后写pending replica IDs。
5. send prepare前再次revalidate并non-blocking enqueue到每个exact session；durable commit decision尚未开始前，任一replacement/
   disconnect/cancellation触发durable abort。
6. ACK按 live participant binding校验；stale/new session ACK拒绝。
7. 所有exact ACK后再次校验live bindings，然后开始durable commit CAS。CAS一旦发出，outcome由durable state
   authoritative；断连/timeout不能再直接假定abort，必须读取state reconcile。
8. durable commit成功后`ActiveRoutingEpochStore`执行一次已验证、不可失败的atomic `Arc` swap。
9. 向仍为exact binding的participants non-blocking enqueue commit；enqueue失败则abort对应session，不能回滚已
   committed durable state。Runtime reconnect按committed bootstrap收敛并丢弃staged candidate。
10. 新admission从swap后捕获新epoch；directory无需mutation/barrier。

Abort路径：durable abort成功后向仍为exact binding且已staged的session enqueue abort；enqueue失败则abort该
session。Runtime reconnect从durable committed bootstrap丢弃staged candidate。prepare/commit/abort writer queue
failure都按exact session fence处理。

`ActivationCoordinator` 可以 await自己的 persistence adapter，但不占用 session/snapshot/dispatcher mailbox，也不
跨 await 持有其它 owner state。“其它 domain owner 不等待 Mongo”，不是“coordinator 不允许 await Mongo”。

### 4.2 Cold recovery

进程启动读取 durable state：

- committed state先构造并发布 active epoch；
- durable pending先安装recovery transaction，但不在listener启动前等待participant；
- 打开Runtime listener后，expected replica注册时用replica IDs绑定新的exact sessions并发送prepare，允许session
  epoch变化；
- candidate加载失败则按 reducer durable abort；
- recovery transaction产生新的 ephemeral participant bindings；ACK仍按该 binding校验；
- 若进程在 durable commit 后、epoch swap 前退出，下一次启动从 committed state构造 epoch，不需要 pending
  publication token或第二份 eligibility cache。

public listener可在committed epoch发布后启动，但readiness/admission只有在至少存在满足current routing epoch的
session并通过E-session gate后开放；pending recovery在后台继续并通过health显式报告，不能阻塞Runtime listener
造成冷启动死锁。

live disconnect abort与cold recovery rebind是两个明确合同，shared corpus必须分别覆盖。

## 5. 实施依赖 DAG

### 5.1 PR 0a：可选择的进程与无 listener skeleton

这是第一个实施 PR，不等待完整 shared model，也不提前选择network stack或解析旧Router config。

instance config增加严格、迁移期专用的`router.implementation: ts | rust`；isolated fixture必须显式写，stable在
hard cut前默认`ts`。binary/source path由checkout + dev-home canonical resolver产生，不读ambient env。instance
supervisor、isolated runtime、differential harness和platform-source probe都把该config解析成唯一
`RouterProcessSpec`：

```text
RouterProcessSpec {
  implementation: ts | rust,
  config_path,
  ts_source_root?,
  rust_binary_path?
}
```

deploy/rollback从release manifest读取同一枚implementation选择。禁止各caller分别判断pnpm/tsx/binary。cutover
删除TS分支和迁移期implementation选择，最终spec恒为Rust。

同时交付：

- empty `skiff-router` Cargo package/binary，只支持direct process identity/lifecycle smoke，不绑定listener；
- `routerBinary` dev path、build/install placeholder、process matching和binary SHA-256 identity；
- implementation-neutral smoke harness；
- 过渡Rust subject精确命名为`router-rust`，拥有Router Cargo package并展开`router-rust-tests`；现有手工
  `router` selector暂时展开`router-ts-tests`与可真实通过的Rust fast leaves，避免同名owner冲突。

PR 0a不实现业务protocol，因此不违反“shared model先于handler”。它只消除后续测试/进程集成环。

### 5.2 C-net 与 PR 0b：final listener skeleton

C0 control/config硬前置完成后，C-net冻结Tokio runtime、HTTP server/upgrade library、body streaming type、WS
library、graceful shutdown和connection limits。Mongo driver归C-router-activation-state。用真实socket做empty HTTP/WS upgrade、connection
limit和shutdown probe；这里只冻结mechanism，不冻结HTTP业务ports。

随后PR 0b让Rust binary解析strict final config并启动public/runtime/control listeners。不得在C0/C-net前先实现
一套过渡listener/config再返工。

### 5.3 M0 与 per-family model packs

M0 与 PR 0a 可以并行设计，业务 handler必须等待其所需 contract pack。

M0只冻结跨 lane都依赖的最小骨架：

- transport envelope、frame family registry、direction和payload presence规则；
- `RuntimeConnectionEpoch`、capability-to-session binding和`RuntimeSessionEpoch` identity；
- artifact routing projection owner、identity generation和strict reader boundary；
- 三类 model分类及Cargo传递依赖closure；
- stable frame-family sink registration contract。

M0先做不改变wire bytes/public API的机械拆分，避免所有family争用当前集中式transport protocol文件；先建立
existing-file move/owner表，复用已有module，不创建第二套owner：

```text
session/bootstrap/health: 从 protocol.rs 移入独立module
request: runtime_assembly_request.rs + 从 protocol.rs 移出的frame DTO
activation: existing assembly_activation.rs
connection: existing connection_protocol.rs + websocket_generation_lifecycle.rs
actor: existing actor_method.rs + actor_owner.rs
spawn: 从 protocol.rs 移入独立module
protocol.rs/lib.rs: 极小registry + re-exports
```

telemetry和其它control family明确不归Router迁移lane。每个family有独立module/corpus/path owner；拆分前后golden
bytes必须一致。

M0后每个family按“接口决策→实现lane→consumer gate”并行推进，避免M-pack与W-model循环：

```text
C-model-registration -> W-model-registration -> M-registration
  bootstrap/capabilities/assembly.activation:Register/runtime.registered/health sequence
C-model-request -> W-model-request -> M-request
C-model-activation -> W-model-activation -> M-activation
  prepared/reject/prepare/commit/abort transaction wire
C-model-connection -> W-model-connection -> M-connection
C-model-actor -> W-model-actor -> M-actor
C-model-spawn -> W-model-spawn -> M-spawn -> H-spawn-parent-cut
C-model-bootstrap-wire -> W-model-bootstrap-wire -> M-bootstrap-wire
  Router->Runtime bootstrap assembly/config refs
C-model-artifact -> W-model-artifact -> M-artifact
  RuntimeAssemblyRef/ConfigSnapshotRef/strict artifact inputs
```

每个C-model先冻结exact shape/owner/direction；W-model实现DTO/codec/corpus；M-pack表示Router/Runtime/artifact
consumer gate完成，随后才解锁handler contract。各family由canonical shared crate subject拥有并行推进，不汇成
单一W-shared队列。

`C-model-spawn`必须解决当前wire只有`targetKind + callerRequestId`而没有parent domain的问题。本计划选择显式
新增closed enum `callerKind = request | actorInvocation`，并按现有规则升级runtime frame schema generation；
不采用靠字符串前缀猜测的fallback。W-model-spawn交付canonical codec/corpus，随后H-spawn-parent-cut让current
Runtime和TS Router同时硬切消费，删除旧shape且无兼容reader。C-spawn在该hard-cut后才解锁。

同理，`H-registration-cut`显式依赖M-registration；current TS Router/Runtime必须先通过shared corpus，再切真实
handshake。不得在canonical model gate前先分别实现新协议。

Router-only persistence另走：

```text
C-router-activation-state
  exact committed/pending DTO、revision、audit、read/CAS/retry/index/driver contract
  -> W-activation-state-repository
  -> P-activation-state gate
```

`CommittedActivationBootstrapReader`只消费repository read-only port，把durable record投影成`M-bootstrap-wire`/
`M-artifact` refs；Runtime不消费durable corpus。coordinator和bootstrap共用同一个repository instance，禁止临时
bootstrap Mongo reader后期替换。

M0 gate：`cargo metadata`证明Router consumer没有宽Runtime execution依赖；空Router consumer可编译共享
envelope/connection identity；新增family先改registry。各M-pack只要求其真实consumer直接消费同一owner corpus；
durable P-gate只要求Router/deployment tooling consumer。

### 5.4 按 lane 解锁的 contract packs

不存在全局“所有 ports冻结”barrier：

```text
C-bootstrap + M-bootstrap-wire + M-artifact + P-activation-state:
  repository read port + durable-to-shared projection + strict loader + initial ActiveRoutingEpoch publication
  -> W-bootstrap + E-bootstrap

C-session + M-registration: connection/session task、handshake、directory、cancellation/barrier/demux ports
  -> W-session + E-session

C-routing-query:
  captured RoutingEpoch + immutable registration revision + cancellation
  -> exact RuntimeSessionEpoch candidates
  -> W-routing-query；W-dispatch/W-activation共同消费

C-dispatch + M-request: routing epoch capture/candidates/admission permit/request terminal
  -> W-dispatch + W-http + E-dispatch/E-http

C-activation-coordinator + M-activation + P-activation-state:
  participant binding + repository mutation + publish port/recovery
  -> W-activation + E-activation

C-client-lifecycle + C-ws + M-connection:
  client cancellation/finalizer barrier + client index + generation ledger + broker attachment/fences
  -> W-ws + E-ws

C-actor + M-actor: catalog view + ownership + activation request + invocation + control + lease ports
  -> W-actor + E-actor-rust/E-actor-parity

C-spawn + M-spawn + H-spawn-parent-cut:
  FunctionSpawnParentResolver + ActorSpawnParentResolver + stateless SpawnSubmitRouter
  -> W-dispatch/W-actor + E-actor-rust

C-process-lifecycle（PR 0b后冻结并随installed components扩展）:
  stop public/control admission
  -> stop new activation + reconcile in-flight durable decision
  -> drain HTTP/client WS finalizers
  -> terminal dispatcher/broker/actor pending
  -> release Runtime generation leases
  -> close Runtime sessions via barrier
  -> join blocking loader/tasks/timers
  -> close Mongo
```

每个 contract pack必须定义：唯一 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/
replacement/shutdown terminal、health fields、fake seam和至少一条真实边界probe。某 pack通过即可开始对应实现，
不等待其它 pack。

process-lifecycle每步有总deadline，超时非零退出/fail-stop；每新增lane都扩composition shutdown test，不等cutover。

`M-spawn/C-spawn`使用不可碰撞的typed parent kind/correlation namespace。两个resolver分别从
`RequestDispatcher`和`ActorInvocationRelay`返回fenced authority snapshot；stateless `SpawnSubmitRouter`只按
exact parent kind选择，sink不拥有pending。必须测试collision、parent terminal和replacement竞态。

### 5.5 Demux 与 composition 不成为 merge hotspot

M0冻结 closed frame-family registry和稳定 sink bundle：

```text
RuntimeFrameSinks {
  session: RuntimeSessionFrameSink,
  request: RequestFrameSink,
  activation: AssemblyActivationSinks {
    registration: RegistrationFrameSink,
    transaction: ActivationTransactionFrameSink
  },
  connection: ConnectionFrameSink,
  actor: ActorFrameSink,
  spawn: SpawnSubmitRouter
}
```

`RuntimeFrameDemux` owner只做 framing、direction、source session fence和按 family分发；各 lane实现自己的 sink，
不修改中央 match。新增 family属于shared model变更，不是普通 feature PR。

`RuntimeSessionFrameSink`只处理Runtime→Router capabilities/health/session controls；activation envelope中的
`Register`由stateless family adapter交`RegistrationFrameSink`，其它transaction variants交
`ActivationTransactionFrameSink`。registration sink调用`RuntimeRegistrationTransition`并发送registered ACK，
不调用coordinator。Router→Runtime bootstrap由connection task通过无状态`RuntimeBootstrapProvider`从captured
active epoch/config构造并写入writer。它们不回流中央demux state。

每个 domain提供固定 factory/port conformance。`RouterSupervisor` composition由明确 integration owner维护，消费
稳定 `RouterComponents` manifest。每个 PR必须构建并运行公共 composition test，但不要求每个 workstream都编辑
同一个 supervisor文件。

## 6. Parallel Workstreams

workstream从各自 contract pack通过后开始，不等待一条全局 D milestone。

| Lane | 前置 | 独占实现 | 首个真实边界 |
| --- | --- | --- | --- |
| W-process/tooling | PR 0a | process spec、build/instance/verify/deploy/rollback builder | TS/Rust显式选择、无listener binary lifecycle |
| W-model-* | M0 + 对应C-model接口决策 | family-specific canonical DTO/codec/corpus；交付对应M-pack | 真实consumer同读同一corpus |
| W-artifact | A0+C-model-artifact | A1 producer、A3 strict reader、loader；交付M-artifact | compiler artifact被Router/Runtime同读 |
| W-TS-projection | A0 | A2 TS strict consumer/hard cut | current TS actor full-chain |
| W-activation-state-repository | C-router-activation-state | 唯一Mongo driver/index/read/CAS/audit/retry实现；交付P-activation-state | bootstrap/coordinator共用真实repository |
| W-bootstrap/foundation | C-bootstrap + repository read port | durable projection、strict loader、`ActiveRoutingEpochStore`及publish port | real committed artifact/bootstrap |
| W-session | M-registration+C-session | connection/session tasks、handshake、directory、health、barrier、demux | 真实Runtime WS restart/reconnect/saturation |
| W-routing-query | C-routing-query + session/epoch ports | stateless exact candidate projection | dispatch/activation共用sequence corpus |
| W-dispatch | M-request+C-dispatch + routing query/session ports | epoch capture、admission pool、dispatcher | fake ingress -> dispatcher -> real Runtime |
| W-http | C-net + C-dispatch port | HTTP socket、selector、body/stream/CORS | real HTTP -> fake dispatcher；E-http再接real Runtime |
| W-activation | M-activation+C-activation-coordinator + repository/routing query/session/publish ports | coordinator；只消费repository和`PublishCommittedEpoch` | 临时Mongo replica set + real/fake participants |
| W-WebSocket | C-net+C-client-lifecycle+M-connection+C-ws + session/dispatch ports | client finalizer/index、generation ledger、broker、JSON-RPC | 真实client WS -> real Runtime；chat归E-chat |
| W-actor | M-actor/M-spawn+C-actor/C-spawn + A3/session/dispatch ports | actor六个owners、spawn consumer | two real Runtime replicas full-chain |
| W-acceptance | PR 0a | process-neutral scenarios、health/live selectors、matrix | 每个lane首个vertical slice |

### 6.1 工作流合入规则

- shared model/canonical crate由既有 Rust subject owner；Router lane只增加consumer test。
- contract pack和owner文件有单一 reviewer/merge owner；lane不得复制DTO或建立私有兼容层。
- 当前wire没有Router capability advertisement。incomplete handler可以编译进isolated Rust binary，但收到未实现
  family必须终止exact Runtime session；checked-in test gate manifest只描述哪些scenario预期可用，不进入wire或
  production feature flag。若未来需要Router capability negotiation，另立wire generation checkpoint。
- “每 PR composition”指运行公共binary/port conformance，不是所有人修改composition root。
- fake用于并行开发，但Definition of Done必须包含真实socket/Mongo/artifact/Runtime一侧。
- 接口变化先改contract/corpus/sequence test，再更新consumer；避免长期分支。

### 6.2 Tooling 从 PR 0a 持续推进

W-process/tooling不是最终阶段：

1. PR 0a：process selection、empty binary、dev path、process match、Rust consumer task。
2. PR 0b/E-bootstrap：final listeners、instance build/up真正安装Router binary。
3. E-session：完成TS→Rust→TS process/bootstrap/register/health/reconnect/shutdown roundtrip，不声称unary。
4. E-http：完成第一次可比较的external HTTP unary rollback roundtrip。
5. E-activation：加入Mongo recovery roundtrip和managed selector。
6. E-ws/E-actor-parity：逐步扩展rollback smoke，而不是cutover前一次补。
7. release candidate：生成最终immutable TS rollback unit并在真实Linux/PM2 clean host演练。

Rust build source key不得使用手写不完整目录表。`rsUnit`通过 `cargo metadata`计算workspace-local transitive
package/source closure，纳入 Cargo.toml、Cargo.lock、build scripts和toolchain inputs。tooling test逐个改变
artifact-model、identity、deployment、transport/request-contract等允许依赖，断言Router source key失效。

## 7. Capability Enablement DAG

Gate ID不表示开发或验收顺序。它表示checked-in tests已证明isolated Rust binary拥有对应行为，不是wire
capability advertisement；生产TS继续唯一服务，直到E-cutover依赖全部完成。

```text
PR0a + C0 + C-net -> PR0b
M-bootstrap-wire + M-artifact + P-activation-state + A0/A1/A3
  + C-bootstrap + W-artifact + W-activation-state-repository + W-bootstrap
  -> E-bootstrap
PR0b + E-bootstrap + M-registration + H-registration-cut + C-session + W-session + C-process-lifecycle
  -> E-session
E-session + M-request + C-routing-query + W-routing-query + C-dispatch + W-dispatch
  -> E-dispatch
E-session + M-activation + P-activation-state + C-routing-query + W-routing-query
  + C-activation-coordinator + W-activation
  -> E-activation
E-session + E-dispatch + W-http -> E-http
E-session + E-dispatch + C-net + C-client-lifecycle + M-connection + C-ws + W-WebSocket
  -> E-ws
E-session + E-dispatch + A0/A1/A3 + M-actor/M-spawn + H-spawn-parent-cut
  + C-actor/C-spawn + W-actor
  -> E-actor-rust
E-actor-rust + A2 TS hard cut + differential full-chain -> E-actor-parity
E-http + E-ws + E-activation + pinned service artifacts -> E-chat
all gates including E-actor-parity/E-chat + ops gates -> E-cutover
```

### E-bootstrap

- `CommittedActivationBootstrapReader`只读committed durable state；初始skeleton遇到pending时fail closed，完整
  recovery归E-activation。
- strict assembly/config reader构造完整`RoutingEpoch`含ingress/deployment/actor indexes并原子发布。
- missing/malformed/identity mismatch、blocking loader saturation和shutdown全部fail closed/归零。

### E-session

- real handshake corpus、pre-auth`RuntimeConnectionEpoch` limit/timeout、capability binding、
  `assembly.activation:Register`、registered ACK、health、session barrier/reconnect。
- ingress/outbound/mailbox saturation和shutdown归零。
- 未实现frame family按M0 direction规则终止exact session，不存在Router wire capability advertisement。

### E-dispatch

- capture routing epoch -> exact `RegisteredSessionLease` candidates -> capacity permit -> registration revision/tuple/
  cancellation revalidation -> request pending链路完整。
- missing/duplicate/invalid selector、wrong deployment/entry、timeout、disconnect fail closed。
- selection/replacement/disconnect竞态不双计capacity、不遗留pending。

### E-activation

- live participant session binding、prepare/reject/commit/abort/stale ACK和cold recovery语义符合第4节。
- managed真实Mongo验证CAS revision、retry不重复audit、audit失败回滚、restart/rebind、decision前disconnect abort
  及decision后durable outcome reconcile。
- durable commit后atomic swap active epoch，再向still-exact sessions发送commit；durable abort后发送abort；control
  enqueue失败abort session并靠committed bootstrap收敛，无session eligibility副本或pending publication token。
- `router-live:activation-mongo`与`router-live:activation-full-chain`均通过，后者覆盖real Runtime re-register和
  new-generation request。

### E-http

- trusted selector、service-scoped ingress、typed/raw opaque payload、unary/stream mapping。
- stream sequencing、cumulative response ceiling、backpressure、disconnect/cancel/deadline。
- CORS preflight/service-managed/platform error和test-dispatch isolation。
- 任意竞态一个external terminal、至多一次cancel，pending/permit/timer归零。

### E-ws

- client connection/business replacement、socket generation、Runtime generation leases和broker correlation各自owner。
- single writer、frame/byte budget、captured writer fence、late result isolation。
- business params/result/error data保持lexical opaque slice；control members strict。
- numeric id先按lexeme验证safe integer，再canonicalize：`1e0` -> `1`、`-0` -> `0`。
- parser corpus/fuzz、slow-client saturation、disconnect races通过；不等待HTTP/chat gate。

### E-chat

- E-http、E-ws、E-activation均通过，isolated Rust instance使用本次gate固定的service artifact manifest。
- `router-live:chat`记录Skiff commit SHA、internals commit SHA、skiff-packages commit SHA和所有service artifact
  identities，再在`internals/agine`执行`npm run e2e:chat-smoke`；该cross-repo full chain成为E-cutover前常驻gate。

### E-actor-rust

- catalog只读A0 routing projection，不读PackageArtifact/File IR。
- ownership registry claim token、activation request broker、invocation、owner control、lease scheduler分别通过
  owner/sequence tests。
- function spawn和actor-method spawn parent authority明确；accepted spawn与parent生命周期分离。
- two-replica full-chain、disconnect/replacement/concurrent claim/lease race/spawn mismatch fail closed。
- actor invocation/control/lease/timer全部归零。

### E-actor-parity

- A2 TS Router已硬切只读canonical actor projection；TS/Rust differential不再拿File IR reader作为baseline。
- two-replica actor/get-or-create/spawn full-chain在两实现无未解释差异。

### E-cutover

依赖全部 capability gate、A2 TS projection hard cut、所有 permanent tests、rollback和ops gate。这里只切default和
删除TS，不首次实现任何build/instance/deploy/lifecycle。

## 8. Named Integration and Verification Tasks

PR 0a只注册当时能真实通过的fast tasks；每个vertical slice首次可用时再原子注册对应非空task。不能预注册
skip/pass的未来能力，也不能把managed live task放进default `router` selector。

| Selector / task ID | Owner | Cadence | 内容 |
| --- | --- | --- | --- |
| `router-rust` / `router-rust:contracts`（leaf `router-rust-contracts`） | Router Rust consumer | PR 0a起每PR | compile、port conformance、unit/corpus consumer |
| `router-rust-process-smoke` / `router-rust:process-smoke` | process seam | PR 0a起每PR | explicit TS/Rust resolver、direct identity/lifecycle，无listener |
| `router-rust-bootstrap-live` / `router-live:bootstrap` | bootstrap lane | E-bootstrap slice起required managed CI | compiler artifact、committed reader、initial epoch |
| `router-rust-session-live` / `router-live:session` | session harness | E-session slice起required managed CI | real Runtime bootstrap/register/reconnect/shutdown；无unary |
| `router-rust-dispatch-live` / `router-live:dispatch` | dispatch lane | E-dispatch slice起required managed CI | fake ingress -> admission/pending -> real Runtime |
| `router-rust-http-live` / `router-live:http` | HTTP lane | E-http slice起required managed CI | real HTTP->Router->Runtime；首次unary rollback |
| `router-rust-ws-live` / `router-live:ws` | WS lane | E-ws slice起相关PR/required CI | real WS/broker/generation |
| `router-rust-actor-live` / `router-live:actor` | actor lane | E-actor-rust slice起相关PR/required CI | two-replica actor chain |
| `router-activation-mongo-live` / `router-live:activation-mongo` | persistence lane | P-activation-state slice起required managed CI | reducer/CAS/retry/audit failure + temporary Mongo |
| `router-activation-full-chain-live` / `router-live:activation-full-chain` | activation integration | E-activation slice起required managed CI | real Router+Mongo+compiler artifact+Runtime commit/re-register/new request |
| `router-chat-full-chain-live` / `router-live:chat` | internals cross-repo workflow | E-chat依赖成熟后main/RC required | pinned Skiff/internals/packages commits + artifact manifest + Agine chat smoke |
| `router-clean-host-live` / `router-live:clean-host` | tooling/release | scheduled + RC | Linux binary/PM2，无pnpm/tsx/router node_modules |

迁移期间手工public `router` selector只展开`router-ts-tests`、Rust subject `router-rust`和
`router-rust-process-smoke`等fast/hermetic leaves；managed Runtime/Mongo/two-replica不进入default `pnpm test`。
cutover时一次原子registry transition：删除TS builder/manual graph，把Rust subject从`router-rust`改为`router`
唯一owner，并用registry transition test防止同名双owner或workspace member漏owner。

`router-rust-contracts`就是`router-rust` Rust subject自动生成的唯一Cargo test leaf，不再手工注册相同
cwd/command/args的第二份task；manual `router` graph展开subject selector而不是重复leaf。process smoke是不同命令。
graph transition test覆盖`implementation-tests`、manual `router`和Rust subject展开后的task去重。

现有live gates继续使用：

- external `runtime-live`显式提供activation URL、ingress URL、artifact root、environment、expected generation；
- `loop-risk-health-live`和cutover前`loop-risk-stress-live`使用显式canonical config；
- `internals/agine`运行`npm run e2e:chat-smoke`；
- `db-encrypted-storage-live`保留业务回归，但不用于证明Router无Node依赖；该证明只归`router-clean-host-live`。

CI落地而不是只写cadence：

- 现有`.github/workflows/verify.yml`的PR job运行fast contracts/process smoke；
- 新增`.github/workflows/router-rust-integration.yml`，对所有`pull_request`和`workflow_dispatch`触发；先运行
  cheap change classifier，非相关PR让required job显式成功，不能用workflow级`paths`导致required check缺失；
  相关PR运行managed Runtime/two-replica/Mongo jobs，启动临时replica set并显式选择Rust process；gate成熟后
  将该稳定job名设为required；
- 新增scheduled/`workflow_dispatch` release workflow运行clean-host、loop-risk和完整rollback；
- 若未来启用GitHub merge queue再增加`merge_group`，本计划不假设当前已启用。

task未实现且未出现在`verify --list`、workflow未实际调用前，不得标记gate已建立。

`router-live:chat`由private `internals` repository的trusted workflow拥有：它以请求中的public Skiff SHA checkout
Skiff，以workflow自身locked internals commit和显式skiff-packages SHA构建artifact manifest。public/fork PR不注入
private credentials；该gate在trusted main/RC或经授权`repository_dispatch`运行，并以稳定check name回报结果。
本地验证使用同一manifest schema和命令。E-cutover必须等待成功结果，不能用未pin的stable instance人工smoke代替。

E-actor-parity时，`router-live:actor`原子扩展为TS/Rust均消费canonical projection的differential full-chain，
checked-in task expectation与A2 hard cut在同一change中更新，避免Rust-only task被误当parity。

`router-live:activation-full-chain`必须逐步证明：activate HTTP→durable prepare→real Runtime prepared→durable
commit→epoch swap→Runtime commit→同session re-register→new-generation HTTP request成功，同时old captured epoch
request可按原lease完成。它不是Mongo adapter test的别名。

## 9. Continuous Integration Matrix

各链路从首个vertical slice起成为常驻task，不等所有capability完成：

```text
HTTP -> ActiveRoutingEpochStore -> RuntimeRegistrationDirectory
     -> RuntimeAdmissionPool -> RequestDispatcher -> real Runtime

ActivationCoordinator -> Mongo -> ActiveRoutingEpochStore -> Runtime sessions

Client WS -> ClientConnectionIndex -> RuntimeGenerationPinLedger
          -> WebSocketRequestBroker -> RequestDispatcher -> real Runtime

Actor catalog/ownership/claim/invocation/control/lease -> two real Runtime replicas

RouterProcessSpec -> build/install/instance/process identity -> shutdown/rollback
```

每条lane首个slice加入implementation-neutral differential scenario：TS/Rust使用独立端口、artifact root、runtime
home和Mongo namespace，不共享Runtime或镜像live traffic。对比HTTP、WS、Runtime frames、health、Mongo state/
audit和terminal counters。normalization只允许UUID、timestamp、ephemeral port、无语义log order。

每删除一个TS test，ledger标记retired/shared owner/Rust replacement/black-box replacement；不能以类型系统代替
observable test。

## 10. Concurrency, Sequence and Health Tests

- deadline、lease、idle、activation budget、generation release使用injected/paused time。
- 必测terminal vs timeout、disconnect vs response、close vs send、replacement vs stale release、ACK vs abort、
  lease expiry vs renewal、mailbox full vs lifecycle close。
- active epoch swap与session query并发时，请求只能捕获完整旧epoch或完整新epoch，不能混合tuple。
- session close barrier在每个consumer mailbox饱和下要么全部ACK归零，要么触发process fail-stop。
- 分别断言external terminal、cancel、capacity permit、routing lease、socket generation、Runtime generation lease、
  actor fence/timer释放至多一次且最终为零。

每个owner发布自己的health snapshot；`HealthAggregator`只聚合。至少暴露：

- active routing epoch；Runtime sessions/capabilities/health/cancellation barrier；
- admission permits/cursor、ordinary request pending；
- client connection generations、Runtime generation leases、broker pending/tombstone；
- actor ownership/claim/invocation/control/lease timers；
- activation durable/live/recovery transaction；
- per-owner mailbox reserved/data occupancy和saturation terminal；
- writer frame/byte queue、blocking loader、spawned tasks/timers/shutdown residue。

新增counter同步更新loop-risk evaluator required fields、missing/nonzero self-test和live fixture。日志/health不得含
Mongo URL、secret、业务payload、WS params/result/data、Authorization/cookie或完整query URL。

## 11. Build, Deploy and Rollback

### 11.1 Binary lifecycle前移

- `build-runtime-stack`使用Cargo-metadata source closure生成`build/runtime-stack/bin/skiff-router`。
- instance `build/up`构建/安装Router binary，`dev-runtime-paths`提供routerBinary，process match理解TS/Rust spec。
- refresh Runtime不重启Router，refresh Router不重启Runtime；worktree isolated instance使用当前checkout binary。
- deploy上传binary，PM2直接执行`--config`；删除Router rsync、remote pnpm/tsx。
- `--only router`只构建/部署Router，不隐式捆绑compiler。
- clean Linux/PM2 gate只提供binary、config、artifacts；PATH故意不含pnpm/tsx，完成start/health/Runtime reconnect/
  unary/shutdown。

### 11.2 Incremental rollback rehearsal

PR 0a定义rollback manifest/schema/builder和TS/Rust process commands。E-session首次演练TS→Rust→TS process/
bootstrap/register/health；E-http增加第一次unary，E-ws/E-actor-parity分别扩展smoke。

最终immutable TS unit必须自包含target-platform pinned Node runtime、最后TS source、materialized Router
dependencies（或随unit提供并校验的offline pnpm store + frozen install）、package/lockfile、process spec和所有
file/source identity。它在全新临时目录/clean host离线启动，禁止复用workspace`router/node_modules`或网络。
release candidate从头重建并验证该unit，不能把“安装收据”当成可启动依赖。

最终演练：

```text
stop admission
-> shutdown current Router
-> verify PID/listener exited
-> start target immutable process
-> Runtime reconnect exact committed tuple
-> activation/readiness
-> open admission
-> HTTP/WS/actor smoke
```

### 11.3 Hard cut

1. 同一release暂带Rust binary和已验证TS rollback unit，但实例只启动一个Router。
2. 停止TS admission/process，确认退出后启动Rust。
3. 等Runtime reconnect、activation reconciliation、readiness后开放admission。
4. 运行HTTP/WS/actor/chat/loop-risk/clean-host关联smoke并观察约定周期。
5. 独立commit删除TS Router和differential harness；发布系统保留上一完整release。

删除内容包括Router TS source/tests/package/lockfile/tsconfig/dist、CI install、remote install、tsx/pnpm process path。
`loop-risk-stress-node.mjs`的`ws`依赖归`scripts/package.json`。

残留gate：

```bash
rg --files router | rg '\.(ts|tsx)$'
test ! -e router/package.json
rg '@skiff/router|pnpm --dir router|tsx.*router'
```

前两项应无结果/成功；第三项在production/CI/tooling无结果，历史implementation record可保留。

不做双Router production canary。删除TS后回滚上一完整repository/build release。若必须改变durable state/wire/
artifact，立即停止并拆成独立generation checkpoint。

## 12. Risk Register

| 风险 | 等级 | Gate/缓解 |
| --- | --- | --- |
| 最后才集成导致关键路径持续暴露问题 | 高 | PR0 process seam；lane解锁；named tasks；vertical matrix |
| 全局shared/interface freeze形成waterfall | 高 | M0最小closure；per-lane contract packs |
| 万能Core换名为Actor/WS/Runtime容器 | 高 | invariant-level owners；合并需共同invariant+sequence proof |
| active routing双owner/混合epoch | 高 | 单一ActiveRoutingEpochStore；session只存registered tuple |
| disconnect事件丢失留下ephemeral state | 高 | shared cancellation token；reserved terminal；ACK barrier；fail-stop |
| selection/capacity/session replacement竞态 | 高 | exact candidates；permit owner；session revalidate/cancel |
| central demux/supervisor成为merge hotspot | 高 | closed family registry；sink bundle；integration owner；composition test |
| Cargo传递依赖把Runtime internals拉入Router | 高 | M0 metadata closure；抽低层wire facts；DAG gate |
| build source key漏shared crate导致stale binary | 高 | metadata-derived inputs；逐crate invalidation tests |
| tooling/rollback末期才接入 | 高 | W-process从PR0；每capability扩展roundtrip；clean-host gate |
| Mongo retry/audit/recovery与live语义混淆 | 高 | durable reducer corpus；live/cold binding分离；real replica set |
| HTTP/WS/actor行为漂移 | 高 | lane contract/real boundary/differential/fuzz/full-chain |
| pending/permit/lease/timer泄漏 | 高 | owner health；zero/nonzero/saturation/shutdown gates |

## 13. Completion Criteria

- 唯一生产Router是Rust `skiff-router` binary。
- Router/runtime wire和artifact DTO/identity/reader只有canonical owner，无TS/local mirror。
- Router不直接或传递依赖宽Runtime execution model、host/eval/request execution；Runtime不依赖Router。
- active routing只有`ActiveRoutingEpochStore`一个authority；session directory无active eligibility副本。
- ordinary pending、WS broker pending、client connection、Runtime generation lease、actor ownership/claim/invocation/
  control/lease和activation transaction各有独立owner。
- 无跨多个independent invariant的`RouterCore`/global state/万能actor coordinator。
- actor catalog只读canonical routing projection，不读PackageArtifact/File IR。
- PR/merge/release named tasks、real Runtime/Mongo/socket、chat/loop-risk/clean-host和rollback全部通过。
- build、instance、test-runner、deploy、PM2、binary refresh全部管理Rust binary。
- TypeScript Router source/tests/package/lockfile/dist/CI/remote install全部删除。
- 所有owner counters在success/error/disconnect/saturation/shutdown后归零。
- 长期owner/contract并入`doc/architecture/`；本文改为`complete`，不充当第二份架构规范。
