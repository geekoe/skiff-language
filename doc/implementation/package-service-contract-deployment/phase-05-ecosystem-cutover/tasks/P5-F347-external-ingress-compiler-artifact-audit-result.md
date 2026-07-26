# P5-F347 External ingress compiler / artifact audit result

状态：完成（只读审计）。

## 审计基线与边界

- commit：`6fe25aa1c2545d76f63e96b0261516cfdc288e99`
- tree：`a458352384a28a055103ae17f617724d4026077f`
- branch：`codex/p5-f347-ingress-compiler-audit`
- 审计对象：`artifact-model`、`artifact-identity`、`compiler/**`、`deployment/**`及其直接
  tests/fixtures。
- 初始 worktree 为 clean；结果文件原先不存在，无既有写入冲突。
- 未运行 workspace/stable/live，未修改 production/test/corpus/lockfile。

## 结论

当前链路确实把 external ingress 错接到了 service operation：

1. `service.yml` 的 `http`/`websocket` 在共享 authoring DTO 中是无类型 JSON；
2. service package 编译把所有 `api.yml` public 且 boundary-available 的 callable 自动投影成
   `ServiceContract.operations`；
3. driver 私有 route parser 把 `operation` 当成该自动 service API 的 stable key；
4. route 最终被写成 `DeploymentIngressBinding.contract_operation_id`，deployment projection 又强制它
   属于 contract operation binding；
5. WebSocket admission 甚至从 `ServiceContract` operation 反推 gateway ABI。

最早的**类型信息损失**发生在
`artifact-model/src/ecosystem_authoring.rs:85-96`：`ServiceManifestAuthoring.http` 和
`websocket` 都是 `Option<serde_json::Value>`，所以 compile input 边界不知道 handler、adapter kind、
adapter args 或 gateway entry。最早的**service-operation 语义提升**发生在
`compiler/contract/src/projection.rs:73-87`：每个 available public callable 以 public path 为 key
进入 contract operation map。最早的**ingress 专属提升**发生在
`compiler/driver/generated_deployment.rs:234-265`：route 的 `operation` 查找
`ServiceContract.operations[*].stable_key`，随后直接取出 `ContractOperationId`。

现有 production 没有可直接复用的 canonical Rust gateway adapter/entry DTO，也没有
`GatewayEntryIdentity` 的 Rust owner。应先落一个共享 model/identity checkpoint；不能让 compiler
复制或反向依赖 Router TypeScript manifest model。

## 当前完整跳转链

### 1. `service.yml` 到 compiler input

- `ServiceManifestAuthoring` 只有强类型的 `id`/`kind`，但
  `http`、`websocket`、`timeout` 都是 JSON value：
  `artifact-model/src/ecosystem_authoring.rs:75-97`。
- `read_service_manifest` 直接对该 DTO 做 YAML serde：
  `compiler/input/src/service_config.rs:109-125`。
- 该边界只额外拒绝 `http.response`，没有解析 route body，也不会在这里拒绝旧
  `operation`：`compiler/input/src/service_config.rs:128-147`。
- `ServiceManifestAuthoring` 顶层虽有 `deny_unknown_fields`，但 opaque JSON 内部没有递归的
  fail-closed schema。因此旧 route 可以穿过真正的 compile input 边界，直到 deployment generation
  才被解释。

### 2. PackageArtifact 到自动 service API

- `compile_service_package` 先生成 canonical `PackageArtifact`，再只用 `service_id` 调
  `project_service_api`：`compiler/driver/pipeline/mod.rs:395-410`。这里没有 ingress/adapter 输入。
- `project_service_api` 遍历 `PackageArtifact.boundary_projections`；每个
  `BoundaryCallableProjection::Available` 都以 public path 为 operation stable key，并记录
  public path 到 `PackageCallableId` 的映射：
  `compiler/contract/src/projection.rs:61-99`。
- 它随后构造并编译 `ServiceContractDefinition`：
  `compiler/contract/src/projection.rs:100-138`。
