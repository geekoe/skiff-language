# P5-F424A Skiff connect-only and outbound owner audit result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本审计确认父节点冻结的单 entry、connect-only、HTTP 上行 / WebSocket 下行方案可在现有
authoring、gateway entry、ServiceDeployment、RuntimeAssembly、activation、runtime transport 和
Router lifecycle 边界内完成。无需支持多个 entry、无需让 author 选择字符串 entry id、无需恢复
receive/message 业务上行，也无需增加新的公共语义。

## 1. 精确输入、边界与结论

| 项 | commit | tree |
| --- | --- | --- |
| 父节点指定 production 输入 | `ba74febaca5dbe8f2b55d6db04e0544a6758bf4b` | `7ac91495f85bbf997fe4f57ddfbec76b82cc753c` |
| audit checkout 起点 | `4b77ed1f6bfeb3deb7f3364981cf06de6e47a522` | `ec8cd55a377e01b37a107301b3707322c64b7921` |

audit checkout 相对 production 输入只多四份 F424 规划文档，没有 production、test 或 fixture
差异。启动时分支为 `codex/p5-f424a-connect-audit`，worktree clean。

审计严格只读 production、test、fixture和设计；没有访问 stable/live、没有启动 instance、没有
merge/rebase/push。唯一写入是本 result 文档。

核心结论：

1. `service.websocket` 当前由 compiler driver 显式 fail-closed；authoring DTO 仍是无类型 JSON，
   所以不能直接解除拒绝。最小修复是把它改成一个严格的可选对象，而不是 map/list，从 schema
   层机械保证每个 service 最多一个 entry。
2. `PackageArtifact` 已有解析 private source callable 所需的完整 typed callable evidence，但它
   故意不拥有 service ingress。connect entry 应由 compiler 用这些 evidence 投影进
   `ServiceDeployment.gateway_entries + ingress`，再进入 `RuntimeAssembly.gateway_ingress`；不能把
   ingress 塞回 `ServiceContract` 或 resurrect F420B 删除的 operation-based Assembly gateway。
3. 当前 std/runtime 的 WebSocket 模型仍以泛型 Context、connect/receive union 和 client message
   为中心，需要整体切除。四个 outbound native 已经是非挂起、无 public entry-id 参数的正确外形。
4. outbound frame 链已经贯通，但普通 HTTP/service/actor activation 没有保留唯一 WebSocket entry，
   因而 native capability 取得的是 `None`。缺口在 assembly admission 到 `ActivationContext`
   构造之间，不在 native 签名或 Router frame。
5. Router connection/policy/fan-out/generation lifecycle 可复用；receive queue/dispatch 必须删除。
   client text/binary data 的最窄 `1003` owner 是 gateway 的 `message` callback，关闭必须发生在任何
   receive scheduling/dispatch 之前。
6. shared schema/compiler checkpoint 与 Router/runtime consumer 不能同时启动。还需要一个 current
   connect-only wire checkpoint；只有它提交后，Router 与 runtime consumer 才能在互斥 write set
   中并行。

## 2. 冻结语义 checkpoint

后续实现必须同时保持以下闭集：

- 每个 service 是零或一个 WebSocket entry；
- 外部 client 只产生 upgrade/connect，text/binary data 一律 close `1003`，零 runtime dispatch；
- ping/pong/close 由协议层处理；
- connect handler 可省略；省略时 Router synthesized accept，零 runtime dispatch；
- connect callable 只接收 `websocket.connectRequest`、`websocket.connectionId`；
- connect result 只有 accept/reject、可选 `businessIdentity` 和可选 policy，不含 Context；
- 四个 send native 不接收 entry id，从当前 activation 的唯一 entry 解析；
- `(serviceId, websocketEntryId, businessIdentity)` 是 business fan-out key，忽略 version/build；
- direct connection send 保持低层能力，但也必须验证 service/entry/generation owner；
- WebSocket external ingress 不是 service operation，不产生 `ContractOperationId`；
- author 不提供 entry id。wire 上的 `websocketEntryId` 是平台生成、校验的内部 metadata，不是
  authoring 字段或 native 参数。

因此本审计发现的“唯一内部 `WebSocketEntryId` owner”和“handler 可省略”是父设计的必要实现
checkpoint，不是 scope expansion。

## 3. Current authoring、拒绝点与最小 target shape

### 3.1 当前为何拒绝

| owner | 当前事实 |
| --- | --- |
| `artifact-model/src/ecosystem_authoring.rs` | `ServiceManifestAuthoring.websocket` 是 `Option<serde_json::Value>`；只保证顶层 unknown-field strict，不能验证 WebSocket 内部 shape。HTTP 已经是 typed map，并有 duplicate-key-preserving deserializer。 |
| `compiler/input/src/service_config.rs` | `read_service_manifest` 验证 service calls 和 HTTP；没有 WebSocket parser/semantic validation。 |
| `compiler/driver/generated_deployment.rs` | `generate_service_deployment` 首先调用 `reject_unwired_websocket_authoring`；只要字段存在就返回 `InvalidManifest{field:"websocket", message:"WebSocket business gateway entries are not defined for this deployment generation; refusing legacy operation ingress"}`。 |
| `compiler/tests/generated_service_deployment.rs` | `generated_service_deployment_rejects_legacy_websocket_operation_ingress` 冻结当前 fail-closed 行为。 |

