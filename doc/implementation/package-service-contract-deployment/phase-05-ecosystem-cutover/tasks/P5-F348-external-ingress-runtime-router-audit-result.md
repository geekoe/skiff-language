# P5-F348 External ingress runtime / Router audit result

状态：Completed（只读审计）。

## 审计基线与边界

- worktree：`/Users/geek/workspace/skiff-p5-f348-ingress-runtime-audit`
- branch：`codex/p5-f348-ingress-runtime-audit`
- exact commit：`6fe25aa1c2545d76f63e96b0261516cfdc288e99`
- exact tree：`a458352384a28a055103ae17f617724d4026077f`
- 审计开始时 worktree clean。
- 只读检查了 `artifact-model` 的 deployment/runtime assembly DTO、`runtime/**`、
  `router/**` 和相关 cross-system fixtures。
- 未运行 workspace/stable/live、未启动本地服务、未运行测试，也未修改 production、test、
  corpus 或 lockfile。

## 结论

当前 `RuntimeAssembly` external ingress 并不是已有 `gatewayEntryIdentity` 链的延伸，而是另一条
以 `ContractOperationId` 代替 gateway entry 的平行链：

1. **首次语义损失发生在共享模型入口。**
   `DeploymentIngressBinding` 在 `artifact-model/src/deployment.rs:92-107` 已经把 ingress 定义为
   `selector -> contract_operation_id`。到 `GlobalIngressBinding`
   （`artifact-model/src/runtime_assembly.rs:80-108`）时，handler、adapter plan、gateway entry
   identity 和 entry 自身的 linked signature 都已不存在；loader/linker 只能继续恢复成 service
   operation。
2. **已有 manifest gateway identity 与 assembly ingress identity 不是同一 owner。**
   legacy manifest HTTP 路径只携带上游提供的 identity；manifest WebSocket 路径由 Router
   `router/src/manifest/identity.ts` 计算三种 identity。assembly WebSocket 又在 Router 和 Rust
   各自重算一套含 `ContractOperationId + serviceProtocolIdentity` 的合成 identity。assembly HTTP
   则没有 canonical `gatewayEntryIdentity`。
3. **唯一应保留的跨系统收敛 seam 是 typed RuntimeAssembly request，而不是 service call
   addressing。**
   Router 应只发送 `{assembly identity, generation, ingress selector, gateway entry identity}` 和
   request-specific metadata；Host 从已 admission 的 linked gateway entry 解析 activation、
   executable、signature 和 adapter plan。随后继续复用
   `ActiveAssemblyRoute -> RuntimeAssemblyRequestTarget -> in-process boundary dispatcher`。
4. **普通 request 执行已经不需要伪造 service caller。**
   wire 强制 `caller.kind = gateway`；Host 直接以 ingress 对应 activation 开始
   `RequestActivationContext`，不会走 service requirement lookup 或 provider switch。这一部分可原样
   保留，只需把当前 `RuntimeAssemblyServiceCallTarget` 的 service-operation 身份外壳泛化为
   boundary/gateway-entry target。
5. **当前 HTTP stream 证据存在跨层矛盾。**
   Router assembly HTTP gateway 会发 `serverStream` 并消费 stream frames，但 Host 和 request
   runner 明确只接受 `unary`。现有 Router stream 测试使用协议 peer/fake runtime，不能证明真实
   Router -> Host stream 链可用。

目标依赖方向应固定为：

```text
shared artifact model + canonical identity owner
                  |
        +---------+----------+
        |                    |
  Rust loader/linker   Router snapshot/gateway
        |
  activation/admission
        |
  Host/transport/request/eval
        |
 HTTP/WS full-chain probes + F346 error acceptance
```

Router 不能成为 Rust 的 canonical identity 依赖；Rust 也不应继续复制 Router 的 TypeScript 哈希
实现。

## 1. Rust 中仍要求 ContractOperationId 的 DTO 与查表

### 1.1 External ingress 上必须切除的耦合