- `compile_service_contract_definition` 对每个 stable key 调
  `contract_operation_id(service_id, contract_version, key)`，生成
  `ContractOperationId` 和 `BoundaryOperationDescriptor`：
  `compiler/contract/src/compile.rs:17-65`。
- `contract_operation_id` 的 preimage 实际只有 schema marker、`service_id` 与
  `stable_operation_key`；传入的 version label 未进入 identity：
  `artifact-identity/src/contract.rs:43-49,165-184`。

因此，只要 external handler 为了现有 route 被放进 `api.yml`，它就会在 route 尚未解析前先成为
service operation。`api.yml` 在当前模型里既是 Package public API，也是自动 service API 的唯一来源；
不存在“仅供 ingress 使用的 public entry”。

### 3. route parser 到 `ContractOperationId`

- 真正的 route DTO 不是共享 model，而是 driver 私有的 `RouteAuthoring`：
  `compiler/driver/generated_deployment.rs:187-202`。
- 它的 target 字段是无类型 `operation: String`。
- `ingress_bindings` 延迟把 opaque `service.http`/`service.websocket` JSON 反序列化为该 DTO：
  `compiler/driver/generated_deployment.rs:208-232`。
- `resolve_route` 先查 `service_api.unavailable[operation]`，再按 descriptor stable key 查
  `service_api.contract.operations`，最后写入 descriptor 的 `operation_id`：
  `compiler/driver/generated_deployment.rs:234-265`。

这条路径没有 handler source selector、`PackageCallableId` 或 adapter kind；唯一允许的 route target
就是 service operation stable key。

### 4. deployment generation 与 projection

- generator 先为 contract 的**每一个** operation 生成
  `ServiceDeploymentOperationInput { contract_operation_id, package_public_path }`：
  `compiler/driver/generated_deployment.rs:148-185`。
- `ServiceDeploymentInput.ingress` 的 canonical Rust shape 同样固定为
  `{ selector, contract_operation_id }`：
  `artifact-model/src/deployment.rs:85-107,187-206`。
- projection 要求每个 contract operation 恰有一个 public package path，且该 path 必须在
  `package_local_abi.public_symbols` 中：
  `deployment/src/projection/operations.rs:22-65`。
- projection 再要求 public callable 的 boundary projection 与 contract descriptor 完全相等，并把
  service operation 映射成 `DeploymentOperationBinding.package_callable_id`：
  `deployment/src/projection/operations.rs:66-121`。
- identity validation 又要求 ingress target 必须出现在 operation bindings 中：
  `artifact-identity/src/deployment/validation.rs:184-204`。
- WebSocket ingress admission 直接以 ingress 的 `ContractOperationId` 查
  `ServiceContract`：`deployment/src/projection/mod.rs:67-85`；
  `websocket_ingress_context` 还要求 stable key 精确为 `websocket`：
  `artifact-model/src/websocket_ingress.rs:440-477`。

现有正向测试也固定了错误耦合：

- `compiler/tests/generated_service_deployment.rs:13-43` 断言 ingress
  `contract_operation_id` 与 service operation binding 相同；
- `compiler/tests/generated_service_deployment.rs:337-379` 用 public `read` callable 和
  route `operation: read` 生成 deployment；
- `compiler/tests/websocket_ingress.rs:38-120`、`:348-403` 把 WebSocket handler 先做成
  contract operation，再以同一个 operation id 生成 ingress。

## 可复用 owner 审计

### 可以复用

1. **全局 selector**

   `IngressSelector { protocol, host, method, path }` 已是严格 Rust DTO，并已有 HTTP/WS method
   validation：`artifact-model/src/deployment.rs:85-100`、
   `artifact-identity/src/deployment/validation.rs:329-351`。它可原样保留。

2. **source selector 的词法规则**

   `SourceSymbolSelector { module_path, symbol }` 及 `parse` 已在
   `compiler/core/src/api_spec.rs:30-34,227-275` 存在。service input 应复用这个 parser，不应新增第二套
   dotted source-path 规则。

3. **Package callable identity、signature 与 executable link**

   `PackageLocalAbiSymbol::Callable` 已携带 `PackageCallableId` 和
   `PackageCallableSignature`；`PackageArtifact.callable_links` 提供 exact
   `OperationTargetRef`：`artifact-model/src/package_artifact.rs:22-35,44-62,86-115`。

