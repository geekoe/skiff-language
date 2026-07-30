# P5-F350 External ingress ecosystem migration audit result

状态：审计完成（三个 integration tree 与 F269 worktree 均只读；未实现、未运行 build/stable/live、未
push）。

## 1. 审计基线与结论

本结果只以任务指定的三个 committed tree 为 production 事实源：

| 仓库 | worktree / branch | HEAD | tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` / `codex/package-service-phase-05` | `6fe25aa1c2545d76f63e96b0261516cfdc288e99` | `a458352384a28a055103ae17f617724d4026077f` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` / `codex/package-service-phase-05` | `609551f0a65bfcc814ed4c894e4c333b4ffb10f1` | `3626636d9729a1b9c036fb4fe14d1139e6233b75` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` / `codex/package-service-phase-05` | `14ccfd417c9f45f00bd77015494cdd727e0f88dc` | `2327a766bcc6f32e7470b57420659fc991ef8a15` |

审计 worktree 起点同 Skiff HEAD/tree，分支为
`codex/p5-f350-ingress-ecosystem-audit`。三个指定 integration worktree 均存在；production source、
manifest、lockfile和任何 artifact store 均未写入。

结论：

1. 三仓库共有 20 个 tracked `service.yml`，其中 8 个声明 external ingress。8 个 manifest 合计
   113 条 HTTP route和2条WebSocket route：75条 route使用`operation`，40条使用`handler`；两种
   callable shape没有在同一route混用。
2. Internals四个真实service当前共48个ServiceContract operation。仅移除external-only API后应为8个：
   Account `21 -> 0`、Relay `17 -> 2`、AIHub `8 -> 6`、Agine `2 -> 0`。被移除的40个operation不是
   service dependency API。
3. Registry保持`20 -> 20`。官方`aliyunoss`、`http-session`、`openai`、`track`是package-only，
   没有ServiceContract或external ingress；六个官方test service为`kind: test`且`api.yml`均为`{}`。
4. Skiff另有一个compiler fixture需要`1 -> 0`，三个旧live service使用40条`handler` route但尚无
   canonical package/API/ServiceContract。更重要的是，test-runner还在两个Rust owner中手工合成
   `IngressSelector -> ContractOperationId`；它们不出现在`service.yml`搜索结果中，必须同批迁移。
5. 不能先逐service落地。当前compiler、artifact model、deployment、runtime、transport和Router都把
   ingress锚到ServiceContract operation，且compiler拒绝零operation contract。必须先有一个共享
   gateway-entry checkpoint，再扇出service目录。
6. F269已迁出的Internals test service source可以保留；其worktree本次完全未触碰。但旧Service API
   receipt、base assembly、operation identity和所有由旧test-runner ingress生成的证据都会失效，必须由
   F269 owner在共享checkpoint与真实service迁移合入后重跑。

## 2. 全量 service manifest 清单

### 2.1 Skiff：9个service root

| root / service id | kind / API | 当前ingress | 预期动作 |
| --- | --- | --- | --- |
| `compiler/tests/fixtures/router-websocket-fixture` / `example.com/websocket_fixture` | normal；`ping`一个Available public function | HTTP 1，`operation: ping`；route另有`host` | `ping() -> string`改为独立typed gateway handler，contract `1 -> 0`；更新compiler→Router artifact compatibility断言 |
| `runtime/encrypted-storage-live/default-service` / `example.com/encrypted-live-default` | legacy；无`package.yml`/`api.yml` | HTTP 21，均为`handler`；另有顶层`http.guard` | canonicalize package/service authoring；生成21个gateway entry、零operation contract |
| `runtime/encrypted-storage-live/mapped-service` / `example.com/encrypted-live-mapped` | legacy；无`package.yml`/`api.yml` | HTTP 13，均为`handler`；另有顶层`http.guard` | 同上；保留package collection mapping事实，但移出旧式`service.yml` owner |
| `runtime/live-tests` / `skiff.run/runtime-live` | legacy；无`package.yml`/`api.yml` | HTTP 6，均为`handler` | canonicalize；5个raw unary entry、1个raw stream entry；contract为0 |
| `test-runner/fixtures/package-service-host/provider` / `example.com/payments` | normal；API `echo` | 无 | `1 -> 1`；`payments/echo`是真实service dependency call，不迁移 |
| `test-runner/fixtures/package-service-host/consumer` / `example.com/consumer` | normal；API `owner`、`run` | 无 | `2 -> 2`；不因“当前无外部route”清理API |
| `test-runner/fixtures/package-service-host/consumer-tests` / `test.skiff/package-service-host-consumer-tests` | `kind: test`；API `{}` | 无 | service manifest不变；其逐test-case合成ingress由test-runner共享lane迁移 |
| `test-runner/fixtures/alias-return-catch-once-tests` / `test.skiff/alias-return-catch-once-tests` | `kind: test`；API `{}` | 无 | 同上 |
| `test-services/std` / `test.skiff/std-tests` | `kind: test`；API `{}` | 无 | 同上 |

`runtime/live-tests`的`runtimeKit.packageEcho`是唯一一个dependency-qualified旧`handler`。目标设计要求
gateway handler解析到当前implementation package的精确callable，因此该route需要一个本地wrapper，不能把
dependency display path直接带入`GatewayEntryIdentity`。

### 2.2 skiff-packages：7个service root

| root / service id | kind / API | 当前ingress | 预期动作 |
| --- | --- | --- | --- |
| `registry` / `skiff.run/registry` | normal；20个registry operation | 无 | `20 -> 20`；source无需external-ingress迁移 |
| `tests/aliyunoss` / `skiff.run/aliyunoss-tests` | `kind: test`；API `{}` | 无 | manifest不变 |
| `tests/http-session` / `skiff.run/http-session-tests` | `kind: test`；API `{}` | 无 | manifest不变 |
| `tests/openai-live` / `skiff.run/openai-live-tests` | `kind: test`；API `{}` | 无 | manifest不变 |
| `tests/openai` / `skiff.run/openai-tests` | `kind: test`；API `{}` | 无 | manifest不变 |
| `tests/registry` / `skiff.run/registry-tests` | `kind: test`；API `{}` | 无 | manifest不变 |
| `tests/track` / `skiff.run/track-tests` | `kind: test`；API `{}` | 无 | manifest不变 |

`aliyunoss`、`http-session`、`openai`和`track`自身没有`service.yml`，所以没有ServiceContract operation
delta。Account继续package-call `http-session`；Agine继续package-call `http-session`、`track`，这些都不是
external ingress或service dependency operation。

Registry的20个operation按四类record各五个：

- `packageArtifact{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `serviceContract{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `serviceDeployment{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `runtimeAssembly{Put,Read,PointerRead,PointerCas,PointerHistory}`。

三个仓库没有指向`skiff.run/registry`的`package.yml.services` edge，但这不构成删除依据：Registry没有
ingress，20个function就是其有意的ordinary service-call contract。`registry/model.skiff`只镜像artifact
identity/ref摘要，不镜像operation或ingress字段；gateway entry schema变化不要求修改这份source model，
但新artifact schema/identity发布后仍需重跑Registry存取验证。

### 2.3 Internals：4个真实service root

| root / service id | HTTP / WS routes | 当前selector | 当前contract | external-only移除后 |
| --- | ---: | --- | ---: | ---: |
| `skiff-platform/account` / `skiff.run/account` | 21 / 0 | 全部`operation` | 21 | 0 |
| `codex-relay/service` / `agine.ai/codex-relay` | 30 / 0 | 全部`operation` | 17 | 2 |
| `aihub/service` / `agine.ai/aihub` | 7 / 1 | 全部`operation` | 8 | 6 |
| `agine/service` / `agine.ai/api` | 14 / 1 | 全部`operation` | 2 | 0 |

四个service合计72条HTTP route和2条WebSocket route。HTTP route中69条是raw unary，Relay的3条
`/v1/*` route共享一个raw server-stream handler。两个WebSocket route都用一个统一handler处理
connect/receive phase。

## 3. `api.yml`逐service分类

这里的“保留”只表示不能作为external-ingress机械清理的一部分删除；没有当前consumer不等于API无效。
额外API收缩必须另开语义任务。

### 3.1 Account

当前receipt冻结的21个Available operation全部只因HTTP route公开：

`account.addMember`、`account.approveCliAuthorization`、`account.checkAuthority`、
`account.createCliToken`、`account.createDomainChallenge`、`account.createOrganization`、
`account.getOrganization`、`account.listCliTokens`、`account.listMembers`、
`account.listOrganizations`、`account.login`、`account.logout`、`account.me`、`account.ping`、
`account.pollCliAuthorizationToken`、`account.register`、`account.revokeCliToken`、
`account.revokeMember`、`account.startCliAuthorization`、`account.verifyDomainChallenge`、
`account.verifySession`。

- 三仓库没有`services: skiff.run/account`，也没有service-call source consumer。
- 21个function均应从service-call projection移除，但保留为service package内部普通handler。
- `AccountService`和`accountService` public-instance声明没有出现在当前21-operation receipt中，不是第22个
  contract operation；也没有跨仓库consumer。是否保留其package-public形状不属于route迁移。
- Skiff Platform的真实external consumer是
  `skiff-platform/client/src/lib/{accountClient.ts,accountServer.ts}`及
  `pages/api/account/[...path].ts`，它们仍按相同HTTP selector调用。

### 3.2 Codex Relay

当前17个operation分为：

| 分类 | public paths | 动作 |
| --- | --- | --- |
| external-only | `adminSession`、`adminLogin`、`adminLogout`、`adminState`、`adminRelayApiKeyCreate`、`adminRelayApiKeyRevoke`、`adminUpstreamSourceCreate`、`adminUpstreamSourceEnable`、`adminUpstreamSourceDisable`、`adminLlmInteractionsList`、`adminLlmInteractionGet`、`adminChatgptOauthStart`、`adminChatgptOauthSession`、`adminOptions`、`v1Proxy` | 从`api.yml`/ServiceContract移除，改为gateway handler |
| 已有真实service dependency call | `relayProxy.responsesCompletedResult` | 必须保留；`aihub/service/internal/aihub_service.skiff`调用`codexRelay/relayProxy.responsesCompletedResult(request)` |
| 非ingress、当前无静态consumer | `relayProxy.responsesCompleted` | 保留；删除它是另一个service API设计决定 |

30条route只对应15个external-only callable：14条primary admin route中
`adminChatgptOauthSession`覆盖GET/DELETE，13条OPTIONS route共享`adminOptions`，3条`/v1` route共享
`v1Proxy`。目标contract因此是`17 -> 2`，不是`17 -> 0`。

真实external consumer包括`codex-relay/admin/admin-api.mjs`及其UI、OAuth管理流程，以及
`codex-relay/scripts/verify-relay-key.mjs`对`/v1/models`的调用；OpenAI-compatible `/v1` caller不应被
改写成Skiff service call。

### 3.3 AIHub

当前8个operation分为：

| 分类 | public paths | 动作 |
| --- | --- | --- |
| external-only | `handleAihubHttp`、`websocket` | 从ServiceContract移除并成为HTTP/WS gateway handler |
| 已有真实service dependency call | `managedLlm.streamChat`、`managedLlm.webSearch`、`providerCatalog.builtinProvider` | 必须保留；三处caller均在`agine/service` |
| 非ingress、当前无静态service consumer | `managedLlm.validateChat`、`providerCatalog.model`、`selectProvider` | 本次保留；不能随route机械清理 |

对应真实caller：

- `agine/service/internal/agent_bridge_llm_adapter.skiff`调用
  `aihub/managedLlm.streamChat(input)`和`aihub/managedLlm.webSearch(input)`；
- `agine/service/internal/provider_runtime.skiff`调用
  `aihub/providerCatalog.builtinProvider()`。

目标contract为`8 -> 6`。`AihubSocketContext`只被AIHub WebSocket handler使用且没有package/service
consumer；新gateway codec从linked signature取得该类型后，可移除这个只为旧ingress contract形成的
`api.yml` export。若另有明确package-public理由则可以显式保留，但不得再因gateway entry进入
ServiceContract schema closure。AIHub的真实external consumer是`aihub/client/app.js`：HTTP读取
`/v1/providers`，WebSocket连接`/ws`并发送/接收chat stream。

### 3.4 Agine

`handleAgineHttp`和`websocket`两个operation均为external-only；三仓库没有
`services: agine.ai/api`或service-call source consumer，目标contract为`2 -> 0`。

`ConnectionContext`虽在Agine package内部被大量模块使用，但跨仓库没有consumer；它当前进入`api.yml`是
WebSocket contract-owned Context的副作用。新gateway entry应从linked handler signature得到Context codec；
这个export可移除，或在有独立package-public理由时保留，但不能再成为service-call schema root。

真实consumer保持external：

- `agine/client/src/lib/http.ts`调用14条HTTP selector中的业务子集；
- `agine/client/src/lib/ws.ts`连接`/ws`；
- `agine/host/src/GatewayClient.ts`也连接同一`/ws`，并携带host metadata；
- `agine/client/e2e/api.chat-smoke.mjs`及`system.two-hosts.e2e.ts`同时覆盖HTTP、WS connect和receive。

### 3.5 Skiff fixture service API

- `example.com/websocket_fixture/ping`仅供HTTP `/ping` artifact compatibility fixture使用，无service
  dependency consumer，目标`1 -> 0`。
- `example.com/payments/echo`被
  `test-runner/fixtures/package-service-host/consumer/main.skiff`两处调用，并由consumer-tests验证，保持
  `1 -> 1`。
- `example.com/consumer/{owner,run}`没有ingress，保持`2 -> 2`；consumer-tests以top-level package
  dependency测试它们，不应在本任务清理。

## 4. `service.yml`旧shape的精确范围

### 4.1 Ingress-bearing manifest

| manifest | HTTP route | WS route | callable shape | 其它ingress字段 |
| --- | ---: | ---: | --- | --- |
| compiler fixture | 1 | 0 | `operation` 1 | route `host` 1 |
| encrypted default | 21 | 0 | `handler` 21 | `http.guard` 1 |
| encrypted mapped | 13 | 0 | `handler` 13 | `http.guard` 1 |
| runtime live | 6 | 0 | `handler` 6 | 无 |
| Account | 21 | 0 | `operation` 21 | 无 |
| Relay | 30 | 0 | `operation` 30 | 无；另有非ingress `timeout` |
| AIHub | 7 | 1 | `operation` 8 | 无；另有非ingress `timeout` |
| Agine | 14 | 1 | `operation` 15 | 无；另有非ingress `timeout` |
| **合计** | **113** | **2** | **`operation` 75；`handler` 40** | **`http.guard` 2；route `host` 1** |

三个legacy live manifest还把非ingress package owner事实留在`service.yml`：

- encrypted default有`version`；
- encrypted mapped有`version + packages`，并在package dependency上声明collection mapping；
- runtime live有`version + packages`。

这些字段应随canonicalization回到`package.yml`。Internals的Relay、AIHub、Agine各有一个合法的非callable
`timeout`区块；Account没有。它们不计入75/40 callable selector统计，也不应被误删。

逐个解析全部20个tracked manifest后，没有发现：

- `pre`、route-level `guard`、`adapterArgs`；
- typed/raw显式mode、request/response adapter source；
- WebSocket显式`connect`或`receive` selector；
- 同一route同时含`operation`和`handler`；
- HTTP/WebSocket之外的external protocol authoring。

当前两个WebSocket manifest都是：

```text
websocket.routes[].path + operation
```

AIHub和Agine source各用一个
`WebSocketIngressEvent<Context> -> WebSocketConnectResult<Context>?`函数，按`event.tag`在函数内区分
`connect`与`receive`。迁移不能错误拆成两个service-call operation；gateway entry应冻结同一linked
callable、两个phase的adapter plan和Context expectation。

### 4.2 不在 `service.yml` 搜索中的旧绑定

`test-runner/src/ecosystem_smoke_fixture.rs`手工构造
`test.skiff/ecosystem-smoke`：

- HTTP `POST /probe`把public `marker`编译成contract operation；
- 可选WebSocket `/socket`把public `websocket`编译成contract operation；
- `ServiceDeploymentOperationInput`和`DeploymentIngressBinding`都保存
  `ContractOperationId`。

它被四个normal-source fixture使用：

- `package-service-websocket-smoke`；
- `package-service-websocket-generation-a`；
- `package-service-websocket-generation-b`；
- `package-service-i02-spawn-submit`。

每次fixture当前合成2-operation contract，两个callable都只服务external smoke；目标是`2 -> 0`并改由
gateway entry定位它们。I02的public path `marker`实际映射到`main.submitSpawnReceipt`；其它三个映射
`main.marker`。四个fixture的`api.yml`是否还为package-test linkage保留`marker`，不能决定ingress是否
进入ServiceContract；两个surface必须显式分开。

`test-runner/src/package_test_assembly.rs`对每个test case另合成一个`run` operation和
`POST /__skiff/package-test/{index}` ingress。该HTTP selector只是test harness执行入口，不是test
service-call API；目标是每case `1 -> 0`，但仍需保留test callable的package-linked执行和
`--base-assembly` service dependency binding。这个改动会使所有F269 test execution evidence失效。

compiler、deployment、runtime、Router的多处unit fixture也手工构造
`DeploymentIngressBinding { contract_operation_id }`。它们是共享model的机械consumer，不是额外
service目录。

## 5. HTTP / WebSocket consumer与证据矩阵

| 模式 | 真实service / consumer | 当前测试与workflow | 审计结论 |
| --- | --- | --- | --- |
| HTTP raw unary | Account 21；Relay admin/OPTIONS 27；AIHub 7；Agine 14。external consumer分别为Skiff Platform client、Relay admin/verify script、AIHub browser、Agine browser。旧live另有39条raw unary handler route | 各service source test及receipt；F269已迁出的top-level package tests；`router/tests/runtime-assembly-unary-dispatch.test.ts`；legacy `raw-http.test.ts` | 真实生产主路径，共69条Internals route。迁移需保持method/path/host/header/body/cookie与fixed-error行为 |
| HTTP typed unary | compiler `/ping` fixture的source是`() -> string`，但只验证compile/load；没有真实client。三仓库没有携带typed adapter metadata的真实service manifest | `router/tests/manifest-validation.test.ts`与`raw-http.test.ts`使用合成direct manifest；runtime `http_adapter`/linked-type-plan unit tests | 没有端到端真实service证据。`runtime/live-tests`的`typedJsonEcho(HttpRequest) -> HttpResponse`手工调用`std.http.decodeJson`，仍是raw，不能冒充typed覆盖 |
| HTTP raw stream | Relay `v1Proxy(HttpRequest) -> Stream<HttpResponseStreamEvent>`覆盖`/v1/responses`、`/v1/responses/compact`、`/v1/models`；verify-relay-key真实调用models。旧live `streamEcho`有1条route | `router/tests/assembly-http-gateway-stream.test.ts`和legacy `raw-http.test.ts`；Relay source tests；runtime live test目前只直接调用`streamEcho` | Router有合成stream证据，但canonical request/host仍只接受unary；Relay是必须闭合的真实blocker |
| WS connect | AIHub `/ws`、Agine `/ws`；AIHub browser、Agine browser和Agine Host为真实client | compiler `websocket_ingress.rs`；runtime request/eval WS tests；Router `assembly-websocket-gateway.test.ts`、`host-ingress.test.ts`；ecosystem/generation/I02 smoke；Agine chat/two-hosts E2E | handler和Context codec当前来自ServiceContract operation；移除API前必须改为gateway-entry-owned plan |
| WS receive | 与connect相同；两个production handler均按`event.tag == "receive"`读取pinned connection Context | 同一组runtime/Router/smoke tests；Agine chat smoke同时发收；AIHub client实际发chat message并消费stream | connection必须在connect时pin exact `GatewayEntryIdentity`/generation，receive不能重新按display service或新deployment猜handler |

补充事实：

- `runtime/live-tests/internal/http_adapter.live.test.skiff`只有`rawEcho`与dependency
  `packageEcho`经过Router round trip；typed-json、binary、guard、stream case都直接调用source function。
- 当前runtime-live canonical plan会因这些legacy root缺少`package.yml`而fail closed，错误明确要求
  “terminal canonical-harness migration”；旧成功记录不能作为新gateway证据。
- encrypted storage live的真实owner为
  `scripts/lib/encrypted-storage-live-harness.mjs` /
  `scripts/check-db-encrypted-storage-live.mjs`，34条route均为raw unary。
- AIHub client当前没有与真实service组合的checked-in E2E owner；其`server.test.mjs`只测静态server。不能把
  静态server test计为WebSocket ingress验收。

## 6. ServiceContract delta与三仓库影响

| service / synthetic owner | 当前operation | 目标operation | identity / downstream影响 |
| --- | ---: | ---: | --- |
| Registry | 20 | 20 | ServiceProtocolIdentity语义不变；artifact schema checkpoint后重发/重验store record |
| Account | 21 | 0 | 新零operation identity；独立于其它Internals service |
| Relay | 17 | 2 | ServiceProtocolIdentity变化；AIHub的service requirement必须对新contract重编译 |
| AIHub | 8 | 6 | 自身identity变化，且package先吸收Relay新identity；Agine随后重编译 |
| Agine | 2 | 0 | 新零operation identity；需先吸收AIHub新identity |
| compiler fixture | 1 | 0 | compiler-generated deployment/assembly/Router loader golden变化 |
| ecosystem smoke synthetic service | 2 | 0 | entrypoint receipt从operation id改为gateway entry identity |
| package-test synthetic service | 每case 1 | 每case 0 | 全部package/test-service assembly与execution receipt变化 |
| payments fixture | 1 | 1 | 无变化 |
| consumer fixture | 2 | 2 | 无变化 |
| legacy runtime/encrypted services | N/A | 0 | 首次进入canonical package/service projection；只生成gateway entries |

因此Internals source目录可以并行编辑，但canonical authoring/发布验收顺序必须为：

```text
Relay(2) ──► rebuild AIHub against Relay ──► AIHub(6)
                                         └──► rebuild Agine against AIHub ──► Agine(0)

Account(0) ─────────────────────────────────────────────────────────────── independent
```

不能通过保留15个Relay、2个AIHub或2个Agine假operation来维持旧identity；设计明确规定external ingress
变化不进入`ServiceProtocolIdentity`。同样不能让Router按旧`ContractOperationId`兼容猜测。

## 7. Shared checkpoint

以下缺口被所有operation-form service、legacy fixture和F269共享，必须由一个串行owner先闭合：

1. **Artifact/identity owner**
   - `artifact-model/src/deployment.rs::DeploymentIngressBinding`只有
     `selector + contract_operation_id`；
   - `artifact-model/src/runtime_assembly.rs::GlobalIngressBinding`仍携带contract ref和operation id；
   - 尚无architecture要求的`GatewayEntryIdentity`、typed gateway entry、handler/pre/guard
     `PackageCallableId`或adapter plan；
   - schema version、canonical identity hashing、store path/strict serde/TS mirror必须原子更新。
2. **Compiler/deployment owner**
   - `compiler/driver/generated_deployment.rs::RouteAuthoring`强制`operation: String`；
   - `resolve_route`只接受`ServiceApiProjection.available`中的contract operation；
   - 尚不能从`service.yml`解析非public handler/pre/guard source selector并解析到linked callable；
   - `compiler/contract/src/compile.rs`对空operations返回`EmptyOperations`，直接阻塞Account、Agine和
     external-only fixture；
   - deployment validation、revision和diagnostic receipt需分别验证operation bindings与gateway entries。
3. **Runtime request/host owner**
   - `runtime/host/src/host/request_entry/assembly_wire.rs`用
     `routing.contract_operation_id`同时校验route、descriptor和target；
   - `runtime/request/src/assembly_ingress.rs`要求`mode == "unary"`并拒绝`http_adapter`，阻塞真实Relay
     stream和typed ingress；
   - activation lookup和execution target必须从admitted gateway entry取得exact callable，不能回查
     ServiceContract display path。
4. **WebSocket codec owner**
   - `runtime/eval/src/assembly_execution/websocket_contract_plan.rs`的
     `PinnedWebSocketContractPlan`从ServiceContract operation取得Event/Result/Context schema；
   - 新plan必须由gateway entry linked signature/adapter plan拥有，并保持connect pin、receive generation、
     nominal zero-byte Context和fixed error语义。
5. **Transport/Router owner**
   - RuntimeAssembly request routing和Router snapshot/gateway当前镜像contract operation id；
   - raw unary、typed unary、raw server-stream、WS connect/receive必须共用同一gateway identity事实；
   - Router可以转发opaque payload，但不能成为业务codec或按source/display name猜target。
6. **Test-runner owner**
   - `ecosystem_smoke_fixture.rs`和`package_test_assembly.rs`必须消费新typed gateway projection；
   - public package-test callable仍可用于package linkage，但test ingress不再制造service operation；
   - fixture entrypoint/receipt API必须以gateway entry identity替代operation id。

该checkpoint应等待F347/F348/F349/F350四份审计合流后冻结唯一model；service agent不得分别发明
`handler`语法、gateway identity preimage、adapter source或零operation special case。

## 8. 可并行迁移批次

```text
C0 shared gateway model/compiler/runtime/router/test-runner API
 │
 ├─ S1 Account service dir (21 -> 0)
 ├─ S2 Relay service dir (17 -> 2)
 ├─ S3 AIHub service dir (8 -> 6)
 ├─ S4 Agine service dir (2 -> 0)
 ├─ S5 compiler fixture + compiler→Router compatibility fixture
 ├─ S6 legacy runtime-live/encrypted-storage service authoring
 └─ S7 test-runner synthetic smoke/package-test authoring
       │
       └─ F269 owner rebase/receipt refresh/test evidence rerun
```

并行边界：

- S1–S4可以按service目录并行写source/manifest/receipt validator；但S2→S3→S4的contract
  publication/assembly acceptance必须按依赖顺序执行。
- S5只拥有compiler fixture及其Router generated-artifact assertion，不与S1–S4混写。
- S6由一个owner统一管理三个legacy root及
  `verify-live-plan`/encrypted harness相邻改动；避免两个live agent同时改共享workflow。
- S7由一个owner同时改两个test-runner Rust synthesis owner和它们的entrypoint receipt，避免
  ecosystem smoke与package test各自发明不同gateway fixture。
- skiff-packages无source migration lane；Registry和官方package只进入revalidation lane。
- F269 worktree、`scripts/prepare-canonical-assembly.mjs`、`scripts/isolated-service-graph.mjs`及迁出的
  test roots只由F269 owner处理；S1–S4不得把co-located旧test重新加回production service。

## 9. F269保存与证据失效

本次只读检查的F269状态：

| 项 | 精确值 |
| --- | --- |
| worktree / branch | `/Users/geek/workspace/internals-p5-f269` / `codex/p5-f269-internals-test-service-migration` |
| HEAD / tree | `14ccfd417c9f45f00bd77015494cdd727e0f88dc` / `2327a766bcc6f32e7470b57420659fc991ef8a15` |
| tracked WIP | 103个status entry；`407 insertions, 34619 deletions` |
| untracked WIP | 123个文件，折叠为8个status directory entry |
| tracked binary diff SHA-256 | `7af6187c3a20dd324bc0fe86e7d93fb5a28491115456734c8ec699a2971283cb` |
| sorted untracked-content inventory SHA-256 | `c243cfe45589c9a7e736191255b7918e8e3238523cb3206bd0b8dd683407742f` |

该worktree尚无F269 commit/result；本审计未修改、stash、index、checkout、build或清理它。

应完整保存的迁移结果：

- `agine/service-tests`；
- `aihub/service-tests`与独立`aihub/live-tests`；
- `codex-relay/tests/{default,admin-remote-invalid,admin-remote-password}`；
- `packages/{agent-tests,llm-api-tests,llm-providers-tests}`；
- `skiff-platform/account-tests`。

这些新root的`service.yml`均只有`id + kind: test`且无ingress，目标shape正确。测试通过
`packages[].access: topLevel`调用production package source，例如`subject/account.*`、
`subject/internal.aihub_service.*`和`subject/proxy_runtime.*`；它们不依赖被移除的service-call
operation，因此不应因API清理被删除或改写回co-located test。

F269当前workflow已在准备：

1. 发布production package dependencies和四个production service；
2. 生成base RuntimeAssembly；
3. 对test service执行`skiff test --base-assembly ... --deny-skips --require-tests`。

但它仍是未提交WIP，且`liveTestServices`中的AIHub live root没有进入普通test执行循环，不能把当前source
布局当作已通过证据。

共享checkpoint后必然失效的证据：

- Account/Relay/AIHub/Agine的`service-api-receipt*.mjs`仍冻结`21/17/8/2`和route
  `operation` shape；
- `agine/service/test-isolated-service-receipt.mjs`仍调用旧Agine receipt validator；
- 四个ServiceProtocolIdentity、deployment revision、assembly identity及artifact record path/receipt；
- package-test每case的synthetic `run` operation、`ContractOperationId`和entrypoint receipt；
- 所有依赖旧base assembly或旧gateway/operation wire identity的F269 test execution输出。

保存顺序：

1. 由F269 owner先把上述精确WIP形成可恢复commit/checkpoint；
2. C0与S1–S7合入integration后，由同一owner rebase/merge到F269 worktree；
3. 只更新旧receipt/route/gateway identity断言及必要的test-runner entrypoint消费；保留迁出的test source、
   package agent API/source修复和test doubles；
4. 重建新base assembly并以strict test-service workflow重跑全部非live test；
5. AIHub live test和Agine端到端smoke在各自显式live阶段重跑，不能拿旧output补证据。

## 10. 后续验证入口（本审计未运行）

### 10.1 Shared Skiff checkpoint

先运行focused static/unit：

```bash
cd /Users/geek/workspace/skiff-phase-05-integration
cargo test -p skiff-artifact-model
cargo test -p skiff-deployment
cargo test -p skiff-compiler --test generated_service_deployment
cargo test -p skiff-compiler --test websocket_ingress
cargo test -p skiff-runtime-request
cargo test -p skiff-runtime-eval
cargo test -p skiff-runtime-host
cargo test -p skiff-test-runner --test package_service_contract_deployment
pnpm --filter @skiff/router exec vitest run \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts \
  tests/host-ingress.test.ts
```

静态反搜至少要证明：

- generated deployment/RuntimeAssembly production DTO不再把ingress绑到`contractOperationId`；
- migrated `service.yml` route不再使用`operation`；
- external-only handler/context不再仅为ingress留在`api.yml`；
- ServiceContract operation bindings仍精确覆盖所有Available ordinary service-call function；
- zero-operation contract、gateway-entry identity drift、unknown/extra adapter source均fail closed。

### 10.2 skiff-packages

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
npm run test:registry
npm test
```

Registry验收应继续精确为20个operation，同时证明新版ServiceContract/Deployment/Assembly identity摘要可
immutable put/read/pointer操作；其它official package不应出现意外ServiceContract。

### 10.3 Internals与F269

先运行纯receipt/shape test，再由F269 owner在其迁移合流后的worktree运行canonical test-service workflow：

```bash
cd /Users/geek/workspace/internals-p5-f269
node --test skiff-platform/account/service-api-receipt.test.mjs
node --test codex-relay/service/service-api-receipt.test.mjs
node --test aihub/service/service-api-receipt.test.mjs
node --test agine/service/service-api-receipt.test.mjs

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/codex-relay

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/aihub

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/api

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs skiff.run/account
```

当前F269实现中每个入口都会准备完整fixture并执行全部普通test service；integration owner可在合流时去重
实际运行，但不能只看生成JSON或零test summary。

### 10.4 External protocol与live终验

只有C0、真实service和F269 isolated tests全部通过后，才进入显式live阶段：

```bash
cd /Users/geek/workspace/skiff-phase-05-integration
node scripts/verify.mjs --only db-encrypted-storage-live
node scripts/run-package-service-ecosystem-smoke.mjs
node scripts/run-package-service-generation-lifecycle-smoke.mjs

cd /Users/geek/workspace/internals-phase-05-integration/agine
npm run e2e:chat-smoke
```

还需由runtime-live owner补齐合法的canonical显式inputs后运行`runtime-live` selector；在三个legacy root
完成package/service迁移前，当前fail-closed结果是precondition，不是测试失败。Agine two-hosts、AIHub
真实client WebSocket及Relay `/v1` stream应分别补/跑实际network probe；typed HTTP因为没有真实service
consumer，至少要增加一个compiler→Router→runtime的normal-source end-to-end fixture，不能只保留direct
manifest unit test。

本审计没有运行以上任何命令，也没有读取或写入stable instance、stable artifact root或live service。