| 层 | 当前 DTO / 查表 | 当前行为与问题 |
| --- | --- | --- |
| deployment model | `DeploymentIngressBinding.contract_operation_id`，`artifact-model/src/deployment.rs:102-107` | 最早把 external entry 压成 service operation；handler、adapterArgs、entry identity、signature 在进入 runtime DTO 前已经丢失。 |
| assembly model | `GlobalIngressBinding.contract_operation_id`，`artifact-model/src/runtime_assembly.rs:80-108` | `RuntimeAssembly.global_ingress` 只能携带 selector/deployment/contract/operation。 |
| loader | `validate_ingress`，`runtime/loader/src/runtime_assembly/graph_validation.rs:441-471` | 以 `(selector, deployment, contract, contractOperationId)` 比较 deployment 与 assembly，固化错误模型。 |
| linker | `link_ingress`，`runtime/linker/src/assembly.rs:504-559` | 用 operation id 查 `activation.operation(...)` 和 contract descriptor，结果仍只是 `GlobalIngressBinding`。 |
| linked candidate | `AssemblyLinkedCandidate.ingress: BTreeMap<IngressSelector, GlobalIngressBinding>`，`runtime/linker/src/assembly/candidate.rs:142-200` | 没有直接 linked handler target、entry signature 或 adapter plan；这是 loader 后的第二个“仍未恢复”点。 |
| activation/admission index | `operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>`，`runtime/host/src/loader/active_assembly_context.rs:20-28,118-132,307-315` | 该表对 internal service calls 合法，但 ingress 也被迫复用它。 |
| Host route | `ActiveAssemblyRoute` 和 `AssemblyAdmission::route`，`runtime/host/src/loader/assembly_admission.rs:70-79,432-466` | selector 命中后再次用 contract operation 查 descriptor 和 provider target。 |
| eval seam | `resolve_ingress_target(... ContractOperationId ...)` 与 `RuntimeAssemblyServiceCallTarget.operation`，`runtime/eval/src/assembly_seam.rs:282-401` | external ingress 被包装成 service-call-shaped target；descriptor 仍从 `ServiceContract.operations` 取得。 |
| request diagnostics | `canonical_runtime_operation`，`runtime/request/src/assembly_ingress.rs:237-258` | telemetry/runtime operation 的 operation、target 和 SPI 都从 service operation descriptor 构造。 |
| WebSocket contract plan | `runtime/eval/src/assembly_execution/websocket_ingress.rs:40-72` | 每次 request 以 service contract operation 编译 `PinnedWebSocketContractPlan` 并校验 executable。 |
| Rust transport | `RuntimeAssemblyRequestRoutingFrameHeader.contract_operation_id`，`runtime/transport/src/runtime_assembly_request.rs:148-168`；lexical validator 在 `runtime/transport/src/runtime_assembly_request/lexical.rs:87-108` | operation id 是 nested routing 的必填字段。 |
| Host wire bridge | `validate_route` 和 `request_envelope_from_route`，`runtime/host/src/host/request_entry/assembly_wire.rs:212-296` | wire、binding、descriptor 三处 operation id 必须相等；随后又把它写成 request `target` 和 SPI。 |

### 1.2 必须保留的 internal service operation 模型

以下 `ContractOperationId` 使用是 internal service boundary 的正确模型，external ingress cutover
不应把它们一并删除：

- `ServiceDeploymentOperationInput` / `DeploymentOperationBinding`
  （`artifact-model/src/deployment.rs:69-83`）：service contract operation 到 provider callable 的
  实现绑定。
- `ResolvedServiceBinding.used_operations`
  （`artifact-model/src/runtime_assembly.rs:50-57`）：caller publish-time 可用 operation 集。
- loader 对 provider callable 的 boundary projection 与 contract descriptor 做相等校验
  （`runtime/loader/src/runtime_assembly/graph_validation.rs:109-190`）。
- `LinkedContractOperation`
  （`runtime/linker/src/assembly/candidate.rs:18-41`）以及
  `link_contract_operations`
  （`runtime/linker/src/assembly.rs:285-341`）。
- `ActivationServiceBinding` 与 `resolve_service_binding`
  （`runtime/activation/src/context.rs:68-114,227-255`）。