4. **HTTP/WS canonical type vocabulary**

   raw HTTP 的 canonical request/response/stream-event shape owner 已在
   `artifact-model/src/http_boundary.rs:5-47`。
   WebSocket canonical shape graph在
   `artifact-model/src/websocket_ingress.rs:14-227`。后续 adapter validator 应从这些 Rust owners
   提取 signature validation，不能继续从 ServiceContract operation 反推。

### 不能作为 canonical checkpoint 复用

`artifact-model/src/service_unit.rs:283-334` 的 `GatewayConfig`/`GatewayRoute`/
`GatewayWebSocket` 是 legacy `ServiceUnit` runtime adapter：

- target 仍是字符串 `operation`/`operationAbiId`；
- 没有 handler `PackageCallableId`、adapter kind、adapter args、codec plan 或独立 entry identity；
- runtime-program identity 只把整个 legacy gateway serde 成 opaque JSON value：
  `artifact-identity/src/runtime_program.rs:84-91,94-139`。

它不是新四对象链上的 authoritative gateway model，也不能被搬到
`ServiceManifestAuthoring` 或 `ServiceDeployment` 继续延长 legacy operation path。

### 缺失的最小 shared checkpoint

建议 owner 精确落在：

- `artifact-model/src/compile_identity.rs`：新增强类型 `GatewayEntryIdentity`；
- 新 `artifact-model/src/gateway_adapter.rs`：定义 canonical
  `GatewayAdapterKind`、`GatewayAdapterSource`、`GatewayAdapterArg`、
  `GatewayAdapterCallable`、`GatewayAdapterPlan`、`GatewayEntry`；
- 新 `artifact-identity/src/gateway_adapter.rs`：唯一负责 normalization、preimage、
  `gateway_entry_identity`、surface/identity validation；
- `artifact-model/src/deployment.rs` 只消费该 leaf，不另建一套同形 DTO。

最小 adapter kind 应覆盖 `TypedJson`、`RawHttp`、`WebSocketConnect`、
`WebSocketReceive`。HTTP stream 是 `RawHttp` handler signature 导出的
`Unary`/`ServerStream` mode，不需要再制造一个与 callable stream mode 竞争的 authoring tag。

建议 resolved entry 至少包含：

- exact implementation `PackageArtifactRef`；
- handler `PackageCallableId`；
- adapter kind；
- parameter 到 typed adapter source 的完整映射；
- input/output/context codec plan；
- unary/server-stream mode。

`GatewayEntryIdentity` preimage 应包含上述语义和 schema marker，但不包含 human text。若 selector 与
entry 分表，host/method/path 不进入 entry identity，而由 deployment identity 覆盖：

```text
ServiceDeployment.gatewayEntries:
  GatewayEntryIdentity -> GatewayEntry

ServiceDeployment.ingress:
  IngressSelector -> GatewayEntryIdentity
```

这样同一 adapter entry 可以被多个 selector 复用；修改 selector 只改变 deployment identity，修改
handler/build/adapter/args/codec plan 同时改变 gateway entry 与 deployment identity。

## 非 public source callable 的当前完整度

对最小 ingress handler 域（非泛型、top-level function），现有 PackageArtifact 已有足够事实，不需要先
把 handler 放进 `api.yml`：

1. `project_implementation_symbols` 遍历每个 File IR declaration，选择
   `ExecutableKind::Function`：`compiler/projection/src/package_artifact/callables/mod.rs:133-164`。
2. source path 为 `<module>.<top-level-name>`，stable callable id 为
   `pkg-callable:<package-id>:top-level:<source-path>`：
   `compiler/projection/src/package_artifact/callables/mod.rs:165-172`。
3. 它从 executable parameter/return/may-suspend facts 生成
   `PackageCallableSignature`，并把 local nominal 规范化为 package-owned symbol：
   `compiler/projection/src/package_artifact/callables/mod.rs:173-217`、
   `compiler/projection/src/package_artifact/callables/normalization.rs:50-150`。
