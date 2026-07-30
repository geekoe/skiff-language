# P5-F333 Wire and observability delta audit result

状态：审计完成（production 只读；未实现、未运行测试）。

## 审计基线与结论

- 审计 HEAD：`28e1039167f3bdc730b65511060b5308a61940c3`
- 审计 tree：`2756510f2e54b3f9b423e87ce1e6c0b764e5b039`
- 直接父结果：F280、F331、F319。
- 审计范围：`runtime/request-contract/**`、`runtime/request/**`、`runtime/transport/**`、
  `runtime/host/**`、`router/**`、`telemetry/**`；只为接缝读取
  `runtime/eval/**`和`runtime/capability-context/**`。
- 已执行任务要求的三组反搜，并按 canonical、可保留 generic、待删除 legacy 与 telemetry
  接缝归类。未运行 cargo、pnpm、完整测试、stable 或 live；本文件是唯一写入。

结论如下。

1. R0–R4 已把 canonical fixed carrier 交到 request 层：
   `RuntimeError::FixedServiceFailure(OpaqueServiceError)`在 eval ingress、WebSocket ingress 和
   `RequestError::Eval`中仍然保持 typed。**第一次实际压平发生在 host assembly request entry 调用
   `RequestError::response_error()`时**；该调用经`WirePayload::payload()`把 fixed carrier 变成
   `InternalError / "canonical service failure" / details=None`。
2. request-contract、Rust transport 和 Router 当前都只有 generic
   `code/message/status/details`的`response.error`，binary payload 被强制为空。已有
   `FixedServiceResponseFailure`/`OutboundResponse::FixedServiceFailure`是正确但未接线的 seam；
   production 搜索没有找到 wire producer/decoder。
3. strict v2 的最小布局是：`response.error`以显式 discriminator 区分 fixed service failure 与
   control/pre-ingress failure；fixed variant 的 binary payload 放**完整且原样的**
   `OpaqueServiceError::encoded_bytes()`，即既定`ServiceErrorEnvelope`的全部 canonical bytes。
   Router 可严格解析得到只读 view，但必须转发原 bytes，不能拆出内部`encodedPayload`再重编码。
4. 完整 local source/stack 目前在
   `CanonicalServiceErrorChannel::export_provider_failure`返回`OpaqueServiceError`时丢失。W2-W
   当前范围内的 host/transport/router 无法事后恢复它；完整 A6 observability 在 fan-out 前需要一个
   eval/capability-context corrective checkpoint。这个缺口不改变 fixed envelope，也不是新增用户设计选择。

## Production 跳点

### Fixed response 主链