- `RuntimeAssemblyEvalTarget::resolve_service_call` 的 requirement slot、SPI witness、used operation
  和 provider activation switch。external ingress 不应进入这条分支。

因此 shared model 需要增加独立的 gateway-entry/link target，而不是把所有 operation table 改名或
删除。

## 2. gatewayEntryIdentity 的现有 owner、校验、传输和重复实现

### 2.1 Legacy manifest HTTP

当前 Router 审计范围内没有 HTTP gateway identity 计算 owner：

1. `router/src/artifacts/manifestProjection.ts:383-506` 从 service assembly 原样复制
   `gatewayEntryIdentity`、handler 和 adapter。
2. `router/src/manifest/loadManifest.ts:338-486` 解析 HTTP route；identity 只做
   `skiff-gateway-v1:sha256:<hex>` pattern 校验
   （`router/src/manifest/loadManifest.ts:1397-1404`），没有 recompute/equality validation。
3. `router/src/router/httpGateway.ts:371-410,750-783,1109-1130` 把 identity、dispatch target、
   handler/guard/pre 和 adapterArgs 放进 legacy `request.start`。
4. legacy Runtime 注册时，
   `runtime/host/src/host/register_mapper.rs:159-184` 递归扫描 gateway JSON 中所有
   `gatewayEntryIdentity`；Router wire validator 在
   `router/src/protocol/runtimeProtocol.ts:2503-2566` 只校验列表 pattern。
5. `router/src/router/runtimeRegistry.ts:779-911,1028-1053` 把 identity 作为 target/build route
   index 的附加维度，但 `runtimeAcceptsGatewayEntry`
   （`router/src/router/runtimeRegistry.ts:1105-1114`）在 runtime 没注册 identity index 时会
   permissive 通过。

所以 HTTP identity 的真正 producer 在本审计范围之外的 artifact/compiler producer；Router 是
transport/partial validator，不是完整 canonical owner。

### 2.2 Legacy manifest WebSocket

`router/src/manifest/identity.ts:17-83` 是当前 manifest WebSocket 的实际计算 owner：

- `computeWebSocketConnectIdentity`
- `computeWebSocketReceiveIdentity`
- `computeWebSocketEntryIdentity`

它们用 stable JSON + SHA-256 计算 connect、receive、whole-entry 三个不同 identity。
`serviceProtocolIdentity` 虽然出现在函数输入类型中，却没有进入三段 hash body。

`router/src/manifest/loadManifest.ts:948-1105`：

- 先校验 operation、adapterArgs、connect/receive response shape 和 operation metadata；
- 再计算三种 identity；
- 若 artifact 提供 identity，则严格要求等于 Router 计算值
  （`router/src/manifest/loadManifest.ts:1380-1395`）。

`router/src/gateway/webSocketGateway.ts:500-533,584-705,787-840` 在 connection 上保留 whole-entry、
connect、receive identity 以及各 phase SPI；connect 和 receive request 分别发送各自
`gatewayEntryIdentity`。这里的 `websocketEntryId` 仍是 manifest entry 的 `id`，不是
`skiff-websocket-entry-v1` identity。

### 2.3 RuntimeAssembly WebSocket 的第二套 owner

assembly 路径没有复用上述 manifest owner，而是出现两份等价重实现：

- Router：`canonicalAssemblyWebSocketIngressIdentity`，
  `router/src/router/assemblyRuntimeRegistry.ts:642-665`。
- Rust eval：`recompute_admitted_identity`，
  `runtime/eval/src/assembly_execution/websocket_identity.rs:15-90`。

两者 hash body 都包含：

- hard-coded `event:websocket.ingressEvent`
- `contractOperationId`
- selector
- service id
- `serviceProtocolIdentity`

并把同一 digest 分别加上 `skiff-websocket-entry-v1` 和 `skiff-gateway-v1` prefix。Rust 并没有
“复用 Router owner”，只是复制同一 projection 并通过 frozen corpus 对齐。

此外：