4. 它为同一 id 写入 exact `PackageCallableLinkFact.target` 与 semantic facts：
   `compiler/projection/src/package_artifact/callables/mod.rs:218-270`。
5. artifact validation 强制 implementation callable id 唯一，signature type refs 可解析，
   `callable_links` keys 精确等于 public + implementation callable ids，且每个 callable 都有 semantic
   facts：`artifact-identity/src/package_artifact/validation.rs:200-234,303-357`。
6. implementation symbols、callable links、semantic facts 都进入 Package build identity；
   implementation symbols不进入 local public ABI identity：
   `artifact-identity/src/package_artifact/projection.rs:24-31,120-153`。

这里的“implementation link”应使用 canonical `PackageArtifact.callable_links`；非 public function
不会进入只从 public exports 初始化的 `PackageImplementationLinks.functions`
（`compiler/projection/src/package_artifact/callables/surface.rs:39-75`）。

### linked signature 的两个限定

- artifact 内部 self-owned package symbol 的 `abi_expectation` 被规范化为 `None`，消费 exact package
  dependency 时才由 `bind_callable_signature_identity` 注入 local ABI identity：
  `compiler/projection/src/package_artifact/callables/normalization.rs:89-98`、
  `compiler/driver/source_compile/canonical_dependencies.rs:247-315`。这不是类型丢失，但 gateway compiler
  必须在 exact `PackageArtifactRef` 上做同样的 binding，不能按显示名比较。
- `ExecutableIr` 有 `type_params`，但 `PackageCallableSignature` 没有；implementation projection 重建
  signature 时不会保留 callable type parameter declaration：
  `artifact-model/src/executable.rs:40-55` 与
  `artifact-model/src/package_artifact.rs:22-35`。如果“完整 linked signature”要求支持 generic
  ingress handler，首次真实丢失就在
  `project_implementation_symbols` 构造 `PackageCallableSignature` 的
  `compiler/projection/src/package_artifact/callables/mod.rs:173-217`。

最小切换应 fail closed 拒绝 generic handler，复用现有非泛型 signature/link；不要为本次切换顺手扩大
PackageArtifact ABI。若后续明确要求 generic gateway handler，再单独扩展
`PackageCallableSignature` 并 bump PackageArtifact/local-ABI/build identity generations。

## `service.yml` 目标 shape 与严格边界

`ServiceManifestAuthoring.http`/`websocket` 应从 JSON value 改为递归
`deny_unknown_fields` 的 typed authoring DTO。authoring handler 使用 source selector，adapter 部分直接
消费 shared Rust adapter kind/source enum。示意 wire：

```yaml
id: example.com/service

http:
  routes:
    - host: "*"
      method: POST
      path: /typed
      adapter:
        kind: typedJson
        handler: main.handleTyped
        args:
          - param: request
            source: { kind: http.body }
    - host: "*"
      method: GET
      path: /raw
      adapter:
        kind: rawHttp
        handler: main.handleRaw

websocket:
  routes:
    - host: "*"
      path: /socket
      connect:
        kind: websocketConnect
        handler: main.connect
        args:
          - param: request
            source: { kind: websocket.connectRequest }
      receive:
        kind: websocketReceive
        handler: main.receive
        args:
          - param: event
            source: { kind: websocket.receiveEvent }
```

最终字段拼写应由 shared Rust DTO 的 serde wire 一次性决定；compiler input 和后续 artifact consumer
都引用该 owner。不得在 driver 保留另一份 `RouteAuthoring`。

严格边界要求：

- `read_service_manifest` 就完成递归 parse、selector lexical validation 和 adapter phase/source
  compatibility validation；
- 任何层级出现 `operation` 都作为 unknown field 失败；
- 同时出现新 `handler` 与旧 `operation` 也失败，不做优先级或 fallback；
- handler 只解释为 source selector，并从
  `PackageArtifact.package_local_abi.implementation_symbols` 解析；
- 不按 `api.yml` public path fallback，不要求 handler public；
- 如果同一个 source callable 同时显式出现在 `api.yml`，它可以独立成为 service operation，但 ingress
  仍绑定其 implementation callable identity/gateway entry identity，两个 identity 域不得相互推导。