| 顺序 | 当前 production owner / 入口 | 当前真实形状与校验 | 遮挡或目标动作 |
| --- | --- | --- | --- |
| 1 | `runtime/model/src/service_error.rs::{ServiceErrorEnvelope,OpaqueServiceError}` | 三种 strict envelope：`PublicTypedError`、`InternalError`、`PlatformError`；deserialize拒绝 unknown field/variant并验证非空 identity、payload、traceId、errorId。`OpaqueServiceError`同时保存 decoded view 与原 bytes | canonical semantic/byte owner，W2-W不得改其字段、bytes或分类规则 |
| 2 | `runtime/eval/src/assembly_execution/service_error_channel.rs::CanonicalServiceErrorChannel::export_provider_failure` | 分类、private/nonclosed/encode failure→固定 InternalError；imported fixed 原样 clone；返回`OpaqueServiceError` | fixed bytes正确；但本地`RequestException::{source,stack,correlation}`在返回时没有 production diagnostic sink |
| 3 | `runtime/eval/src/assembly_execution/{ingress.rs,websocket_ingress.rs}` | canonical ingress failure 以`RuntimeError::FixedServiceFailure(OpaqueServiceError)`冒泡；现有测试证明不含 provider private string | 保持 typed，不应在 eval 构造 HTTP/WS policy |
| 4 | `runtime/request/src/assembly_ingress.rs`与`runtime/request/src/error.rs::RequestError::Eval` | `?`把 eval error保留为`RequestError::Eval(EvalRuntimeError)` | 这是尚未丢失 identity/bytes 的最后一个 request seam |
| 5 | `runtime/host/src/host/request_entry/assembly.rs:160-184` | `request_error.response_error()`先执行；随后 supervisor只收`ResponseError`，transport只收`ResponseEvent::Error(ResponseError)` | **当前 fixed 主链的第一次永久压平** |
| 6 | `runtime/request/src/error.rs::{RequestError::response_error,WirePayload::payload}`→`runtime/eval/src/error.rs::RuntimeError::payload` | fixed变为 generic`InternalError`、固定 message、无 details | fixed consumer必须在调用 generic payload 前显式分支；禁止按 code/message恢复 |
| 7 | `runtime/request-contract/src/response_event.rs::ResponseEvent` | 只有`End`和`Error(ResponseError)` | 增加复用`FixedServiceResponseFailure`的 typed variant；不新增第二个 envelope |
| 8 | `runtime/transport/src/response_mapper.rs::response_event_into_frame` | generic error写进`ResponseErrorFrameHeader.error`，binary payload固定为空 | Rust是 v2 encoder canonical owner；fixed写 exact envelope bytes，control仍写 generic header |
| 9 | `runtime/transport/src/protocol.rs::{ResponseErrorFrameHeader,RuntimeErrorFramePayload}` | `serde(deny_unknown_fields)`拒绝 header/nested extras；但 typed decode只做 serde，不核对`schemaVersion`/`type`值，也不要求非空 code/message或 status 400–599 | v2 decoder必须显式验证 version、type、discriminator、字段集合及 payload presence |
| 10 | `runtime/host/src/host/router_session.rs:550-566`→`runtime/transport/src/response_mapper.rs::response_error_to_outbound` | router→runtime response被要求空 payload，永远生成`OutboundResponse::Error` | fixed variant须 decode为已有`OutboundResponse::FixedServiceFailure`并保留原 bytes；generic仅用于 control/protocol |
| 11 | `router/src/protocol/envelope.ts::{ResponseErrorFrameHeader,RuntimeErrorPayload}` | TS interface镜像 generic header；全局常量仍为`skiff-runtime-frame-v1` | 只镜像 Rust v2 union，不成为 envelope/classifier owner |
| 12 | `router/src/protocol/runtimeProtocol.ts::{responseErrorProperties,validateResponseError}` | declarative schema对 nested error写`additionalProperties:true`；实际 manual validator又拒绝 header/error extras、检查 version、type、字符串类型和 status 400–599。两套描述已经漂移；空字符串仍可过 | 单一 strict parity shape；同一 golden corpus同时约束 Rust 与 TS |
| 13 | `router/src/router/runtimeEndpoint.ts:519-522,644-651` | manual validator admission后仍强制`response.error` payload为空，只把`header.error`交 dispatcher | 按显式 variant检查：service bytes必须非空且 strict；control bytes必须空 |
| 14 | `router/src/router/runtimeDispatcher.ts:647-682` | `unaryFrame`重建 generic header+空 payload；普通 pending变成`RuntimeResponseError` | unaryFrame原样转发 header+bytes；外部 pending按 explicit fixed/control variant分流 |
| 15 | `router/src/router/errors.ts::RuntimeResponseError` | 根据`error.code`/status table重分类；502时还能把完整 runtime error包进 details | class与表保留给 control/gateway error；fixed另走 typed mapper，绝不调用此分类器 |

`UnhandledServiceError`不是当前 fixed carrier 的压平结果。它位于
`runtime/eval/src/error.rs::user_exception_payload`，只为仍落入 legacy
`RuntimeError::UserException` generic fallback 的 request-local异常生成安全 message 与
`traceId/errorId` details。canonical fixed ingress在它之前已是`FixedServiceFailure`，当前实际落入的是上表
第 6 步的`"canonical service failure"` generic fallback。W2-W 应删除 fixed 主链对 generic
`WirePayload`的依赖；`UnhandledServiceError`可暂时保留为非 canonical fail-closed fallback，但不得再作为
service response成功路径的测试预期。

### HTTP/WebSocket 与 telemetry 跳点