- Router assembly connection 对 connect/receive 使用同一个合成 gateway identity，不是 legacy
  manifest 的 connect/receive/entry 三 identity 模型。
- assembly HTTP request builder
  （`router/src/router/assemblyHttpGateway.ts:210-265`）不发送 `gatewayEntryIdentity`。
- assembly registration 只注册 environment/generation/assembly/replica
  （`router/src/protocol/assemblyActivationProtocol.ts:62-98`），
  `AssemblyRuntimeRegistry` 也不注册 per-entry identity。
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts:205-249` 还在 TypeScript 内重算
  RuntimeAssembly identity 和 ServiceProtocolIdentity；这是另一个跨语言 canonical projection
  duplicate owner 风险。

### 2.4 Canonical owner 结论

Rust 不能直接依赖或调用 TypeScript `manifest/identity.ts`。正确方向是：

1. 在 shared artifact/identity dependency leaf 定义唯一 external gateway entry projection、typed
   identity 和 language-neutral golden vectors。
2. compiler/artifact producer 写入 canonical identity。
3. Rust loader 与 Router snapshot loader 都对同一 serialized entry 做严格 equality validation，
   或在明确只有 producer 计算的方案下，至少由同一个共享 projection/golden 规范验证。
4. Router 不再从 `ContractOperationId` 合成 assembly identity；Rust eval 也不再 request-time
   recompute 这套合成值。

## 3. Handler、adapter、signature、activation 与 request lifecycle 跳点

### 3.1 当前消费位置

| 事实 | legacy manifest 路径 | RuntimeAssembly 路径 | 审计结论 |
| --- | --- | --- | --- |
| handler target | `loadManifest` 在 `router/src/manifest/loadManifest.ts:368-450` 从 operation target 或 package handler target 得到 `dispatchTarget`；HTTP gateway 发送 target/handler。 | shared model 没有 handler；linker只能用 contract operation 找 provider target。 | handler 必须进入 shared gateway entry，并由 loader/linker 从 package callable resolve；wire 不应让 Router选择或注入 handler。 |
| adapterArgs | manifest loader 校验并由 HTTP/WS gateway 传输。 | HTTP transport DTO有 handler/adapterArgs（`runtime/transport/src/runtime_assembly_request/metadata.rs:60-136`），但 Host 明确拒绝；WS 则 Router 和 Host 都 hard-code 单一 event arg。 | static adapter plan 应来自 admitted linked entry；wire 只带 request-specific HTTP/WS metadata，或对携带的 static plan做 exact-match。 |
| linked signature | legacy service operation 依赖 operation ABI/runtime program。 | loader 用 ServiceContract `BoundaryOperationDescriptor` 校验 callable projection；descriptor 留在 `ServiceContractStore`，linked operation只留 target。 | 新 linked gateway entry 必须保留/可查自身 boundary signature、mode、adapter plan 和 target，不能借 ServiceContract operation descriptor。 |
| activation owner | legacy registry按 service/build/target/activation identity选择。 | selector -> deployment -> exact `ActivationContext`；`ActiveAssemblyRoute` 持有 active generation 和 activation。 | assembly activation owner 模型正确，应保留。 |
| timeout | manifest gateway按 route/operation timeout生成 deadline。 | `DeploymentPolicy.timeout_ms` 存在（`artifact-model/src/deployment.rs:169-177`），但 assembly Router snapshot/gateway不消费；HTTP/WS gateway使用 option 或默认 120s。Host把 wire deadline放入 request extra。 | shared entry/activation policy要明确唯一 effective-timeout owner；不能继续让 deployment policy静默失效。 |
| cancel | Router dispatcher在 timeout/client disconnect/backpressure 等终态发 `request.cancel`（`router/src/router/runtimeDispatcher.ts:914-1001`）；Host supervisor cancel token。 | 同一机制已被 assembly gateway/Host复用。 | 可保留。 |
| error | generic control 与 typed fixed service failure 分支。 | assembly Host 已完整复用。 | 见 F346 小节，可保留。 |
| stream consumption | legacy request layer有 `RuntimeIngressHandler`/`BinaryHttpIngressHandler` 和 `ResponseStreamWriter`（`runtime/request/src/runtime_ingress.rs:25-45`、`runtime/request/src/http_ingress.rs:23-100`）。 | Router发/收 stream；Host `validate_narrow_unary_header` 和 request runner 均拒绝非 unary。 | 必须新增真实 assembly stream handoff；不能仅改 Router DTO。 |

### 3.2 HTTP adapter 和 stream 的明确断裂

Rust transport 已能 decode `httpAdapter.handler/guard/pre/adapterArgs`
（`runtime/transport/src/runtime_assembly_request/metadata.rs:60-136`），但：

- `runtime/host/src/host/request_entry/assembly_wire.rs:109-180` 要求 mode 为 `unary`，并拒绝任何
  HTTP adapter metadata。
- `runtime/request/src/assembly_ingress.rs:190-235` 再次拒绝非 unary 与 legacy HTTP adapter。
- 对应 Host 测试
  `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs:193-290`
  明确把 `http-adapter` 和 `server-stream` 都列为必须 fail closed。
- 与之相反，`router/src/router/assemblyHttpGateway.ts:109-179` 会按
  `binding.operationMode === serverStream` 调用 `dispatchBinaryStream`。
- `router/tests/assembly-http-gateway-stream.test.ts:58-125` 只让协议 peer 模拟 runtime stream
  frames，没有经过真实 Host，因此掩盖了断裂。

shared model cutover 必须同时解决 handler/signature/adapter 和 stream consumer，不能把当前
transport 中未被信任的 `httpAdapter` 字段简单放开。

### 3.3 不伪造 service caller 的普通 request 收敛

当前这段路径是正确且可复用的：

1. Rust lexical validator 强制 `caller.kind == "gateway"`
   （`runtime/transport/src/runtime_assembly_request/lexical.rs:44-49`）。
2. Host admission 先以 selector 固定 `ActiveAssemblyRoute`，其中已有 exact assembly generation、
   deployment activation、descriptor 和 executable target。
3. `ActiveAssemblyRoute::request_target`
   （`runtime/host/src/loader/assembly_admission.rs:134-149`）直接
   `RequestActivationContext::begin(receiver activation)`。
4. `resolve_ingress_target`
   （`runtime/eval/src/assembly_seam.rs:282-362`）要求 current 与 receiver 是同一 `Arc`，不调用
   `resolve_service_binding`、不切换 provider，也不创建 service caller。
5. `execute_runtime_assembly_request`
   （`runtime/request/src/assembly_ingress.rs:33-163`）进入 production in-process boundary。
6. `dispatch_in_process_boundary` 使用 `InProcessBoundaryDispatchOrigin::Ingress`；fixed service
   failure 保持 typed failure 向 Host 返回，不执行 internal service caller import
   （`runtime/eval/src/assembly_execution/mod.rs:131-153`）。

目标实现只应把第 2～4 步的 lookup key 和 boundary target 从 contract operation 改为 linked
gateway entry；第 5～6 步是唯一 runtime execution convergence。

## 4. WebSocket connection identity、generation 与 drain

### 4.1 当前保持方式

Router assembly WebSocket connection 保存：

- connect 时取得的 immutable snapshot 与 binding；
- `websocketEntryId` / `gatewayEntryIdentity`；
- context bytes/codec、business identity；
- dispatcher-issued runtime connection receipt。

定义见 `router/src/gateway/assemblyWebSocketGateway.ts:83-94`。connect 在
`router/src/gateway/assemblyWebSocketGateway.ts:229-332`：

1. 从当前 committed snapshot 选 ingress；
2. 计算 identity 并把 snapshot/binding/identity 存到 connection；
3. `expectConnection` 预登记 exact
   `{serviceId, assemblyIdentity, generation, websocketEntryId, connectionId}`；
4. connect response 必须由同一个 runtime receipt 完成；
5. runtime 发 acquire 后才接受 upgrade。

receive 在 `router/src/gateway/assemblyWebSocketGateway.ts:352-381` 使用 connection 中保存的旧
snapshot、binding、entry identities 和 receipt，不重新读取 current snapshot。

Runtime 侧：

- `WebSocketGenerationPin` 直接持有 exact `ActiveAssemblyRoute`
  （`runtime/host/src/host/websocket_generation.rs:23-60`）。
- connect acquire tuple 定义在
  `runtime/transport/src/websocket_generation_lifecycle.rs:43-52`，并在
  `runtime/host/src/host/websocket_generation.rs:77-145` 创建。
- receive 通过
  `runtime/host/src/host/request_entry/websocket_generation.rs:22-51` 查 pin；`pinned_route`
  （`runtime/host/src/host/websocket_generation.rs:268-297`）校验 assembly identity、
  generation、websocket entry id 后返回旧 route，不做 artifact I/O。
- release exact-match、幂等 cache 和 disconnect cleanup 在
  `runtime/host/src/host/websocket_generation.rs:299-408`。

Router lifecycle 还校验 acquire sender 就是 pending connect request 的 runtime，并以 pin count
参与 drain（`router/src/router/webSocketGenerationLifecycleRouter.ts:83-161,250-334`）。
`AssemblyRuntimeRegistry.replicaCanUseActivation`
（`router/src/router/assemblyRuntimeRegistry.ts:373-381`）只允许仍有 in-flight 或 connection pin
的 draining replica 继续使用旧 activation。

这套 generation pin / receipt / drain owner 是正确的，不应重做。

### 4.2 仍绑 serviceProtocolIdentity 的位置

- assembly synthetic WebSocket identity 的 Router 和 Rust hash body 都包含 SPI。
- `GlobalIngressBinding.contract` 和 Router `RuntimeAssemblyIngressBinding.contract` 仍携带 SPI，
  目的是回查 ServiceContract operation/mode。
- Router snapshot 为得到 mode，必须加载 exact ServiceContract 并按
  `(service id, version, SPI, contractOperationId)` 查 operation
  （`router/src/router/runtimeAssemblySnapshot.ts:212-399`）。
- Host request envelope 和 `RuntimeOperation` 仍写入 SPI
  （`runtime/host/src/host/request_entry/assembly_wire.rs:255-296`、
  `runtime/request/src/assembly_ingress.rs:237-258`）。
- legacy runtime registry 的 route/cursor key 仍包含 SPI、target 和可选 gateway identity
  （`router/src/router/runtimeRegistry.ts:1192-1242`）；SPI在 build 选择后作为 compatibility
  witness 校验（`router/src/router/runtimeRegistry.ts:849-875`）。
- legacy WebSocket connection 保留 connect/receive phase SPI 并随 request 发送。

但 generation lifecycle tuple 本身**没有** SPI 或 contract operation；它只含 service id、assembly
identity/generation、websocket entry id 和 connection id。这说明 pin/drain 模型可直接迁移到
canonical gateway entry。cutover 时应把 tuple/route 校验绑定到 shared connection entry identity
（并按模型决定是否另带 phase gateway identity），不要重新引入 SPI。

## 5. F346 fixed error 链复用与证据失效

### 5.1 可原样复用的 API

operation identity 改动不应改变 F346 的 wire/error contract：

- ingress boundary 保留 `RuntimeError::FixedServiceFailure` typed branch：
  `runtime/eval/src/assembly_execution/mod.rs:131-153`。
- `RequestError::fixed_service_failure` /
  `fixed_service_response_failure`：
  `runtime/request/src/error.rs:76-91`。generic code/message 不会被升级成 fixed。
- `RequestSupervisor::complete_fixed_service_failure`：
  `runtime/host/src/host/request_supervisor.rs:168-190`，继续记录 trace/error correlation。
- assembly Host 的 fixed branch：
  `runtime/host/src/host/request_entry/assembly.rs:160-207`。
- `ResponseEvent::FixedServiceFailure` 到 strict `response.error` binary frame：
  `runtime/transport/src/response_mapper.rs:20-54`。
- fixed/control 互斥与 payload strict decode：
  `runtime/transport/src/protocol.rs:894-1009`。
- Router dispatcher 恢复 `FixedServiceResponseError`：
  `router/src/router/runtimeDispatcher.ts:651-687`。
- HTTP 的 `toGatewayError`、WebSocket 的 `externalGatewayErrorMessage` redaction/correlation：
  `router/src/router/assemblyHttpGateway.ts:62-70`、
  `router/src/gateway/assemblyWebSocketGateway.ts:624-645`。

`runtime/host/tests/p5_f345_service_error_convergence.rs:205-290` 的 exact bytes、typed branch 和
fixed/control exclusivity 不依赖 external operation identity，应该保持不变。

### 5.2 必须更新的 schema、fixture 与测试范围

#### Cross-system evidence

- `cross-system-fixtures/package-service-ecosystem/checkpoint.json` 的
  `runtimeAssemblyRequestRouting.fields` 当前冻结 `contractOperationId`。
- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json` 的四个正例、mutation、
  raw case 和 option-equivalence evidence 都以 `routing.contractOperationId` 为必填；其 Rust consumer
  是 `runtime/transport/src/runtime_assembly_request/tests.rs:154-168`，fixture verifier 在
  `cross-system-fixtures/package-service-ecosystem/verify.mjs:267-337`。
