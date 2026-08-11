# Router（Rust）Architecture

本文是`skiff-router`进程、Router-owned mutable state与Runtime transport的长期内部契约。Release pointer、
artifact store与lazy-load规则只由
[`runtime-lazy-load-deployment.md`](runtime-lazy-load-deployment.md)定义；ServiceDeployment与gateway
identity只由[`package-service-contract-deployment.md`](package-service-contract-deployment.md)定义。本文不
复制它们的DTO或状态机。

## 1. 进程拓扑

Router与Runtime是两个独立Rust进程，不共享mutable state：

```text
client / ingress
  -> skiff-router public HTTP + client WebSocket
  -> skiff-router control/runtime listener
       <- runtime actively connects over /runtime

skiff-router
  -> shared immutable artifact store + release pointer table

runtime
  -> the same artifact store
  -> service DB transport supplied by Router bootstrap
```

Router负责external ingress、release resolution、Runtime selection、request/stream correlation、WebSocket
connection/broker与Actor routing。Runtime执行用户代码并按buildId lazy-load。Router不启动Runtime、MongoDB
或telemetry，不解析Package executable，也不持有deployment执行状态。

External request必须带受信`x-skiff-service`与`x-skiff-version`。Router严格解析
`(profile, serviceId, version) -> buildId` pointer，再在该deployment内解析gateway entry。缺selector、
pointer、entry或eligible Runtime都fail closed。

## 2. Connection bootstrap

Runtime主动连接后，Router恰好发送一次连接级bootstrap：

```text
artifactsPath
serviceDb.mongoUrl
http.maxResponseBytes
transport/schema capabilities
```

同一连接中bootstrap缺失、重复冲突或变更都fail closed。Runtime随后通告：

```text
RuntimeReplicaId
loadedBuildIds
lazyLoadCapability + artifactStoreIdentity
capabilities.platformErrorProjectionRegistry
transport capabilities
```

Loaded set可以增长或因本地eviction缩小；它是placement hint，不是release truth。Router不等待全体replica
加载某个build，也不等待多replica确认。Preload只能是fire-and-forget hint。

M3 hard cut使用`skiff-runtime-frame-v5`，其中
`runtime.capabilities.capabilities.platformErrorProjectionRegistry`是required strict
`PlatformErrorProjectionRegistryRef`，exact JSON shape为
`{ registryId: "skiff-platform-error-projections", registryVersion: 1,
fingerprint: "sha256:<64 lowercase hex>" }`。Descriptor绑定Runtime binary与session incarnation；同一session的
capabilities refresh只能重复相同exact值，冲突refresh必须终止session。Replica需要更换fingerprint时必须建立
new incarnation，不能原地改写registration facts。旧frame或缺失descriptor无registration成功路径。

## 3. State owners

禁止一个万能`RouterCore`或协调器。Router业务mutable state穷尽为pointer、session、routing与actor四个
domain；每个事实只能落在下表一个owner中：

| Owner | 唯一拥有 | 明确不拥有 |
| --- | --- | --- |
| `ReleasePointerIndex` | strict release pointer target与原子refresh | artifact/route write、Runtime image、request/session状态 |
| `RuntimeSessionDirectory` | live session/replica/runtime-transport socket、session incarnation、immutable platform-error registry descriptor、loaded build set、lazy-load/store capability、容量计数与permit、session health observation | release pointer、request terminal、Actor truth |
| `RequestRoutingState` | validated immutable ingress-routing view（含build registry descriptor）、exact-build/descriptor dispatch pin、unary/stream/task-attempt correlation、client connection/socket incarnation、peer WebSocket pending/tombstone/deadline | pointer mutation、session replacement、Actor ownership |
| `ActorRoutingState` | actor identity/incarnation、exact-build/descriptor owner fence/lease、instance get/create dedup、method与owner-control correlation、idle/expiry schedule | release pointer、newest/superseded build、ordinary request/peer WebSocket pending |

`RequestDispatcher`、`ClientConnectionIndex`与`WebSocketRequestBroker`只能是`RequestRoutingState`内互斥记录种类；
Actor registry、instance broker、invocation relay、owner-control broker与expiry scheduler只能是
`ActorRoutingState`内部结构。`RouterSupervisor`只负责process config、construction、listener/task join与shutdown，
不成为第五个业务state domain。Health端点按需读取上述owner投影，不能建立独立ledger或反向修改owner。

## 4. Routing and pinning invariants

一次新dispatch按以下顺序捕获事实：

```text
trusted service/version
  -> ReleasePointerIndex exact buildId
  -> strict routing authority(buildId, registry descriptor)
  -> RequestRoutingState exact gateway entry identity
  -> descriptor-matching Runtime session + capacity permit
  -> immutable DispatchOwnerPin(buildId, registry descriptor, deployment identity, session incarnation)
```

- Strict routing authority来自exact deployment PackageArtifact closure。Closure中的
  `platformErrorProjectionRegistry`必须唯一一致；mixed fingerprint不能产生routing view。Router只消费该typed
  metadata，不解析Package executable。