拒绝是刻意阻止旧 `websocket.routes[].operation` 再次进入 deployment，不是 parser 偶然缺功能。

### 3.2 最小 current DTO

`ServiceManifestAuthoring.websocket` 应变为
`Option<WebSocketGatewayEntryAuthoring>`，而不是
`BTreeMap`、`Vec` 或保留 arbitrary JSON。该对象至少包含：

- strict `deny_unknown_fields`；
- `host`，默认 `"*"`；
- 必填 `path`；
- 可选 connect target；
- connect target 的精确 source callable selector 与 adapter args；
- adapter source 仅允许 `websocket.connectRequest`、`websocket.connectionId`。

不得包含：

- author-chosen `id`；
- `routes`；
- receive/message selector；
- Context codec；
- message/connection envelope；
- uplink mode；
- 多 entry 容器。

compiler 为这个唯一对象分配 deployment-local、compiler-owned `GatewayEntryKey`。它不是 public
authoring，也不作为 native 参数暴露。`compiler/input/src/service_config.rs` 负责 host/path、target、
source selector 和 duplicate/unknown-field 检查；driver 只消费已经验证的 typed DTO。

### 3.3 现有 schema 的一个必要改动

`artifact-model/src/deployment.rs` 的 `DeploymentGatewayEntry.handler` 当前是必填。父设计允许没有
connect handler，所以 shared checkpoint 必须把 target 表示成协议可区分的闭集，或将 handler 改为
可选并增加严格 invariant：

- HTTP entry 必须有 handler；
- WebSocket connect entry 可以没有 handler；
- 无 handler时 adapter args 必须为空，Router synthesized accept；
- 其它协议/组合一律拒绝。

不能用 dummy callable 或合成 service operation 填这个空位。

## 4. Source callable 到 Router 的完整记录链

### 4.1 PackageArtifact 只提供 typed callable evidence

`artifact-model/src/package_artifact.rs` 已经保存：

- `package_local_abi.implementation_symbols`；
- `PackageCallableSignature`；
- `callable_links`；
- `callable_semantic_facts`。

`compiler/driver/http_gateway_projection/{mod.rs,resolver.rs,schema.rs}` 已实现可复用的 exact resolver：

```text
source symbol selector
  -> implementation symbol
  -> PackageCallableId + PackageCallableSignature
  -> callable link exact target/file
  -> callable semantic facts
```

WebSocket projector 应镜像或抽取这段 resolver，验证：

- target 是本 package 的精确 private callable；
- callable 非 generic；
- 参数数量、顺序和 allowed source 一致；
- 返回值是非 generic、non-null 的 `WebSocketConnectResult`；
- 不接受 HTTP source 或 receive/context source。

`PackageArtifact` 不应新增 service ingress record。它拥有的是 compiler 投影 ingress 所需的 typed
callable evidence；service-specific selector、entry、policy 属于 `ServiceDeployment`。

### 4.2 Current artifact/deployment/assembly chain

目标链应为：

```text
service.yml typed singleton websocket authoring
        +
PackageArtifact exact callable evidence
        |
compiler WebSocket gateway projector
        |
DeploymentGatewayEntry
  - GatewayEntryKey (compiler-owned)
  - GatewayEntryIdentity
  - WebSocket protocol surface
  - optional connect handler
  - connect-only adapter plan
        |
DeploymentIngressBinding
  IngressSelector{protocol:websocket, host, method:None, path}
      -> GatewayEntryKey
        |
ServiceDeployment
        |
deployment projection / RuntimeAssembly.gateway_ingress
        |
runtime loader exact graph validation
        |
runtime linker exact callable target
        |
Router RuntimeAssembly v2 snapshot + exact ServiceDeployment snapshot
        |
current assembly connect-only gateway
```

现有可用类型：

- `artifact-model/src/deployment.rs`
  - `IngressProtocol::{Http, WebSocket}`；
  - `IngressSelector { protocol, host, method: Option<_>, path }`；
  - `DeploymentIngressBinding { selector, gateway_entry_key }`；
  - `DeploymentGatewayEntry { identity, surface, handler, pre, guard, adapter_plan }`。
- `artifact-model/src/runtime_assembly.rs`
  - operation-free 的
    `GatewayIngressBinding { selector, deployment, gateway_entry_key, gateway_entry_identity }`。
- deployment projection、`runtime/loader/src/runtime_assembly/gateway_ingress.rs` 和
  `runtime/linker/src/assembly/gateway.rs` 已按 entry key/identity 做 exact join。

当前缺少：

- `GatewayProtocolSurface::WebSocketConnect`；
- `GatewayAdapterKind::WebSocketConnect`；
- source `websocket.connectRequest`、`websocket.connectionId`；
- optional-handler invariant；
- internal canonical `WebSocketEntryId` owner；
- Router/Rust v2 connect request wire；
- Router current assembly WebSocket consumer；
- Host connect-only execution target。