| 跳点 | 当前事实 | 缺口 |
| --- | --- | --- |
| Router production gateway | `router/src/router/server.ts:169-188`实际注册`AssemblyHttpGateway`与`AssemblyWebSocketGateway` | 审计外部策略应以这两条路径为准，而不是旧 gateway helper |
| HTTP error write | `AssemblyHttpGateway.writeError`调用`GatewayError.toPayload()`，把 details原样写入`{"error":...}` | `GatewayError.toHttpBody()`虽会隐藏 5xx detail，但 production assembly gateway没有调用它；当前不存在可靠的 5xx detail redaction boundary |
| WebSocket upgrade/close | `assemblyWebSocketGateway.ts::{writeUpgradeFailure,websocketCloseReason}`直接发送/截断`error.message` | 当前无 structured fixed mapping或 correlation body/reason；provider message若到达这里会泄露 |
| ingress trace owner | HTTP gateway和WebSocket gateway各生成`traceId/spanId`；`assembly_wire.rs`把 trace放进`RequestEnvelope.extra` | trace事实已存在，不应另造第二个 trace |
| eval errorId owner | request context把`extra.trace.traceId`传给 invocation；`ProgramExecutionContext::next_exception_correlation`生成`${traceId}:local-error:${sequence}` | errorId已在 fixed envelope内，但没有进入 top-level telemetry DTO |
| host request telemetry | `RequestTelemetryContext`已有 trace/span slots；`emit_trace`会复制它们 | `assembly_request_telemetry_context`没有从 request extra应用 trace/span；`RequestTraceFields`目前只在 control-plane route-error使用 |
| host error telemetry | `request_supervisor::complete_error`只收`ResponseError`，再用`response_error_to_telemetry_map`复制 code/message/details | external response DTO与 diagnostic DTO耦合：保留 stack会泄露，先脱敏又丢 local stack |
| telemetry protocol | Rust`runtime/transport::protocol::TelemetryEvent`、Router TS mirror、telemetry service TS mirror均有 trace/span，但都没有 top-level errorId或 restricted discriminator | 三镜像必须由同一 checkpoint原子冻结；Rust DTO当前也没有`deny_unknown_fields` |
| telemetry admission/storage | `telemetry/src/protocol.ts`严格拒绝未列 event字段；server验证后，Mongo/in-memory store存`redactTelemetryEvent(event)` | 未加字段前 errorId/restricted event会被拒；存储没有 visibility/access split |
| telemetry query | `/logs`、`/traces`、`/traces/:id`返回匹配 event | restricted local stack当前没有 fail-closed过滤；不能把新增 restricted event直接混入现有可见结果 |
| redaction | host producer做大小限制/敏感 key遮罩；telemetry storage再次按敏感 key、深度、长度、数量redact | 可复用为 defense in depth；response redaction必须更早且独立，restricted stack结构/source可保留，secret value仍应遮罩 |

## Canonical、duplicate 与 legacy owner

### Canonical / 可直接复用

| 语义 | 唯一 owner | 约束 |
| --- | --- | --- |
| service error分类、identity、correlation和 canonical bytes | `runtime/model/src/service_error.rs`及`runtime/eval/src/assembly_execution/service_error_channel.rs` | W2-W只消费；不得复制 public/private、SchemaClosed、platform/Internal分类 |
| request-local source/full stack | `RequestException`及 eval `ProgramExecutionContext`/canonical export call site | 只可流入本 service的 restricted diagnostic；不得进入 fixed envelope、runtime frame或 gateway body |
| typed response carrier | 已有`runtime/capability-context/src/response.rs::FixedServiceResponseFailure`与`outbound_response.rs::OutboundResponse::FixedServiceFailure` | request-contract复用同一类型；不要创建“transport service error”平行 envelope |
| response.error v2 encode/decode | `runtime/transport/src/{protocol.rs,response_mapper.rs}` | Rust写/读 canonical frame；TS只 strict parity与 opaque forward |
| request trace | Router ingress创建；request wire/extra携带；host从同一 request读取 | 不允许 host/eval重新生成 trace |
| errorId | `ServiceErrorEnvelope::{trace_id,error_id}`与 eval correlation generator | telemetry从 typed carrier/diagnostic复制，不从 message/details解析 |
| telemetry shared shape | Rust`TelemetryEvent`为 producer DTO owner；Router/telemetry TS是严格镜像 | 一次 checkpoint、共享 goldens；消费者不得各加临时字段 |

### 重复/漂移面

- `router/src/protocol/envelope.ts` interface、
  `runtimeProtocol.ts::runtimeFrameHeaderSchemas` declarative schema和同文件 manual validator是三份
  `response.error`描述；当前 nested `additionalProperties`已互相矛盾。
- `TelemetryEvent`分别手写在 Rust transport、Router TS protocol、telemetry TS protocol；三者同时缺
  errorId/restricted discriminator。
- `RUNTIME_FRAME_SCHEMA_VERSION`在 Rust/TS手工镜像。response.error v2 version必须在一个 shared
  checkpoint原子更新，并由 cross-language corpus证明；不能由 host/router consumer各自改。