- Candidate是已注册该buildId且descriptor exact-match的session，或descriptor exact-match并声明同一artifact
  store lazy-load能力的session。Runtime loader仍对PackageArtifact、bytecode与binary descriptor做最终三方验证。
- Pointer在dispatch后变化不迁移request/stream/connection；新dispatch重新解析。
- Runtime load失败以该request的明确platform error收敛；Router不改投另一个build。
- Session replacement先cancel/close old incarnation再安装new incarnation；old finalizer不能删除replacement记录。
- Pending同时持有exact owner pin、session lease与session directory签发的permit，terminal恰好释放一次。
- 同进程service child由Runtime自己的boundary scheduler处理；Router不把service call降级为Package call。

WebSocket upgrade固定exact deployment build、registry descriptor、gateway entry与
`ClientSocketIncarnation`。JSON-RPC response必须精确匹配connection、socket incarnation、direction/profile与
transport id；pointer更新不迁移旧socket。

Actor routing 使用独立 `ActorIncarnationFence`，每个 live identity 的 owner fence 另钉住创建它的 exact
deployment `buildId`与registry descriptor。不同 identity 可以同时运行不同 build；同 identity 的不同 build
或descriptor请求直接拒绝，不触发
升级/逐出，也不刷新当前 owner 的 idle 时钟。实例因 idle、断连或 shutdown 销毁后，下一次 claim 由请求
自己的 build 决定，允许回退。`ActorRoutingState` 不保存 Actor release pointer、newest/superseded 集合或
ambient multi-service release state。

HTTP、client WebSocket、Actor与task attempt虽然分别由`RequestRoutingState`或`ActorRoutingState`拥有pending/
fence，但它们不得各自拥有registry truth；每条会触发Runtime执行的route都消费
`RuntimeSessionDirectory`保存的同一个immutable descriptor并与build routing authority exact-match。

## 5. Model ownership

| 类别 | canonical owner | Router职责 |
| --- | --- | --- |
| Router↔Runtime wire | `skiff-runtime-transport`与request contract | runtime-frame-v5 strict encode/decode、immutable session descriptor state |
| artifact/identity | `skiff-artifact-model`、`skiff-artifact-identity` | strict build/registry routing view，不解析code |
| release pointer | `runtime-lazy-load-deployment.md`定义的store | read/refresh/resolve target |
| gateway与service frame identity | package/service deployment文档 | strict ingress-routing view；service payload/operation保持opaque |
| VM/request execution | `bytecode-vm.md`与Runtime crates | 不依赖、不执行 |

`skiff-router`不得依赖runtime evaluator/VM/host execution。Runtime不依赖Router实现 crate。Unknown nested
wire/artifact field、schema mismatch、owner mismatch与跨build substitution都fail closed；没有dual-read或
display-name fallback。

Rolling upgrade可以同时注册不同registry fingerprint的session；routing exact-match使其只处理各自build。仍有
release/artifact引用旧descriptor时不得清退最后一个matching session。Health只投影该事实，不能放宽候选集。

## 6. Limits, health and rollback

`runtime.maxConcurrency`按Runtime WebSocket session限制ordinary pending request；满载时立即overload，不
排队。HTTP request/response limits由Router operator配置并按`../reference/runtime.md`执行。Actor/control
frame不计ordinary pending permit。

`/__router/health`是pointer、session capability、loaded build与loop-risk的只读projection，不是另一份
routing state。健康信息不能让不存在的pointer/build变得eligible。

Service rollback只原子把release pointer指回已验证旧buildId。Router binary rollback是process release
操作：停止接流量、shutdown运行中的process、启动目标immutable binary/config、等待Runtime reconnect，再做
HTTP/WS/Actor/chat smoke。Service rollback与Router process替换彼此独立。

## 7. Verification contract

至少验证：

- pointer resolution、missing/incompatible build与跨build frame substitution fail closed；
- loaded candidate与lazy-load candidate选择，load failure不fallback；
- runtime-frame-v4、缺失/非法descriptor、same-session descriptor mutation与跨fingerprint candidate都fail closed；
- new session incarnation可以声明不同fingerprint；旧fingerprint仍被release引用时保留最后一个matching session；
- deployment PackageArtifact closure的descriptor必须唯一一致，Router routing view与Runtime三方load验证一致；
- HTTP、WebSocket、Actor与task route都执行同一个build/session descriptor exact-match；
- pointer更新不迁移in-flight unary/stream/WebSocket；
- permit、pending、session、broker tombstone在所有terminal/disconnect路径归零；
- session replacement与client socket replacement的old finalizer不能删除new incarnation；
- Actor claim/install/release只由`ActorRoutingState`改变truth；同identity跨build拒绝、不同identity异版本并存、
  mismatch不刷新idle、idle销毁后任意build重新claim都有竞态测试；owner lease expiry
  不能抢在idle discard之前只清Router fence，测试必须同时证明Runtime instance已被exact discard或旧session
  已被fence；
- Router不依赖VM/eval crate，也不持有deployment execution image或跨service协调状态；
- real Router↔Runtime HTTP、WebSocket、Actor与Agine chat smoke通过。

具体迁移历史留在`doc/implementation/`，不再作为第二份架构规范。