`runtime/host/src/loader/assembly_admission.rs` 当前明确拒绝非 HTTP surface；Router 的
`runtimeAssemblySnapshot.ts` 和 `runtimeAssemblyDeploymentSnapshot.ts` 也只接受 HTTP，且后者要求
handler。这些是 fail-closed consumer，不是 parallel schema owner。

### 4.3 不能复活的旧链

F420B commit
`7a6b9af64435704063b104022dd86889fa1ecae0`
已删除：

- `router/src/gateway/assemblyWebSocketGateway.ts`；
- server/export wiring；
- `CanonicalAssemblyWebSocketIngressIdentity`；
- operation-based identity helper；
- `assembly-websocket-gateway.test.ts`；
- `assembly-websocket-ingress-identity.test.ts`。

旧 gateway 以 `ContractOperationId` dispatch connect + receive，保存 Context，并把 client message
送入 runtime。它不能恢复或改名复用。可复用的是它依赖的通用 lifecycle/dispatcher/generation
基础设施，不是旧 assembly gateway 或 identity preimage。

同样必须退休：

- `artifact-model/src/service_unit.rs` 的 `GatewayWebSocket` /
  `GatewayWebSocketRoute`；
- `artifact-model/src/websocket_ingress.rs` 的
  `WEBSOCKET_INGRESS_OPERATION_NAME`、WebSocket Event/Result ServiceContract builtin 与
  operation validator；
- Router `skiff-runtime-manifest-v1` 中的 operation/routes/connect/receive/context WebSocket
  projection。

Router 的目标 manifest 是 current RuntimeAssembly v2 snapshot 加 exact deployment snapshot，
不是 `router/src/manifest/**` / `router/src/artifacts/**` 的 legacy manifest reader。

## 5. Std connect model 与 legacy production/test owners

### 5.1 当前 `std/websocket.skiff`

当前 public model 包含：

- `TextConnectionMessage`、`BinaryConnectionMessage`、`ConnectionMessage`；
- 缺少 `websocketEntryId` 和 `gatewayEntryIdentity` 的 `WebSocketConnectRequest`；
- generic `WebSocketConnection<Context>`；
- generic `WebSocketReceiveEvent<Context>`；
- generic connect/receive union `WebSocketIngressEvent<Context>`；
- `WebSocketCloseEvent`；
- `WebSocketConnectionPolicy`；
- generic `WebSocketConnectResult<Context>`，accept 强制携带 Context。

目标只保留：

- connect request，其中加入 platform-owned
  `websocketEntryId`、`gatewayEntryIdentity`；
- `WebSocketConnectionPolicy`；
- non-generic、无 Context 的 connect result；
- 四个 send native 和 JSON helpers。

message、receive、connection、Context、ingress union 与 user close event 都应删除。协议 close
不需要 user-visible business event。

### 5.2 Legacy owner set

| 层 | 必须删除或迁移的 owner |
| --- | --- |
| compiler lowering/projection | `compiler/lowering/src/type_lowering.rs`；`compiler/projection/src/package_artifact/{boundary/types.rs,schema.rs}`；`compiler/source/src/type_resolution_model.rs` |
| runtime boundary/type shape | `runtime/boundary/src/{type_descriptor.rs,websocket_shape_descriptor.rs}`；`runtime/linked-type-plan/src/websocket_shape.rs` 及 parity tests |
| runtime eval | `runtime/eval/src/assembly_execution/{websocket_contract_plan.rs,websocket_ingress.rs,websocket_response.rs,websocket_identity.rs}`；`runtime/eval/src/websocket_adapter.rs`；相关 invocation/http/request-boundary branches |
| runtime request | `runtime/request-contract/src/{envelope.rs,response_event.rs}`；`runtime/request/src/{assembly_ingress.rs,assembly_ingress/websocket_request.rs,eval_invocation_builder.rs,websocket_ingress.rs}` |
| runtime old transport | `runtime/transport/src/{protocol.rs,request_mapper.rs,response_mapper.rs,ingress_selector.rs}` 中 connect/receive/context branch |
| Router legacy | `router/src/manifest/{types.ts,loadManifest.ts,identity.ts}`；`router/src/artifacts/{manifestProjection.ts,serviceAssembly.ts,loadArtifactRoot.ts}`；`router/src/protocol/{envelope.ts,runtimeProtocol.ts}` 的旧 WS event；`router/src/gateway/webSocketGateway.ts` 的 receive business dispatch |
| compiler/runtime tests | WebSocket std/generic/projection tests；boundary/request/eval/transport parity、response、identity与receive tests |
| Router tests | manifest validation、protocol WebSocket response、gateway receive/context tests |
| fixtures/tooling | `test-runner/fixtures/package-service-{websocket-smoke,websocket-generation-a,websocket-generation-b,i02-spawn-submit}`；`test-runner/tests/package_service_contract_deployment.rs`；`scripts/tests/package-service-i02-combined.test.mjs`；cross-system WS wire fixtures |