## generation / identity 必要变化

### ServiceContract / ServiceProtocolIdentity

- 不新增 ingress、handler、adapter、route 或 codec 字段。
- `ServiceProtocolIdentityProjection` 继续只含 service id、service operations 与其 package type
  requirements；现有 owner
  `artifact-identity/src/contract.rs:51-60,186-207` 的边界是正确的。
- `project_service_api` 继续只把**显式 `api.yml` service API**投影成 contract。仅为 ingress 而存在的
  handler 从 `api.yml` 移除后，自然不再产生 `ContractOperationId`。
- ingress selector、handler implementation、adapter plan 或 deployment policy 的变化必须不改变
  `ServiceProtocolIdentity`；无需为 external ingress 给 ServiceContract schema/identity bump。
- 不保留当前 WebSocket “stable key 必须为 `websocket`”的 contract admission path；相应 validation
  转到 gateway adapter signature/plan。

### PackageArtifact

- 最小方案不新增 gateway route/entry 字段：route/adapter 是 service/deployment authoring，不是
  reusable package public ABI。
- 复用现有 `implementation_symbols`、`callable_links`、`callable_semantic_facts`，新增一个 compiler
  resolver/validator，把 `SourceSymbolSelector` 精确解析为 implementation
  `PackageCallableId + PackageCallableSignature + OperationTargetRef`。
- adapter/codec compiler 从 package-owned type descriptors生成 resolved plan，并把 plan放进
  ServiceDeployment gateway entry。
- 增加生成与 identity probes，保证 private handler 的 signature/body 变化改变
  `PackageBuildId`，但不改变 `PackageLocalAbiIdentity`；source path rename 改变
  `PackageCallableId`。
- 因现有非泛型 facts 已充分，最小切换无需 bump `PACKAGE_ARTIFACT_SCHEMA_VERSION` 或 package identity
  markers。若选择支持 generic handler，则必须先扩展 signature owner并做独立 generation bump，不能
  静默接受残缺 signature。

### ServiceDeployment

- `DeploymentIngressBinding.contract_operation_id` 改为
  `gateway_entry_identity`。
- `ServiceDeploymentInput`/`ServiceDeployment` 新增 canonical
  `gateway_entries`，其 values 是 shared `GatewayEntry`。
- ingress validation 改为：selector 唯一；entry identity 存在且重算一致；entry implementation 与
  deployment exact implementation ref 一致；handler id、signature、link、semantic facts 与
  PackageArtifact 一致；adapter kind/args/codecs/mode 通过专用 validator。
- operation bindings 仍只覆盖真正的 `ServiceContract.operations`，但与 ingress 集合完全独立。
- 删除 `resolve_route` 的 contract lookup、
  `validate_ingress_contracts` 的 `websocket_ingress_context(contract, operation_id, ...)` 路径。
- deployment identity preimage 同时纳入 normalized gateway entries 与
  selector-to-entry bindings。
- wire 已不兼容，必须一次性 bump：
  `SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION`、`SERVICE_DEPLOYMENT_SCHEMA_VERSION`、
  `DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER` 与
  `DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX`；只接受新 generation，不 dual-read。

### 下游必然变化

`GlobalIngressBinding` 当前仍携带 `contract` 与 `contract_operation_id`：
`artifact-model/src/runtime_assembly.rs:80-109`。deployment 切换后，下游 assembly projection 应改成
`selector + deployment + gateway_entry_identity`，并相应 bump RuntimeAssembly schema/identity。
runtime/router manifest/emission 必须消费 shared resolved entry，不再从 service operation 生成或猜测
gateway adapter metadata。

## 最小 production 与负向 probes