- request trace读取同时存在于`runtime/request/src/context.rs`和
  `runtime/host/src/host/request_trace.rs`；两者读取同一`extra.trace`事实。host assembly telemetry应复用
  `RequestTraceFields`，不是再写第三套 JSON path。

### 必须保留的 generic DTO

以下 generic surface不属于 service fixed response，不应全局删除或替换：

- `RuntimeErrorPayload`用于 actor/spawn/control/bootstrap/gateway/pre-ingress decode/route failures；
- `ResponseError`和`OutboundResponse::Error`用于 timeout、cancel、protocol、unsupported transport以及
  真正 control response；
- Router`GatewayError`、`RuntimeResponseError`及 code→HTTP status table用于上述 control/gateway
  surface；
- Router对 runtime-originated legacy`request.start`返回的
  `InProcessServiceCallRequired`仍是 control/protocol rejection，不是假造 fixed service envelope；
- request/host的 route、assembly-wire decode、generation/cancel等在真正 service execution前失败时，
  仍可生成 generic error。

### Fixed 主链必须删除或绕开的 legacy

- `RequestError::response_error()`对`RequestError::Eval(FixedServiceFailure)`的 generic flatten；
- fixed进入`RuntimeError::payload()`的`"canonical service failure"`fallback；
- request-contract只有`ResponseEvent::Error(ResponseError)`的出口；
- response mapper把所有`response.error`写成 generic header+空 payload；
- `response_error_to_outbound`永远生成`OutboundResponse::Error`；
- host/router session与Router endpoint对所有`response.error`强制空 payload；
- dispatcher把 fixed交给`RuntimeResponseError`并按 code/message/status分类；
- supervisor从 external`ResponseError`构造 telemetry diagnostic；
- fixed external mapping经过`GatewayError.toPayload()`或 raw WebSocket`error.message`。

删除的含义是 fixed variant不再经过这些路径；generic control调用仍保留。任何实现都不能用
`code == InternalError`、`message == canonical service failure`、HTTP status或 JSON shape把 generic
error“升级”为 fixed。

## Strict wire 与 restricted telemetry 目标接线

### response.error v2

建议在 shared protocol checkpoint冻结如下最小逻辑形状。字段拼写由该 checkpoint一次确定；以下使用
`errorKind`说明 discriminator，不构成新的 semantic owner。

```text
Fixed service response
  header = {
    schemaVersion: "skiff-runtime-frame-v2",
    type: "response.error",
    requestId,
    errorKind: "fixedService"
  }
  binary payload = exact OpaqueServiceError.encoded_bytes()

Control / pre-ingress response
  header = {
    schemaVersion: "skiff-runtime-frame-v2",
    type: "response.error",
    requestId,
    errorKind: "control",
    error: { code, message, status?, details? }
  }
  binary payload = empty
```

这里的 v2 是`response.error` schema版本；binary container magic/version无需改变，无关 actor/control
frame也无需为本任务重写语义。validator按`type == response.error`要求 v2，明确拒绝 v1 generic
service frame。Skiff尚未发布，不增加 dual read/write或 v1 adapter。

选择完整`ServiceErrorEnvelope`bytes作为 fixed binary payload有四个直接收益：

1. 复用唯一 Rust serde/validation owner，不在 frame里再定义 public/platform/internal字段；
2. imported/unlinked middle service可以 byte-for-byte forwarding；
3. Router可以 strict parse得到 envelope kind/traceId/errorId view，同时保留原 bytes继续转发；
4. 不拆出内部`encodedPayload`，避免 decode/re-encode改变 opaque bytes或引入第二分类器。

Rust encoder/decoder和TS validator均须检查：

- exact`schemaVersion`、`type`、`errorKind`与 requestId非空；
- variant exact field set；unknown variant、extra/missing field失败；
- fixed payload必须非空、control payload必须为空；
- fixed payload必须是 strict`ServiceErrorEnvelope`；unknown kind/platform enum、额外字段、空
  owner/key/type/payload/correlation失败；
- control generic payload保持其自身 code/message/status约束，但永远不能按值转成 fixed；
- frame decode后保留原 payload bytes；TS/Rust转发路径均不得 stringify已解析 envelope。

目标接线如下。