四个 package-service fixture 的 `api.yml`/source 仍公开
`WebSocketIngressEvent<null>` 并实现 receive branch。它们应改成普通 private connect callable，
由 current `service.yml` 引用；receive/downlink business branch 必须删除，而不是翻译成另一种
operation。

## 6. Outbound native、frame 链与 Activation 缺口

### 6.1 四个实际签名与 effect summary

`std/websocket.skiff` 与 `artifact-model/src/native_signature.rs` 当前一致：

```text
sendTextToConnection(connectionId: string, payload: string) -> void
sendBinaryToConnection(connectionId: string, payload: bytes) -> void
sendTextToBusinessIdentity(businessIdentity: string, payload: string) -> void
sendBinaryToBusinessIdentity(businessIdentity: string, payload: bytes) -> void
```

四者由 `detached_scalar_native` 描述：

- `may_suspend = false`；
- alias/escape/write effects 全部为 false；
- return provenance 是 `Fresh`。

`runtime/native-contract/src/required_context.rs` 对四者都要求
`NativeRequiredContext::Websocket`。目标不应把“Websocket required context”误解为“只能从
WebSocket connect handler 调用”；它表示执行 activation 必须能解析当前 service 的唯一
WebSocket entry。

### 6.2 实际发送链

```text
Skiff std native
  -> runtime/native/src/dispatch/websocket.rs
  -> runtime/host/src/capability_context/websocket.rs
     WebsocketCapabilityContext{service_id, websocket_entry_id, router_sender}
  -> runtime/capability-context/src/outbound_control.rs
     ConnectionSendControl{service_id, websocket_entry_id,
                           business_identity | connection_id, payload_kind}
  -> runtime/transport/src/control_mapper.rs
     ConnectionSendFrameHeader + binary connection.send
  -> router/src/router/runtimeEndpoint.ts
     frame validation + registered sender
  -> Router ConnectionSendHandler
  -> WebSocketConnectionLifecycle index / socket send
```

text UTF-8、binary payload、empty target、sender registration 和 provider-unavailable 已有局部校验。
business/direct 两种 target 在 Host 都要求 `websocket_entry_id: Some(_)`；否则返回
“websocket entry id is not available”。

### 6.3 精确 Activation 缺口

`runtime/activation/src/context.rs` 的 `ActivationContext` 保留 service/deployment/build/bindings，
但不保留唯一 WebSocket entry。`LinkedActivationTemplate` 虽然持有 exact
`Arc<ServiceDeployment>`，`ActiveAssemblyContextSet::from_candidate` 构造 activation 时只带 source
template，deployment 的 gateway entries 没进入 activation。

因此 HTTP、ordinary service call 与 actor adapters 都调用
`websocket_from_request(service_id, None, ...)`。frame 链存在，但从非 WebSocket activation 永远
没有 entry id。

最小修复 seam：

1. 在 assembly admission / `ActiveAssemblyContextSet::from_candidate` 中检查 exact linked
   `ServiceDeployment`；
2. 找出该 service 的零或一个 `IngressProtocol::WebSocket` binding；
3. exact 验证 selector、entry key、entry identity、surface和 canonical internal entry id；
4. 把一个 typed optional entry record 放入 `ActivationContext`；
5. 零 entry 时四个 native fail unavailable；多 entry、dangling key、identity/surface mismatch 在
   admission 时 fail closed；
6. connect request execution 用 header 的 pinned entry id，但必须先与 activation record exact
   match；
7. HTTP/service/actor execution直接取 activation 中的唯一 entry。

这让四个 native 保持现有 public signature。不得给 native 增加 string entry id 参数。

### 6.4 唯一 canonical `WebSocketEntryId`

generation protocol 已在 TS/Rust 两侧接受
`skiff-websocket-entry-v1:sha256:<64hex>`，但 current artifact chain 没有 typed canonical
producer。现存
`runtime/eval/src/assembly_execution/websocket_identity.rs` 使用
event source + `ContractOperationId` + selector + `ServiceProtocolIdentity` 的旧 preimage；F420B
已删除同构 TypeScript owner。

shared checkpoint 必须建立一个 canonical producer，建议由当前
`(serviceId, compiler-owned GatewayEntryKey)` 派生，使 logical entry 跨 version/build 稳定。具体
preimage/prefix必须以 language-neutral golden vectors 冻结。Router/Rust 可以传输或严格重算，但不能
各自拥有不同 hash 规则，也不能复用旧 operation preimage。

## 7. Router 可复用面、client data 与 sender trust

### 7.1 可复用

- `WebSocketConnectionLifecycle`
  - reservation/admission；
  - connection、business identity、runtime generation indexes；
  - policy；
  - business fan-out/direct send；
  - slow-client budget；
  - close/deindex/shutdown。
- `WebSocketGenerationLifecycleRouter` 与 Rust Host generation registry；
- `RuntimeDispatcher`；
- `RuntimeEndpoint` 的 `connection.send` frame decoding/registration；
- generation pin、release 和 endpoint lifecycle。