| lane | 最小 production probe | 必须有的负向 probe |
| --- | --- | --- |
| HTTP raw unary | 非 public `main.raw(std.http.HttpRequest) -> std.http.HttpResponse`；不在 `api.yml`；生成 `RawHttp + Unary` entry，handler id/link 精确命中 implementation symbol；contract 无该 operation | 0/2 个参数、错误 request type、错误 response type、generic handler、public-path-only selector、旧 `operation` 均在 compiler/deployment 边界失败 |
| HTTP typed JSON | 非 public `main.typed(Request) -> Response`；args 覆盖每个 formal；private package-owned Request/Response 生成 exact codec plan；contract/ProtocolIdentity 不含这些业务类型 | 缺失/重复/未知 formal、HTTP 不允许的 source kind、不可 codec 类型、owner/local-ABI identity 不匹配、伪造 handler id/entry id 失败 |
| HTTP raw stream | 非 public handler 接受 exact `HttpRequest`，返回 `Stream<HttpResponseStreamEvent>`；entry mode 为 `ServerStream`；selector 与同一 entry 可重放生成稳定 identity | `Stream<bytes>`、unary response 配成 stream、stream event owner/type identity 不正确、缺 exact callable link 失败 |
| WebSocket connect | 非 public connect handler；只允许 connect phase sources；返回 exact `WebSocketConnectResult<Context>`；entry 保存 Context codec/identity | receive-only source、错误返回、nullable/context owner 不一致、connect/receive entry identity 互换、旧 unified service operation 失败 |
| WebSocket receive | 独立非 public receive handler；只允许 receive phase sources；Context 与 connect entry 完全一致；返回允许的 unary completion | connect-only source、Context type/codec 不一致、server stream、未知 handler、缺 receive formal mapping 失败 |

还需四组跨 lane 不变量 probes：

1. 仅修改 host/path/method、handler body或 adapter args，`ServiceProtocolIdentity` 保持不变。
2. 修改 selector 会改变 `DeploymentArtifactIdentity` 但不改变复用的
   `GatewayEntryIdentity`；修改 handler/build/kind/args/codec plan 会同时改变二者。
3. private handler rebuild 改变 `PackageBuildId`，但不改变
   `PackageLocalAbiIdentity`；handler source path rename 改变 `PackageCallableId`。
4. raw YAML corpus 在 compile input 边界穷举拒绝：
   `operation`、`operation + handler`、未知 adapter kind、未知 arg source、协议/phase 不兼容 source、
   duplicate selector 与 unknown nested field。

## 建议后续 DAG

```text
F347-A shared gateway model / identity checkpoint
  artifact-model GatewayEntryIdentity + adapter/entry DTO
  artifact-identity canonical preimage/normalization/validation
  strict wire + identity mutation tests
        |
        +----------------------+----------------------+
        |                      |                      |
F347-B compiler input     F347-C package facts   F347-D deployment model
typed service.yml        source selector ->     input/output schema v2,
legacy operation reject  private callable       entry map + ingress ref,
phase/source validation  exact signature/link   identity v2
        |                      |                      |
        +----------------------+----------------------+
                               |
                    F347-E generator / projection cutover
                    adapter+codec plan compile;
                    no ContractOperationId ingress path;
                    HTTP/WS positive+negative probes
                               |
                    F347-F contract/package regression gate
                    ingress-only handler absent from contract;
                    ProtocolIdentity invariance;
                    PackageArtifact identity invariants
                               |
                    downstream assembly/runtime/router cutover
                    RuntimeAssembly generation/identity bump;
                    shared entry emission/consumption;
                    delete legacy gateway operation path
```

依赖与 ownership 要点：

- `F347-A` 是唯一 shared schema/identity checkpoint，必须先落；
- `F347-B/C/D` 可以在 checkpoint 后按非重叠 production 域并行；
- `F347-E` 同时依赖 B/C/D；
- `F347-F` 只做跨对象不变量与 generation gate；
- 下游 cutover 不得在 compiler/deployment 中留下 legacy DTO、dual write、fallback public path 或
  `operation` compatibility parser。

## 写入与验证记录

- 唯一新增文件：
  `P5-F347-external-ingress-compiler-artifact-audit-result.md`。
- 无 production/test/corpus/lockfile 写入冲突。
- 本任务是只读审计，没有运行测试或 live/stable 命令；提交前仅做 Git 写入边界与文档 diff 校验。