```text
Eval FixedServiceFailure(OpaqueServiceError)
        │ typed extraction（不得调用 WirePayload）
        ▼
Request fixed result ──► Host ResponseEvent::FixedServiceFailure
        │                         │
        │ restricted sidecar      ▼
        │                  Rust response.error v2 encoder
        │                         │ exact envelope bytes
        │                         ▼
        └─► local telemetry   Router strict validator
                                  ├─ service-to-service unaryFrame:
                                  │    exact header+bytes forward
                                  └─ external HTTP/WebSocket:
                                       explicit fixed mapper + redaction
```

Router fixed mapper只按 frame/envelope显式 discriminator dispatch。它不得重新判断 public/private、
SchemaClosed、platform allowlist或何时生成 InternalError；这些都已由 Rust envelope owner决定。当前没有
fixed external mapper，最小安全策略是：

- InternalError只暴露稳定通用信息与允许的 correlation，不暴露内部 message以外的 diagnostic；
- public/platform payload只有在既定 gateway adapter明确允许、且由 strict envelope/schema view解码时
  才可映射；否则 fail closed为脱敏 5xx；
- HTTP body与WebSocket upgrade body/close reason不得直接使用 provider exception display/message；
- fixed不得进入`runtimeErrorStatus(error.code)`；control error继续使用该表。

这不要求本审计发明新的公共 HTTP status/body schema；consumer节点应沿既有 gateway policy做最小安全
映射，并用 W1 probes冻结外部不泄露和 correlation。若后续产品要公开新的 typed HTTP error contract，
那是独立公共 API任务，不应阻塞 fixed wire。

### restricted telemetry

目标必须把一份失败拆成两个不同可见性事件，而不是复用 external response DTO。

| 事件 | 必需字段 | 允许内容 | 禁止内容/可见性 |
| --- | --- | --- | --- |
| operational request/error trace | service/build/activation/runtime/request/target、top-level traceId、spanId/parentSpanId、top-level errorId、duration、safe fixed/control kind | 稳定 safe code/name、预算与 correlation | 不含完整 stack、source path/function、private payload；可进入现有 operational查询 |
| restricted local service diagnostic | 显式有限 discriminator/visibility、service/activation/request/operation、top-level traceId/errorId、当前 service的 source与完整 local stack、safe cause kind | `RequestException`现有 Local/RemoteBoundary stack结构；每 service hop一份 | 不进入 response/frame/gateway；现有普通`/logs`、`/traces`、`/traces/:id`必须默认排除，只有显式内部受限读取面可见 |

共享 telemetry checkpoint要原子增加：

- Rust、Router TS、telemetry TS三份 top-level`errorId`；
- 一个有限值 restricted/operational discriminator（具体字段名属于实现布局）；
- strict allowed-field与值校验；
- storage document/index/query filter所需的同一字段语义。

host assembly entry必须把`RequestTraceFields::from_request(request)`应用到
`RequestTelemetryContext`；fixed envelope/diagnostic的 errorId直接复制到 event top level。
`request_supervisor::complete_error`应按 typed result分别发 operational safe trace与 restricted
diagnostic，不能再用`response_error_to_telemetry_map`承担 fixed error。

redaction边界分三层：

1. eval导出时固定 envelope已删除 private/nonclosed/provider diagnostic；
2. host telemetry producer对 restricted event做大小上限和 secret-key遮罩，但保留有用的本地 stack结构；
3. telemetry ingress/storage再次严格校验与 redact；普通 query fail closed过滤 restricted。

第二、三层不能成为 external response的素材来源，外部 redaction必须在 gateway fixed mapper处独立完成。

## 建议 DAG 与互斥写入范围

完整 A6不能直接把 W2-W拆成多个并行 agent：当前还缺 eval local diagnostic handoff，而 frame 与 telemetry
DTO又是共享文件。最少 checkpoint、最大安全 fan-out如下。

```text
RΔ  eval restricted-diagnostic handoff（corrective prerequisite）
 │
 └────── blocked-by：F331 fixed carrier/stack事实
          │
C0  shared response.error v2 + telemetry parity（单一串行 owner）
          │
          ├──────── H  request/host/session consumer
          ├──────── R  router dispatcher/gateway consumer
          └──────── T  telemetry storage/query consumer
                         │
                         └──── C1  W1/S1 cross-layer test-only convergence
```

RΔ与C0可以由 integration owner在确认 typed diagnostic handoff contract后顺序落地；若先冻结一个最小
typed callback/sidecar签名，C0的 wire部分可并行准备，但在 fan-out前两者都必须已合并。为避免 API形状在
H/T期间反复改变，推荐实际集成顺序为 RΔ→C0→H/R/T。