必须从 lifecycle 删除 receive queue、active receive、pending counters 与
`scheduleReceive`。connect-only 不允许在业务层保留 pending client message buffer。

### 7.2 Client data 当前流向与最窄修复

当前路径：

```text
ws.on("message")
  -> WebSocketGateway.handleClientMessage
  -> lifecycle.scheduleReceive
  -> buildWebSocketReceiveDispatch
  -> dispatchReceive
  -> dispatchWebSocketOperation
  -> RuntimeDispatcher.dispatchBinary
```

最窄 target owner 是 `handleClientMessage` 或 current assembly gateway 的等价 `message`
callback。它必须第一步执行：

```text
lifecycle.close(connection.id, {code: 1003, reason: bounded protocol reason})
```

不得先 parse、enqueue、schedule 或 acquire runtime。验证必须同时覆盖 text/binary，断言
dispatcher invocation count 恒为零；ping/pong 后连接仍保持，peer close 会 deindex。

### 7.3 Connect pin/release 与无 handler

有 connect handler：

- Router 用 exact assembly/deployment/entry binding dispatch；
- generation pin 必须覆盖整个 connection lifetime；
- accept 后持有 receipt，socket close/reject/error 时 exact once release。

无 connect handler：

- Router synthesized accept；
- 不 dispatch、不 acquire runtime generation；
- connection 仍保存 exact snapshot/binding，用于 entry ownership、policy 和 outbound 验证；
- 不得伪造 runtime pin。

### 7.4 额外发现：current sender trust 缺口

当前通用 `WebSocketGateway` 订阅 `connection.send` 时丢弃 sender 参数，只把 message 交给
`handleConnectionSend`。`RuntimeEndpoint` 只知道 socket 属于某个已注册 assembly replica，并依赖
handler 返回 disposition；现有 gateway handler没有对 sender 的 exact pinned assembly/service/entry
ownership 做完整验证。

`runtime-endpoint-connection-send-trust.test.ts` 只用 synthetic handler disposition 证明 transport
plumbing，不证明真实 gateway authorization。

新的 current assembly gateway 必须验证：

- sender 对应 pinned assembly/generation/replica；
- frame `serviceId` 与 connection owner一致；
- frame `websocketEntryId` 与 current entry一致；
- direct target 确实属于同一 owner；
- mismatch 关闭违规 runtime，不能向 client 发送；
- closed direct-send race 安全返回 miss。

该修复不增加 public semantics，只闭合现有 control frame trust。

## 8. 缺口表

| 缺口 | 当前 owner | 最小修复 owner | Fail-closed 规则 |
| --- | --- | --- | --- |
| untyped singleton authoring | `ecosystem_authoring.rs` / `service_config.rs` | shared schema/compiler leaf | map/list/id/routes/unknown fields全拒绝 |
| connect surface/source | `artifact-model/src/gateway.rs` | shared artifact/identity leaf | unknown kind/source、HTTP source全拒绝 |
| optional connect handler | `deployment.rs` + all exhaustive consumers | shared checkpoint定义 invariant；consumer后续实现 | absent只允许 WS connect；HTTP absent拒绝 |
| exact callable projection | HTTP projector可复用 | compiler WS projector | generic/wrong params/wrong return/nullable拒绝 |
| internal entry id | 只有 wire validators，旧 eval hash含 operation | shared canonical identity owner | 多 producer、旧 preimage、author id拒绝 |
| current connect wire | v2 request wire HTTP-only | serial wire checkpoint | receive/context/unknown field拒绝 |
| activation sole entry | activation丢 deployment entries | runtime admission/activation leaf | 0 unavailable；>1/dangling/mismatch admission fail |
| connect execution | old service-operation receive path | runtime connect-only consumer | no ContractOperationId/context/receive |
| Router v2 gateway | server只启动 assembly HTTP | Router current consumer | legacy manifest fallback拒绝 |
| client data | receive schedule/dispatch | gateway message callback | text/binary close 1003，dispatch count 0 |
| sender authorization | generic gateway drops sender | current assembly gateway + endpoint tests | assembly/service/entry mismatch拒绝 |
| legacy fixtures/oracles | old event/context/service op | convergence leaf | reverse-search gate必须归零到明确 allowlist |

## 9. 最小开发 DAG、互斥范围与快速测试

### D0 — shared authoring/artifact/compiler checkpoint（串行）

精确 write range：

- `std/{websocket.skiff,api.yml}`；
- `artifact-model/src/{ecosystem_authoring.rs,gateway.rs,deployment.rs,service_unit.rs,websocket_ingress.rs,lib.rs}`；
- `artifact-identity/src/{gateway.rs,deployment.rs,runtime_assembly.rs,contract/normalization.rs,lib.rs,tests/**}`；
- `compiler/input/src/service_config.rs`；
- `compiler/driver/generated_deployment.rs`；
- `compiler/driver/http_gateway_projection/**` 的公共 resolver抽取；
- 新 WebSocket gateway projector及 compiler tests；
- compiler legacy WebSocket type projection owner；
- deployment projection/assembly validation/tests；
- 因 enum/optional handler 编译失败而必须机械更新的
  `runtime/{loader,linker,host}` exhaustive match，但此 leaf 不实现 connect业务。