- `cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json` 的
  `admittedIdentityGolden` 和 mutation 把 SPI/contract operation 纳入 assembly synthetic identity；
  consumer 是 `runtime/eval/src/assembly_execution/websocket_identity.rs:192-269`。

这些 evidence 必须由 canonical gateway entry schema/identity golden 替换；F346 的 service error
corpus和 fixed payload bytes 不应被重写。

#### Rust tests

- `runtime/loader/src/runtime_assembly/tests.rs` 中 deployment/global ingress operation fixtures。
- `runtime/linker/src/assembly/tests.rs` 与 `runtime/linker/src/assembly/tests/fixtures.rs` 中
  `GlobalIngressBinding.contract_operation_id` assertions。
- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs` 和
  `runtime_assembly_request.rs` 中 wire operation equality、wrong-operation、adapter/stream rejection。
- `runtime/host/src/host/router_session/tests/websocket_generation_lifecycle.rs` 的 request header fixture。
- `runtime/request/src/assembly_ingress.rs` 的 canonical selector、legacy adapter rejection 和 WS
  identity metadata tests。
- `runtime/eval/src/assembly_execution/websocket_identity.rs` 的 synthetic identity golden。

internal `LinkedContractOperation`、service binding、service error import/export tests不应因 external
ingress 改名而失效。

#### Router tests

必须重写 RuntimeAssembly ingress fixture/schema assertions：

- `router/tests/compilerGeneratedManifestCompatibility.test.ts`
- `router/tests/host-ingress.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`
- `router/tests/assembly-http-gateway-stream.test.ts`
- `router/tests/assembly-websocket-gateway.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`
- `router/tests/assembly-replica-dispatch.test.ts`
- `router/tests/assembly-runtime-endpoint.test.ts`
- `router/tests/runtime-endpoint-connection-send-trust.test.ts`

F346 behavior assertions仍有效，但以下文件中的 assembly binding/request builders 要改成 gateway
entry identity：

- `router/tests/service-error-cross-layer-convergence.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`
- `router/tests/assembly-http-gateway-stream.test.ts`
- `router/tests/assembly-websocket-gateway.test.ts`

若 legacy manifest identity owner 同时收敛到 shared owner，还需更新
`router/tests/identity.test.ts` 的 frozen hashes，以及
`router/tests/artifacts.test.ts`、`runtime-registry-dispatch.test.ts`、
`websocket-gateway.test.ts` 中 connect/receive/entry identity assertions。不能把这些 legacy
hash tests直接当作新 shared model 的 oracle。

## 6. Shared model 后的并行实现与建议 DAG

### 6.1 Shared prerequisite

先落一个 shared external ingress model 和唯一 identity owner。具体命名可由 F347/F348 汇总决定，
但至少要表达：

- selector 与 canonical gateway entry identity；
- owner deployment / activation；
- handler package callable reference；
- entry kind、mode、boundary signature/type plan；
- canonical adapter plan/adapterArgs；
- HTTP typed/raw metadata；
- WebSocket connection entry及 connect/receive phase facts、context codec contract；
- timeout/policy owner。

`ServiceDeployment.operation_bindings` 和 internal service contract operation graph 保持独立。

### 6.2 可并行工作包

Shared prerequisite 合入后可并行：

1. **Loader/linker/activation**
   - loader 校验 gateway entry identity、selector uniqueness、handler callable closure、boundary
     availability、signature/adapter plan compatibility；
   - linker 产生 `LinkedGatewayEntry`（概念名），直接持有 target 和 admitted boundary plan；
   - activation/admission 建立 `selector/entry identity -> exact activation + linked entry`；
   - 不经 `LinkedContractOperation` 或 `ServiceContractStore.operation_descriptor` 解析 external entry。
2. **Transport codec**
   - Rust/TS 同步把 nested routing 的 `contractOperationId` 换成 canonical
     `gatewayEntryIdentity`/entry reference；
   - 更新 strict field sets、raw decoder、binary corpus 和 negative mutations；
   - 保留 `caller.kind = gateway`、assembly identity/generation、selector、deadline、trace 和
     request-specific HTTP/WS metadata。
3. **Router snapshot/gateway**
   - Router snapshot 直接 decode shared gateway entries，不再为 ingress mode 加载 ServiceContract；
   - 删除 `canonicalAssemblyWebSocketIngressIdentity`；
   - HTTP/WS request 都发送 artifact 中的 exact entry identity；
   - connect/receive connection继续保存 snapshot、entry identity和runtime receipt。

随后：

4. **Host/request/eval integration**（依赖 loader/linker + transport）
   - Host 以 selector + entry identity exact-match admitted route；
   - handler、adapter plan、signature只取自 linked entry，不信任 Router 注入；
   - 泛化 `RuntimeAssemblyServiceCallTarget` 为 ordinary boundary target，同时保留
     `RequestActivationContext::begin` 和 in-process dispatcher；
   - telemetry target 改成 gateway entry identity/stable diagnostic name；
   - 接入 unary 与真实 serverStream consumer，并应用明确的 deployment/entry timeout policy。
5. **WebSocket generation cutover**（依赖 Host + Router）
   - lifecycle tuple/pinned route 使用 shared connection entry identity；
   - 保留 receipt ownership、old-generation route pin、release ack、disconnect cleanup和drain gate。
6. **Evidence/acceptance**（依赖全部 consumers）
   - 更新 schema/corpus/goldens；
   - 重跑 F346 fixed/control exact-byte acceptance；
   - 再做真实 Router/runtime probes。

### 6.3 最小真实探针

不应继续只依赖 fake dispatcher。最小集合建议为：

1. **HTTP external handler probe**
   - 用一个不是 ServiceContract operation 的 package-owned HTTP handler；
   - 经真实 Router binary wire 和真实 Host 执行；
   - 断言 `caller.kind=gateway`、exact assembly generation/gateway entry identity、正确 activation、
     adapterArgs 与 response；
   - identity/selector/handler claim 任一篡改都在 Router或Host admission fail closed。
2. **HTTP serverStream + cancel/error probe**
   - raw HTTP handler产生至少两个真实 chunk；
   - 验证 Router backpressure、client disconnect -> same request/socket `request.cancel`、Host
     cancellation和stream terminal；
   - 在 `response.start` 前触发 fixed service failure，确认 F346 exact typed frame和外部 redaction；
   - 该探针专门关闭当前 Router stream test 与 Host unary-only 实现之间的证据缺口。
3. **WebSocket generation cutover probe**
   - generation A connect 并 acquire；
   - commit generation B 后，新 connect 走 B，旧 connection receive仍走 A 的同一 linked entry和
     runtime receipt；
   - close/release 后 A 的 pin 归零并完成 drain；
   - 篡改 entry identity、generation、connection id 或 sender receipt 均 fail closed。

以上三项通过后，external ingress 才能证明已经从 `ContractOperationId` ingress 收敛到唯一
gateway-entry/runtime request seam。