### RΔ：eval restricted-diagnostic handoff，串行 prerequisite

- blocked-by：F331/R0–R4已冻结的`RequestException`、per-hop stack与
  `OpaqueServiceError` exact bytes。
- production写入范围：
  - `runtime/capability-context/src/{telemetry.rs,lib.rs}`：一个 typed internal diagnostic
    callback/sink；不得改 service envelope；
  - `runtime/eval/src/assembly_execution/{service_error_channel.rs,ordinary.rs,async_stream_cancel.rs}`；
  - `runtime/eval/src/{eval_context.rs,program_execution.rs}`仅在 test-effect/上下文接线确有需要时。
- test写入范围：上述文件的 co-located tests，尤其把
  `ordinary.rs::record_ordinary_provider_failure`现有`#[cfg(test)]` spy所证明的
  source/stack事实提升为 production sink probe，并补 async/stream/cancel/forwarded fixed。
- 交付：每个 export hop在丢弃 provider heap前发送一份 typed restricted diagnostic；返回的
  `OpaqueServiceError::encoded_bytes()`完全不变。
- 禁止：修改`runtime/model/src/service_error.rs`、frame/TelemetryEvent DTO、host/router；若在上述范围内
  无法形成 typed handoff，停止并由 integration owner重新分配，不能退回 generic JSON callback。

这是 F280 已要求“channel owner记录完整本地栈”的实现 API缺口，不是 W2-W自行重开 semantic design。

### C0：shared wire/telemetry checkpoint，唯一串行 owner

- blocked-by：RΔ的 typed diagnostic handoff签名；F331 fixed carrier API。
- production写入范围：
  - `runtime/request-contract/src/{response_event.rs,lib.rs}`；
  - `runtime/transport/src/{protocol.rs,response_mapper.rs,lib.rs}`；
  - `router/src/protocol/{envelope.ts,runtimeProtocol.ts}`；
  - `telemetry/src/protocol.ts`。
- test写入范围：
  - `runtime/transport/src/{protocol/tests.rs,response_mapper/tests.rs}`；
  - `router/tests/protocol.test.ts`中纯 validator parity；
  - `telemetry/tests/protocol.test.ts`；
  - 新增且只由C0拥有的
    `runtime/transport/testdata/service-error-response-v2*.json` shared corpus。
- 交付：request-contract typed variant、v2 Rust canonical encode/decode、TS strict union、telemetry
  errorId/restricted discriminator和 Rust/TS共同 goldens。
- 互斥：H/R/T不得同时修改上述任何 production/testdata文件；C0不得改 consumer行为。

### H：request/host/session consumer，可与 R/T 并行

- blocked-by：RΔ、C0。
- production写入范围：
  - `runtime/request/src/{error.rs,assembly_ingress.rs,runner.rs}`中的最小 fixed extraction与 legacy
    telemetry helper隔离；
  - `runtime/host/src/host/{request_entry/assembly.rs,request_supervisor.rs,request_trace.rs,router_session.rs}`；
  - `runtime/host/src/{telemetry.rs,capability_context/telemetry.rs}`以及为 typed callback转接所需的
    `capability_context/native_projection.rs`最小相邻实现。
- test写入范围：
  - request co-located fixed extraction tests；
  - `runtime/host/src/host/router_session/tests{.rs,/**}`；
  - `runtime/host/src/host/telemetry/tests.rs`；
  - assembly request entry现有 focused tests。
- 交付：fixed不再调用 generic payload；supervisor分离 operational/restricted event；assembly trace
  传播；双向 session把 v2 fixed映射成 typed carrier且 exact bytes不变。
- 不触碰：request-contract、transport protocol/mapper、Router、telemetry TS。

### R：Router consumer与外部 mapper，可与 H/T 并行

- blocked-by：C0。
- production写入范围：
  - `router/src/router/{runtimeEndpoint.ts,runtimeDispatcher.ts,errors.ts,assemblyHttpGateway.ts}`；
  - `router/src/gateway/assemblyWebSocketGateway.ts`。
- test写入范围：
  - `router/tests/{runtime-protocol-websocket-response.test.ts,runtime-assembly-unary-dispatch.test.ts,assembly-runtime-endpoint.test.ts}`；
  - `router/tests/{assembly-http-gateway-stream.test.ts,assembly-websocket-gateway.test.ts,runtime-errors.test.ts}`中相关 focused cases。