产出：

- strict singleton authoring；
- WebSocket connect surface/kind/source；
- optional-handler invariant；
- canonical internal entry id；
- ServiceDeployment/RuntimeAssembly exact entry emission；
- old operation projection删除。

快速测试：

```bash
cargo test -p skiff-artifact-model -p skiff-artifact-identity \
  -p skiff-compiler-input -p skiff-compiler -p skiff-deployment
node scripts/verify.mjs --only compiler,foundation --list
```

关键正负例：

- path-only、无 handler -> 正好一个 binding，handler absent；
- allowed connect sources + private callable -> exact target；
- list/map/multiple/id/routes/receive/context/message -> reject；
- generic callable、wrong/nullable return、HTTP source -> reject；
- dangling key/identity、HTTP absent handler -> reject；
- entry不进入 ServiceContract，ServiceProtocolIdentity不因 authoring entry新增而变化。

### D1 — current connect-only wire checkpoint（D0 后串行）

精确 write range：

- `router/src/protocol/runtimeAssemblyRequest*.ts`；
- `router/src/protocol/{envelope.ts,runtimeProtocol.ts}` 中需替换的 current frame定义；
- `runtime/transport/src/runtime_assembly_request.rs` 及其
  `{metadata.rs,lexical.rs,strict_json.rs,tests.rs}`；
- 必要的 connect result response wire；
- cross-system request/response JSON corpus与 parity tests。

产出是 HTTP 与 `websocketConnect` 的 closed discriminated union。connect header只含 routing、
connectionId、URL/query/headers/cookies、version（可选）、websocketEntryId、gatewayEntryIdentity；
没有 receive/context/message。

快速测试：

```bash
cargo test -p skiff-runtime-transport runtime_assembly_request
pnpm --dir router test -- runtime-assembly-request
node scripts/verify.mjs --only runtime,router --list
```

此 checkpoint 只冻结 wire，不执行 business handler。

### D2 — Runtime/Host consumer（D1 后）

精确 write range：

- `runtime/activation/src/context.rs`；
- `runtime/loader/src/runtime_assembly/**`；
- `runtime/linker/src/assembly/gateway.rs`；
- `runtime/host/src/loader/{active_assembly_context.rs,assembly_admission.rs}`；
- `runtime/host/src/host/request_entry/**`；
- `runtime/request/**`、`runtime/request-contract/**`；
- `runtime/eval/**` 中旧 WebSocket contract plan/identity/receive owner；
- `runtime/host/src/{capability_context,eval_capability_adapter}/websocket.rs`；
- `runtime/native/**`、`runtime/native-contract/**` 的 targeted tests；
- Rust generation lifecycle。

D0 已接触的 shared loader/linker/host match 必须先提交；D2 才用它们完成真实 validation/execution。

快速测试：

```bash
cargo test -p skiff-runtime-activation -p skiff-runtime-loader \
  -p skiff-runtime-linker -p skiff-runtime-request \
  -p skiff-runtime-transport -p skiff-runtime-eval \
  -p skiff-runtime-native -p skiff-runtime-host
```

重点新增当前缺失的 activation/capability/linker/loader WebSocket entry tests；现有 filtered
discovery在这些包中几乎为零。

### D3 — Router current assembly consumer（D1 后，可与 D2 并行）

精确 write range：

- `router/src/router/{runtimeAssemblySnapshot.ts,runtimeAssemblyDeploymentSnapshot.ts,filesystemRuntimeAssemblySnapshotLoader.ts}`；
- 新 current assembly connect-only gateway；
- `router/src/router/{server.ts,runtimeEndpoint.ts,runtimeDispatcher.ts}`；
- `router/src/gateway/{webSocketGateway.ts,webSocketConnectionLifecycle.ts}` 中通用 lifecycle收敛；
- `router/src/router/webSocketGenerationLifecycleRouter.ts`；
- `router/src/index.ts`；
- Router legacy WS manifest/projection residue；
- `router/tests/**`，但不修改 D1 的 protocol corpus。

快速测试：

```bash
pnpm --dir router test
pnpm --dir router exec tsc --noEmit
node scripts/verify.mjs --only router
```

D2/D3 只有在 D1 独占所有跨语言 protocol file、D0 完成 shared schema 和 Rust exhaustive consumer
机械修复后才真正互斥。它们不能与 D0/D1 同时开始。

### D4 — fixture/oracle/tooling convergence（D2 + D3 后）

精确 write range：

- `test-runner/fixtures/package-service-{websocket-smoke,websocket-generation-a,websocket-generation-b,i02-spawn-submit}/**`；
- `test-runner/tests/package_service_contract_deployment.rs`；
- `scripts/tests/package-service-*.test.mjs`；
- D1 未独占的 cross-system checkpoints/goldens；
- verify/reverse-search tooling。

快速测试：