- 交付：payload presence enforcement、unaryFrame原 bytes forwarding、fixed/control显式 dispatch、HTTP/WS
  fixed redaction与 correlation；generic control classifier原样保留。
- 不触碰：`router/src/protocol/**`、shared corpus、runtime/telemetry service。

### T：telemetry storage/forward/query consumer，可与 H/R 并行

- blocked-by：RΔ、C0。
- production写入范围：
  - `telemetry/src/{server.ts,mongoStore.ts,redaction.ts,queryApi.ts}`；
  - `router/src/telemetry/producer.ts`仅在新增字段的 queue/forward保持确需显式处理时；当前结构通常可透明
    forward，不应无因改动。
- test写入范围：
  - `telemetry/tests/{server.test.ts,store.test.ts,redaction.test.ts,queryApi.test.ts}`；
  - `router/tests/http-telemetry.test.ts`仅作 forward字段保持 probe。
- 交付：strict admission、errorId/visibility原样存储、secret redaction、restricted默认不可见、operational
  trace可按 traceId/errorId关联。
- 不触碰：`telemetry/src/protocol.ts`、Router runtime protocol/gateway、Rust host/transport。

### C1：cross-layer convergence，test-only

- blocked-by：H、R、T全部集成。
- production写入范围：无。
- test写入范围：新建一个专用 W1/S1 integration fixture/test目录；不得回写 C0/H/R/T co-located
  production owner。
- 交付：真实 eval fixed→host→Rust frame→Router→HTTP/WS与 A→B→C per-hop telemetry证据。
- 禁止：为让测试通过修改 frozen DTO、添加 v1兼容或按 code/message分类。

## W1 最小正负探针与外部泄露反搜

### 正向最小探针

| ID | 证明 |
| --- | --- |
| W1-P1 | 三种`ServiceErrorEnvelope`各自从 eval fixed进入 request/host；host在 typed branch前后比较`encoded_bytes()`完全相等 |
| W1-P2 | Rust v2 encoder→decoder round trip保持 header discriminator与完整 binary payload byte-equal；不 stringify envelope |
| W1-P3 | 同一 shared corpus由 Rust decoder与TS validator消费；TS得到相同 kind/identity/traceId/errorId view，同时保留原 bytes |
| W1-P4 | Router `unaryFrame` fixed响应→Rust `router_session`→`OutboundResponse::FixedServiceFailure`；跨Router的 bytes完全相等 |
| W1-P5 | 一个真实 control/pre-ingress failure仍走 generic control variant+空 payload；相同 code/message的伪造 frame绝不变 fixed |
| W1-P6 | fixed到 production Assembly HTTP/WS gateway；按显式 variant映射，外部仅含 policy允许的 safe body/reason与 correlation |
| W1-P7 | gateway生成的 traceId/spanId到达 host operational event；top-level errorId等于 fixed envelope errorId |
| S1-P1 | A初始失败，B未处理，C未处理：A/B/C各一份 restricted event与各自 local stack；三跳 envelope bytes和 traceId/errorId保持一致 |
| S1-P2 | operational query可按 traceId/errorId关联且无 full stack；普通 query看不到 restricted event，内部受限 reader看到经 redaction的完整本地 stack结构 |

### 负向最小探针

| ID | 必须拒绝/不得发生 |
| --- | --- |
| W1-N1 | `skiff-runtime-frame-v1` generic service`response.error`，包括旧`code/message/details`形状 |
| W1-N2 | unknown/missing`errorKind`、extra header/error/envelope field、fixed空 payload、control非空 payload、fixed variant携带 generic error、control缺 generic error |
| W1-N3 | malformed JSON envelope、unknown envelope kind/platform enum、空/有外围空白的 owner/key/type/trace/error id、空 encoded payload |
| W1-N4 | Rust/TS任一 consumer decode后重编码 opaque envelope；用 message/code/status/shape重分类 fixed |
| W1-N5 | runtime frame、HTTP JSON、WebSocket upgrade body/close reason出现 callee path、function、sourceId、sourceFrame/sourceFrames、frames、stack或 private sentinel |
| W1-N6 | fixed service response进入`response_error_to_telemetry_map`、`RuntimeResponseError`或`runtimeErrorStatus` |
| S1-N1 | restricted event缺 traceId/errorId/visibility、被普通 query返回、或 secret value未被 producer/storage redaction |
| S1-N2 | middle service重新生成 InternalError/correlation、复用 callee exception stack、或同一 hop重复发 restricted event |