```bash
cargo test -p skiff-test-runner package_service
node --test scripts/tests/package-service-*.test.mjs
node scripts/verify.mjs --only compiler,foundation,runtime,router,tooling
```

### D5 — 最早 cheap combined probe

D2/D3 首次集成后，在隔离临时目录中增加一个 current service：

- 一个可选 connect handler返回 business identity；
- 一个普通 HTTP handler调用 text/binary business-identity send；
- 加 direct connection send覆盖；
- exact v2 ServiceDeployment/RuntimeAssembly loader；
- client connect后只由 HTTP 上行触发下行；
- client text/binary各触发 `1003` 且 runtime dispatch计数不变；
- ping/pong不触发业务；
- peer close释放generation/index；
- 另一个无 handler entry synthesized accept且零 runtime dispatch。

建议形成独立命令：

```bash
node --test scripts/tests/package-service-connect-downlink-combined.test.mjs
```

probe 不访问 stable/live，不启动完整 N5。F424B/F424C 未完成前，D0-D5 都不能单独解除 consumer
迁移或 N5。

## 10. 验证矩阵

| 面 | 正例 | 负例 / invariant |
| --- | --- | --- |
| authoring | singleton path；optional connect target；host default | list/map/multiple、id、routes、receive/context/message、unknown field拒绝 |
| callable | private non-generic；allowed source；non-null result | missing/dangling symbol、generic、wrong args/return、HTTP source拒绝 |
| deployment | selector -> exact key/identity；no-handler WS | duplicate selector、dangling key、identity mismatch、HTTP no-handler拒绝 |
| contract isolation | WS entry不新增 operation | ServiceContract/ServiceProtocolIdentity delta即失败 |
| wire | HTTP bytes保持；connect metadata exact | receive/context/unknown union、identity mismatch拒绝；TS/Rust bytes parity |
| runtime connect | exact callable invoke；accept/reject/policy | old operation lookup、Context decode、receive dispatch均不可达 |
| no-handler | Router accept，0 dispatch，0 runtime acquire | dummy handler或fake pin即失败 |
| activation outbound | HTTP/service/actor中四个 native都带 service + sole entry | 0 entry unavailable；>1/mismatch admission失败；empty target拒绝 |
| Router data | text/binary -> 1003；dispatcher count 0 | 任何 queue/build/dispatch invocation即失败 |
| protocol control | ping/pong保持；close deindex/release | protocol frame进入 business runtime即失败 |
| policy/fan-out | `(service,entry,business)`；跨 build/version fan-out | scope/version/build进入 key即失败 |
| sender trust | exact pinned sender/direct owner发送 | assembly/service/entry mismatch关闭 runtime `1008`；client不收 payload |
| race/cleanup | closed direct send miss；release exact once | dangling index、double release、old generation send即失败 |

## 11. Identity、generation 与 oracle 变化

- `compiler/driver/generated_deployment.rs::generated_revision` hash 包含 service manifest/profile/build；
  新增或修改 WebSocket authoring 会改变 `DeploymentRevision`。
- `ServiceDeployment` identity 包含 gateway entries + ingress，因此
  `DeploymentArtifactIdentity` 改变。
- `RuntimeAssembly` identity包含 resolved deployment与gateway ingress，因此 `AssemblyIdentity`
  改变。
- `GatewayEntryIdentity` 只由 protocol surface决定：handler implementation/build变化不应改变它；
  request/result/policy/downlink surface变化必须改变它。
- 单纯 `service.yml` gateway entry变化不应改变 `ServiceProtocolIdentity` 或 `PackageArtifact`。
- 修改 `std/websocket.skiff` 后 std自身和依赖它的 package source需要重新编译，对应
  `PackageBuildId` 会变化。
- connection generation tuple继续使用 exact assembly identity + canonical internal entry id。
- fixture pointers、assembly/deployment receipts、generation oracles和cross-language corpus必须随新
  identity重建，不能手工保留旧 digest。

## 12. Legacy 反向搜索

### 12.1 计数方法

计数固定在 production input
`ba74febaca5dbe8f2b55d6db04e0544a6758bf4b`，不是包含 F424 planning docs 的 audit HEAD。
表中格式是“occurrence / file”：

- `doc`：`doc/**`；
- `test`：路径位于 `tests/**`、`test-runner/**`、`fixtures/**`，或文件名为
  `tests.rs` / `_tests.rs`；
- 其余为 `production`；
- production 文件内嵌的 `#[cfg(test)]` 仍按文件路径计入 production；
- literal均大小写敏感；组合项使用下方列出的 regex。

| search | production | test/fixture | doc |
| --- | ---: | ---: | ---: |
| `WebSocketIngressEvent` | 31 / 19 | 23 / 12 | 16 / 11 |
| `receiveEvent` | 29 / 9 | 33 / 11 | 9 / 5 |
| `websocketReceive` | 14 / 3 | 10 / 4 | 6 / 6 |
| `websocket.receive` | 9 / 7 | 11 / 4 | 2 / 2 |
| `contextCodec` | 32 / 4 | 8 / 5 | 1 / 1 |
| `contextPayloadPresent` | 18 / 4 | 24 / 6 | 2 / 2 |
| Assembly WS regex | 0 / 0 | 0 / 0 | 94 / 35 |
| legacy operation regex | 10 / 5 | 9 / 4 | 2 / 2 |
| old routes regex | 9 / 5 | 3 / 2 | 3 / 3 |

补充 shape inventory：

| search | production | test/fixture | doc |
| --- | ---: | ---: | ---: |
| `WebSocketReceiveEvent` | 30 / 16 | 2 / 2 | 8 / 6 |
| `WebSocketConnection<` | 2 / 1 | 0 / 0 | 2 / 2 |
| `WebSocketConnectResult<` | 1 / 1 | 21 / 9 | 10 / 8 |

组合 regex：

```text
AssemblyWebSocket|assemblyWebSocket|assembly-websocket|assembly_websocket
WEBSOCKET_INGRESS_OPERATION_NAME|operation: websocket|operationName.*websocket|websocket.*Operation
GatewayWebSocketRoute|entry.routes|projected.routes|websocket.routes|routes?: unknown[]
```

multiline YAML `websocket:` 后接 `routes:` 另有 compiler driver内嵌 test 1处和doc 1处。旧 routes
production命中集中于 artifact-model legacy export/service unit 和 Router
manifest projection/parser/types；test命中集中于 Router manifest validation/gateway tests。

F420B 的旧 Assembly gateway production/test命中为零，证明目标实现必须新增 current v2
connect-only owner，而不是恢复已删文件。D4 应把上述词收敛到 protocol/history doc或明确的
negative rejection allowlist；不能仅降低总数。

## 13. 真实 test discovery 与审计限制

已执行的 authoritative discovery：

```bash
node scripts/verify.mjs --only compiler,foundation,runtime,router,tooling --list
```

结果：PASS，列出 58 个 phase。它确认真实 suite owner包括 compiler/foundation/runtime Cargo
packages、`cwd=router pnpm test`，以及 package-service authoring/ecosystem/I02/cross-system tooling。

已执行 focused Cargo library discovery：

```bash
CARGO_TARGET_DIR=/tmp/skiff-p5-f424a-cargo-target \
  cargo test --lib \
  -p skiff-artifact-model -p skiff-compiler-input -p skiff-compiler \
  -p skiff-deployment -p skiff-runtime-transport -p skiff-runtime-request \
  -p skiff-runtime-eval -p skiff-runtime-host websocket -- --list
```

结果：PASS，共发现 48 个 test：

| package | count |
| --- | ---: |
| artifact-model | 4 |
| compiler | 1 |
| compiler-input | 0 |
| deployment | 0 |
| runtime-eval | 17 |
| runtime-host | 8 |
| runtime-request | 6 |
| runtime-transport | 12 |

多数 runtime命中仍证明 receive/context legacy，后续要删除或改写，不能当作 connect-only coverage。

另一个 `--lib` discovery 对
runtime-native/native-contract/activation/capability-context/linker/loader执行成功：native 只有3个
WebSocket test，activation/capability/linker/loader没有 sole-entry coverage；`connection` filter也
没有补出相关测试。这是 D2 的明确 test gap。

下列 discovery 没有伪报通过：

1. 同一 Cargo package组去掉 `--lib` 后，在既有
   `compiler/tests/actor_dispatch_linking.rs:92` 停止：test仍访问不存在的
   `RuntimeAssembly.global_ingress`，current字段是 `gateway_ingress`。该文件相对 audit input零 diff，
   与本只读审计无关。
2. `pnpm --dir router exec vitest list` 返回 exit 254：
   `ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL`，当前 worktree未安装 vitest依赖。没有因审计而安装依赖。
   Router的权威执行命令仍由 verify plan确定为 `cwd=router pnpm test`。
3. `cargo test -p skiff-test-runner websocket -- --list` 编译中因 `/tmp` 空间不足停止，未得到可信
   list。审计专用 `/tmp/skiff-p5-f424a-cargo-target` 随后被安全删除，没有触碰 repo文件。

本任务是 owner audit，未把 discovery failure解释为候选实现失败。后续实现 leaf 必须在依赖已安装、
独立 target容量足够的环境中执行完整 suites，而不能只复用这次 list结果。

## 14. Worktree 与 handoff

写入本 result 前：

- branch：`codex/p5-f424a-connect-audit`；
- `git status --short` 为空；
- production/test/fixture/design diff为空；
- audit HEAD 相对 exact production input只包含既有四份 F424 planning docs。

提交约束：

- 只 stage/commit
  `P5-F424A-skiff-connect-outbound-owner-audit-result.md`；
- 不 merge/rebase/push；
- commit后的 exact commit/tree 与 clean status由任务交付回执记录。

handoff顺序固定为 `D0 -> D1 -> (D2 || D3) -> D4 -> D5`。本结果只解除共享
Skiff authoring/artifact/compiler与后续 Router/runtime开发 leaf 的规划阻塞；不解除 consumer迁移或
N5。