外部 leakage probe至少使用不可误判 sentinel，例如
`provider-private-secret`、`/callee/private/source.skiff`、`calleePrivateFunction`，同时检查 raw binary
frame、HTTP response bytes、WebSocket upgrade bytes与 close reason；不能只对 parsed HTTP detail做断言。

### 收敛后反搜

反搜结果不能简单要求零命中，因为 generic control DTO必须保留；必须逐个证明 fixed service路径没有命中。

```bash
rg -n \
  'RuntimeErrorPayload|ResponseError|UnhandledServiceError|runtimeErrorStatus|response_error_to_telemetry_map' \
  runtime/request-contract runtime/request runtime/transport runtime/host router telemetry

rg -n \
  'FixedServiceFailure|FixedServiceResponseFailure|OpaqueServiceError|ServiceErrorEnvelope' \
  runtime/request-contract runtime/request runtime/transport runtime/host router telemetry

rg -n \
  'sourceId|sourceFrame|sourceFrames|frames|stack|function|module_path|symbol_path|provider-private-secret' \
  runtime/transport router/src/router router/src/gateway
```

第一组允许 control/pre-ingress owner命中，但 fixed producer/consumer不得依赖这些值；第二组必须形成
eval→request→host→transport→router→reverse outbound连续链；第三组 production external serializer应无
callee diagnostic命中，测试只应包含 negative sentinel/assertion。

## 证据失效边界

| 变更 | 失效证据 | 不应连带推翻 |
| --- | --- | --- |
| `ServiceErrorEnvelope`字段、validation、canonical JSON或`OpaqueServiceError`bytes变化 | F331/R0及全部 wire/golden/exact-forward探针 | 不允许由W2-W直接修改；退回 shared model owner |
| response.error version/header/discriminator/payload布局变化 | C0、H/R和W1 cross-layer wire证据 | eval local exception/channel语义 |
| `RequestError` typed extraction、`ResponseEvent`或host branching变化 | H与end-to-end response证据 | TS isolated validator和telemetry store |
| Router endpoint/dispatcher/gateway fixed policy变化 | R、HTTP/WS redaction与unary forward证据 | Rust fixed envelope与host telemetry |
| `TelemetryEvent` errorId/visibility shape变化 | C0 telemetry parity、H/T、S1关联/查询证据 | response.error exact bytes |
| eval diagnostic sink/export timing变化 | RΔ与S1每跳 local stack证据 | fixed envelope byte证据，前提是 envelope返回值不变 |
| telemetry redact/store/query filter变化 | T与restricted leakage/access证据 | wire、gateway external response |
| trace extra路径或Router ingress trace shape变化 | H trace propagation与W1-P7/S1 correlation | fixed分类与transport framing |

每个 consumer节点只重跑自己和直接下游证据；只有 shared DTO/bytes owner变化才使整个 C0→C1链失效。

## 设计缺口

**无新增用户决策。**

F280已经明确 fixed envelope的物理放置、error id实现和 restricted telemetry内部字段组织属于实现布局。
本审计在既定约束内推荐“完整`ServiceErrorEnvelope` exact bytes放 binary payload、header只放显式
fixed/control discriminator”，它最小化 duplicate schema并直接满足 opaque forwarding，不需要用户选择。

当前有三个真实但均属 implementation/API ownership 的缺口：

1. **eval restricted diagnostic handoff缺失。**完整 local source/stack只存在于
   `RequestException`与 export call site；ordinary lane甚至已有仅测试可见的
   `record_ordinary_provider_failure` spy，但 production没有 sink，async/stream也没有等价 emission。
   必须先做 RΔ；W2-W host/telemetry不能从 fixed envelope重建栈。
2. **production gateway脱敏接错方法。**Assembly HTTP gateway调用`toPayload()`而不是会隐藏 5xx details的
   `toHttpBody()`；WebSocket直接使用 raw`error.message`。这不是公共 policy选择，而是 fixed mapper和
   safe serializer缺失。
3. **telemetry没有 restricted access marker。**三份 DTO都缺 top-level errorId/visibility，storage/query
   也没有默认隔离。字段具体命名由C0冻结即可；“restricted event不得进入普通外部/operational响应”已经由
   父设计确定。

另有一个应在 C0 顺手消除的工程漂移：Router declarative schema与manual validator对 nested
`additionalProperties`意见相反。它不需要用户裁决；Rust canonical DTO、TS strict parity与同一 corpus应成为
唯一答案。
