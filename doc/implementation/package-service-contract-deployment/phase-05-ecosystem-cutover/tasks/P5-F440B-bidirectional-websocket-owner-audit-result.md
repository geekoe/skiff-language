# P5-F440B 双向 WebSocket JSON-RPC owner 审计结果

状态：`PASS — TASK_EXECUTABLE`。

本结果只冻结 owner、公开/内部 wire、双向状态机、实现 DAG 与验证入口，不包含实现。审计基线为：

- commit：`a5fdcbd712dbcd30f6a421ee48b6b2876f970e36`
- tree：`33911300aa666a610f6ed82087682efe1153fe97`
- branch：`codex/p5-f440b-bidirectional-websocket-audit`

权威输入只使用本任务、直接父节点及父节点的直接引用。F439C 的旧 outbound-only 协议结论没有被恢复；
其现有 owner 事实只在未被 F440 覆盖处继续使用。

## 1. 结论与停止条件

三个停止条件均未触发：

1. 现有 Router owner 不要求 broker 核心解析业务 JSON。JSON-RPC 控制字段可以由独立
   `jsonrpc-2.0-text` profile adapter 解析，broker 只接收 opaque method、id 与 payload token。
2. 现有 pending owner 不要求两个方向共享 map。Router `RuntimeDispatcher.pending`、runtime
   `OutboundRequestRegistry` 和新增的 WebSocket 双向状态都可以保持独立。
3. 公开 API 和 wire 已能闭合：公开 std callable/error、authoring/artifact、内部 frame families、classifier、
   terminal outcome 与 generation pin 都在本文冻结。

因此实现可以继续。开始 production 实现前必须先合入父 DAG 要求的两个 checkpoint：

- F440A 产出的 external manifest shared checkpoint；
- F440C 之后完成的 F439A required cancellation checkpoint，保证取消不可捕获且不生成普通
  `response.error`。

## 2. 当前 `connection.send` 全链

当前四个 raw send 是完整、可工作的 best-effort 非挂起链，不应被 RPC 改造替换：

```text
std/websocket.skiff
  sendTextToConnection / sendBinaryToConnection
  sendTextToBusinessIdentity / sendBinaryToBusinessIdentity
        |
        v
std/api.yml
        |
        v
artifact-model/src/native_signature.rs
  exact NativeSignatureDef
  NativeCallableSemantics { may_suspend: false }
        |
        v
compiler exact std import / callable effects / lowering
        |
        v
runtime/eval/src/eval_context.rs
  native_call_suspends(binding_key) == false
        |
        v
runtime/native/src/dispatch/websocket.rs
  coerce args, invoke NativeWebsocketCapability, return null/void
        |
        v
runtime/host/src/capability_context/websocket.rs
  validate target + resolve activation WebSocketEntryId
  queue OutboundControlMessage::ConnectionSend on unbounded writer
        |
        v
runtime/transport/src/control_mapper.rs
  typed binary connection.send header + opaque payload
        |
        v
runtime/host/src/host/router_session.rs
  writer loop -> runtime WebSocket
        |
        v
router/src/router/runtimeEndpoint.ts
  decode header, validate sender/service and text UTF-8
        |
        v
router/src/gateway/webSocketGateway.ts
  exact service/entry/generation/runtime-owner validation
        |
        v
router/src/gateway/webSocketConnectionLifecycle.ts
  admitted/open/slow-client checks -> socket.send
```

关键 owner 事实：

- `runtime/host/src/capability_context/websocket.rs` 只把消息提交给 unbounded Router writer；调用不等待
  Router、socket callback 或客户端消费。
- `router/src/gateway/webSocketGateway.ts` 的 direct target 校验现有 connection 保存的 service、
  WebSocket entry、assembly generation、replica/receipt；business target 使用当前 generation 的
  `(serviceId, websocketEntryId, businessIdentity)` 索引。
- `WebSocketConnectionLifecycle` 在 Router 侧处理关闭、慢客户端预算与 `socket.send` 异常。这些结果不会
  回溯成原 raw send 的 await/backpressure。

以下现有证据必须原样保留，并在新增 request callable 后同时证明“旧四个 false、新 callable true”：

| 层 | 当前锁定证据 |
| --- | --- |
| std/public ABI | `compiler/tests/websocket_ingress.rs::compiler_published_std_keeps_only_connect_shapes_and_exact_send_signatures` |
| source effects | `compiler/source/src/callable_effects/tests.rs::missing_dynamic_mutable_and_capability_semantics_remain_fail_closed` |
| eval suspension | `runtime/eval/src/actor_executor.rs::suspension_probe_matrix_matches_only_real_async_native_paths` |
| raw send segment | `runtime/eval/src/actor_executor.rs::connection_send_stays_inside_the_current_synchronous_segment` |
| Rust transport | `runtime/transport/src/control_mapper.rs` 与 `runtime/transport/src/protocol/tests.rs` 的 `connection_send_*` |
| Router trust/delivery | `router/tests/runtime-endpoint-connection-send-trust.test.ts`、`websocket-gateway.test.ts`、`runtime-registry-dispatch.test.ts`、`protocol.test.ts` |

`requestJsonToConnection` 是新增挂起 operation；不得让 `sendText*` / `sendBinary*` 经过其 registry、deadline
或 response path。

## 3. 可复用机制与禁止共享的 owner

| 能力 | 现有 owner/原语 | 可复用范围 | 禁止事项 |
| --- | --- | --- | --- |
| native 真挂起 | `runtime/native/src/dispatch/adapter.rs` 的 async dispatch；`runtime/eval/src/eval_context.rs` 的 `native_call_suspends` 和 actor segment suspend/resume | 新 callable 标记 `may_suspend=true`，WebSocket capability 返回 `NativeCapabilityFuture` | 不得修改 raw send 的 false 语义 |
| runtime request lease | `runtime/capability-context/src/outbound_response.rs` 的 registry、lease、terminal CAS/Notify、cancel-on-drop | 抽取/复制 lease 机制，建立独立 `ConnectionRequestRegistry` | 不得把 connection response 放进 service-call `OutboundRequestRegistry.pending` |
| deadline/cancel select | `runtime/host/src/capability_context/outbound_service.rs::receive_response` | 复用 biased terminal 思路、effective deadline 与 cancel-on-drop | ancestor cancel 不能投影为普通 WebSocket error |
| typed JSON codec | `runtime/native/src/dispatch/json.rs`、`RuntimeBoundaryContract`、调用点 `RuntimeTypePlan` | outbound 以 `TRequest` encode、`TResponse` decode；inbound 以 linked handler plan decode/encode | Router 不持有 `RuntimeTypePlan`、不按 schema 解业务值 |
| gateway dispatch | `router/src/router/runtimeDispatcher.ts` 的 exact runtime socket、timeout、AbortSignal、detach-before-cancel | 新增 sibling `dispatchAssemblyWebSocketJsonRpc` | broker 不能访问或合并 `RuntimeDispatcher.pending`；只保存 peer id 到 dispatcher correlation 的窄映射 |
| runtime ingress target | `runtime/request/src/websocket_connect_target.rs`、`runtime/eval/src/runtime_http_gateway.rs`、Host request-entry/loader admission | 新增独立 JSON-RPC target/executor/adapter，复用 exact linked callable/plan 校验 | 不得把 connect target 或旧 raw receive target改造成多态 handler |
| generation pin | Router `Connection` 的 assembly generation/可选 receipt；runtime `WebSocketGenerationRegistry` 的 immutable `ActiveAssemblyRoute` | 每个 socket 捕获 method table；有 connect handler **或** declared method 的 socket 在 upgrade 阶段建立 exact runtime pin，runtime 从同一 pinned candidate 解析 method route | 不得在首个 message 到达时懒选 runtime，也不得回查 current snapshot/current activation |
| socket/index | `WebSocketConnectionLifecycle` 与 `AssemblyWebSocketGateway.Connection` | 连接 attach/close 时安装/清除 broker generation | broker 不另建第二套物理连接 owner |
| completion | runtime lease CAS；Router `RuntimeDispatcher.finishPending` 的 detach-first | 每层自己的 map 最多 terminal 一次 | 不得把“dispatcher 完成”直接等同为“socket 已写回” |

当前 `runtime/host/src/host/websocket_generation.rs` 的 `WebSocketGenerationPin` 已保存 immutable
`ActiveAssemblyRoute`，但 `pinned_route` 只服务过时的 receive 命名。实现时应把它收窄/改名为从已 pin 的
candidate 解析 JSON-RPC method route；不能重新做 artifact I/O。

现有 gateway 对无 connect handler 的 socket 直接放行，因而没有 runtime receipt/pin。这对 path-only raw
send entry 可以保持，但对带 declared method 的 connection 不足：replacement 后无法证明旧 method 仍由
哪一个 runtime generation 执行。冻结的闭环是：

1. Router 在 upgrade 时先冻结 method table；只要 connect handler 存在或 method table 非空，就选择 exact
   assembly runtime、登记 generation expectation，并走现有 physical WebSocket connect admission；
2. connect handler 缺席时，Host 不调用用户代码，而是在验证 compiler-owned physical entry 后合成默认
   accept；它仍通过现有 acquire/ack lifecycle 建立 pin 并返回 dispatcher receipt；
3. gateway 在 attach socket 前要求 acquire 已确认，保存 receipt/replica；close 时按“有 generation
   owner”而不是“有 connect handler”释放；
4. path-only、无 connect、无 method 的 entry 仍可无 runtime 打开，receipt/replica 为 absent，且它没有
   inbound dispatch surface。

这不是隐式 connect callback，也不增加公开/wire frame family；它只是把现有 connect admission 同时作为
method-bearing connection 的 eager pin handshake。禁止等首个 peer request 才选 runtime，否则
replacement、同时到达的首批 request 和 close 会形成未定义 owner race。

## 4. Shared authoring / artifact checkpoint

### 4.1 Authoring

共享 checkpoint 消费 F440A 的三文件 parser，并冻结：

```yaml
# websocket.yml
path: /ws

connect:
  handler: websocket.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }

jsonRpc:
  getStatus:
    method: status.get
    handler: websocket.getStatus
    adapterArgs:
      - param: input
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
```

规则如下：

- `websocket.yml` 顶层只允许 `path`、`connect`、`jsonRpc`；文件存在时 `path` 必填。只含 path、供
  Skiff 主动 send/request 的 entry 合法。
- `jsonRpc` 每个 key 是 `GatewayEntryKey`；key 和 external `method` 分别唯一。method 是非空 string，
  业务声明不得以 `$/` 开头。
- handler 必须是当前 implementation package 的精确、非 generic callable；没有 `guard`/`pre`，只能
  unary return，`Stream<T>` 直接拒绝。
- adapter args 必须覆盖每个 formal parameter 且 param 名唯一。`websocket.jsonRpcParams` 必须恰好一次；
  `websocket.connectionId` 和 `websocket.businessIdentity` 可按通用 source 规则绑定给其它参数。
- `jsonRpcParams` 参数必须有可执行 JSON codec 和可派生的 entry-local external schema；其所有合法顶层值
  必须是 object/array。nullable/scalar-only、generic、stream 或开放的 untyped schema 失败。
- return 必须有可执行 JSON codec；`void` 的 external/result schema 是 `Null`。
- 平台 request id、raw frame、message body、字段 path、notification/event name 都不是 adapter source。
- HTTP 与 JSON-RPC entry 最终进入同一个 deployment `gatewayEntries` map；跨 `http.yml` /
  `websocket.yml.jsonRpc` 的 key 冲突和用户占用 compiler-reserved `websocket` key均在 projection
  阶段拒绝。

### 4.2 物理 entry、method entry 与 selector

每个合法 `websocket.yml` 始终投影一个 compiler-owned 物理 entry：

- key 固定为 `GatewayEntryKey("websocket")`；
- kind 固定为 `websocketConnect`；
- connect handler 可选；无 handler 时 adapter args 必须为空；
- `WebSocketEntryId` 继续只由 `(serviceId, "websocket")` 导出，因此不随 method key/数量变化；
- connect selector 是现有 WebSocket host/path selector，`method=None`。

每个 `jsonRpc` record 另投影一个 method entry：

- key 是作者 key，kind 为新增 `websocketJsonRpc`，handler 必填；
- selector 复用物理 entry 的 host/path，`protocol=WebSocket`，`method=Some(external method)`；
- profile 不增加第二个作者字段；它是 method protocol surface 中精确的
  `jsonrpc-2.0-text`。Router 的查找 key 是
  `(WebSocketEntryId, profile, selector.method)`；
- 当前 `IngressSelector` wire 因此无需新增 profile 字段，但 Rust/TS decoder 必须允许
  `WebSocket + method!=null` 仅连接到 `websocketJsonRpc` entry。`method=null` 只能连接到
  compiler-owned `websocketConnect` entry；
- 同一 service 的所有 method binding 必须关联同一个 compiler-owned physical entry，并共享其
  WebSocketEntryId。

这保留了两步查找：

```text
(websocketEntryId, jsonrpc-2.0-text, method)
  -> GatewayEntryKey
  -> GatewayEntryIdentity / exact linked handler
```

Router upgrade 时从当时的 immutable snapshot 构建该 method table，并把它放入 `Connection`。active
assembly replacement 后，老 socket 继续使用老表；新 socket 使用新表。

### 4.3 Artifact surface 与 identity

Rust/TS 两侧增加同名、strict DTO：

```text
GatewayAdapterKind::WebSocketJsonRpc      wire "websocketJsonRpc"
GatewayAdapterSource::WebSocketJsonRpcParams
                                         wire "websocket.jsonRpcParams"
GatewayAdapterSource::WebSocketBusinessIdentity
                                         wire "websocket.businessIdentity"
GatewayWebSocketRpcProfile::JsonRpc2_0Text
                                         wire "jsonrpc-2.0-text"

GatewayWebSocketConnectProtocolSurface {
  connectRequestShape = v1,
  connectResultShape = v1,
  connectionPolicyShape = v1,
  externalSources,
  downlinkFrames,
  rpcProfiles = [jsonrpc-2.0-text]
}

GatewayWebSocketJsonRpcProtocolSurface {
  profile,
  dispatchMode = unary,
  externalSources,
  paramsSchema,
  resultSchema
}
```

`websocket.connectionId` 现有 source 可同时用于 connect 和 JSON-RPC；新增两个 source 的阶段矩阵必须由
compiler、artifact validation、Router strict loader 和 runtime adapter 四处一致校验。
两种 protocol surface 都继续使用外层
`GatewayEntryProtocolSurface.externalErrorProjection = fixed/v1`；不得在 method record 再声明作者可配的
error schema/code。

| source | HTTP typed/raw | WebSocket connect | WebSocket JSON-RPC |
| --- | --- | --- | --- |
| `websocket.connectRequest` | 禁止 | 可用 | 禁止 |
| `websocket.connectionId` | 禁止 | 可用 | 可用 |
| `websocket.jsonRpcParams` | 禁止 | 禁止 | required exactly once |
| `websocket.businessIdentity` | 禁止 | 禁止 | optional |

Identity preimage 冻结为：

- physical `websocketConnect` identity：connect request/result v1、connection-policy v1、允许的 raw
  text/binary downlink classes、支持的 profile 列表 `[jsonrpc-2.0-text]`、fixed external error
  projection；
- method `websocketJsonRpc` identity：kind、profile、unary、canonical sort/dedup 后的 external source
  kind 集合、params/result external schema、fixed JSON-RPC gateway error projection；
- 不包含 external method 字符串、GatewayEntryKey、formal param 名或顺序、handler /
  `PackageCallableId`、internal nominal type identity、Package build 或 deployment policy。

完整 `GatewayAdapterPlan { param, source }`、handler 和 selector 仍属于 `ServiceDeployment`。改变
`websocket.yml` 不改变 PackageArtifact 或 ServiceContract；它改变 ServiceDeployment revision/identity
及后续 RuntimeAssembly。

### 4.4 Fail-closed 删除面

shared checkpoint 必须拒绝或删除：

- `service.yml.http` / `service.yml.websocket` 及旧 inline reader；
- `receive`、`websocketReceive`、`websocket.receiveEvent`、`websocket.message`、
  `websocket.messageBody`、raw business notification/event fallback；
- `bind`、`handlerArgs`、`identity`、`connection.identity`、transport id source；
- JSON-RPC batch authoring、binary RPC authoring、notification handler；
- Router/runtime 旧 `receiveEvent` metadata 与 README 中的 raw receive 描述。

Skiff 尚未发布，不保留 compatibility alias。

## 5. Profile-neutral broker

### 5.1 内部接口

broker 核心只依赖如下抽象；`OpaquePayload` 只能被 profile adapter 创建、传递和重新编码：

```ts
type ProfileId = 'jsonrpc-2.0-text';
type OpaquePeerId =
  | { kind: 'string'; value: string }
  | { kind: 'safeInteger'; value: number };
declare const opaquePayloadBrand: unique symbol;
type OpaquePayload = { readonly [opaquePayloadBrand]: true };

interface ProfileLimits {
  maxTextBytes: number;
  maxJsonDepth: number;
  maxJsonNodes: number;
  maxStringBytes: number;
}

interface OutboundIdGeneration {
  readonly randomPrefix: string;
  takeSequence(): bigint; // monotonic, no rollback/wrap/reuse
}

type ProfileResponse =
  | { kind: 'success'; result: OpaquePayload }
  | {
      kind: 'remoteError';
      code: number;
      message: string;
      dataPresent: boolean;
      data?: OpaquePayload;
    };

type PlatformRpcError =
  | { kind: 'parse' }
  | { kind: 'invalidRequest' }
  | { kind: 'methodNotFound' }
  | { kind: 'invalidParams' }
  | { kind: 'internal' }
  | { kind: 'serverBusy' }
  | { kind: 'timeout' }
  | { kind: 'cancelled' };

interface WebSocketRpcProfileAdapter {
  readonly profile: ProfileId;

  classifyText(frame: string, limits: ProfileLimits): ProfileAction;
  peerIdKey(id: OpaquePeerId): string;
  nextOutboundId(generation: OutboundIdGeneration): OpaquePeerId;
  fromRuntimePayload(
    bytes: Uint8Array,
    purpose: 'outboundParams' | 'inboundResult',
    limits: ProfileLimits
  ): OpaquePayload;
  toRuntimePayload(payload: OpaquePayload, limits: ProfileLimits): Uint8Array;

  encodeOutboundRequest(input: {
    id: OpaquePeerId;
    method: string;
    params: OpaquePayload;
  }): string;
  encodeCancel(id: OpaquePeerId): string;
  encodeResult(id: OpaquePeerId, result: OpaquePayload): string;
  encodePlatformError(id: OpaquePeerId | null, error: PlatformRpcError): string;
}

type ProfileAction =
  | { kind: 'request'; id: OpaquePeerId; method: string; params: OpaquePayload }
  | { kind: 'response'; id: OpaquePeerId; terminal: ProfileResponse }
  | { kind: 'cancel'; id: OpaquePeerId }
  | { kind: 'ignoredNotification'; method: string }
  | {
      kind: 'platformError';
      id: OpaquePeerId | null;
      error: PlatformRpcError;
    }
  | { kind: 'close'; code: number; reason: string };
```

`jsonrpc-2.0-text` adapter 可以用 JSON parser 检查 control shape，但不得检查 params/result/data 的业务
字段。broker 核心不得直接访问 peer JSON 的 `jsonrpc`、`result`、`error.code` 等字段；它只能搬运 adapter
产出的 typed terminal/opaque payload。新增未来 profile 必须实现同一接口，不复制
connection/pending/timer owner。

`fromRuntimePayload(outboundParams)` 只验证 UTF-8、JSON、limit 与 object/array 顶层；`inboundResult`
允许任意 JSON result。`toRuntimePayload` 用于 peer success result/remote data 进入
`connection.response`，以及 peer params 进入 runtimeAssembly request。业务 schema/`RuntimeTypePlan`
仍只在 runtime；这些转换不得按字段解释 payload。
`platformError.id != null` 仍由 broker 先做 inbound duplicate/tombstone transition，再调用
`encodePlatformError`；adapter 不拥有 settled state。

### 5.2 Connection generation

每次 socket attach 创建不可复用的 `socketGeneration` token，并保存：

```text
ConnectionGeneration {
  socket object
  socketGeneration
  connectionId
  serviceId / websocketEntryId
  deployment revision / artifact identity
  assembly identity / generation
  optional runtime receipt / replica owner
  optional businessIdentity
  immutable Map<(profile, method), pinned ingress binding>
  outbound id generation state
}
```

`runtime receipt / replica owner` 对 method table 非空或 connect handler 存在的 generation 是 required
invariant；只对纯 path-only raw-send connection 可 absent。broker 构造时断言该 invariant，不接受运行中
补绑 owner。

查找同时比较 socket object 与 token；connectionId 即使未来被错误复用也不能跨 generation 命中。
outbound id 使用 generation-random prefix 加单调计数器，计数器不回绕；耗尽时 fail
`resourceLimit`，不得在该 generation 重用旧 string id。

### 5.3 两张独立表

Outbound 表：

```text
OutboundPeerKey =
  (direction=outbound, connectionId, socketGeneration, profile, stringPeerId)

OutboundRuntimeKey =
  (originRuntimeSocket/session, runtimeCorrelationId)

OutboundPending {
  both keys
  exact ConnectionGeneration reference
  source service/entry/assembly owner
  method
  effective deadline/timer
  terminal token
}

OutboundTombstone =
  OutboundPeerKey -> { settledAt, expiresAt, FIFO sequence }
```

Inbound 表：

```text
InboundPeerKey =
  (direction=inbound, connectionId, socketGeneration, profile,
   typedPeerIdKey("s:<value>" | "n:<canonical-safe-integer>"))

InboundActive {
  peer key + original typed id
  exact ConnectionGeneration reference
  pinned GatewayEntryKey/Identity/selector
  dispatcherCorrelationId
  AbortController
  unique execution token
}

InboundTombstone =
  InboundPeerKey -> { settledAt, expiresAt, FIFO sequence }
```

必须有两个方向各自的 per-generation 与 broker-global active/pending limit，以及各自 tombstone
limit/TTL；计数和 map insert 在同一临界区。tombstone 满时驱逐最旧项，不拒绝新 request。Outbound
的“不复用”由 id generator 保证，不依赖 tombstone 永久保存；inbound 在 tombstone 到期/驱逐后允许
peer 复用 id。
TTL 由 broker 的每方向有界 expiry queue + 单一 sweeper/lazy sweep 拥有，不为每个 tombstone 留一个
独立 timer；connection teardown 同步清除对应 queue/index entry。

`RuntimeDispatcher.pending` 是第三张、仅用于 Router→runtime gateway dispatch 的表。broker 的
`InboundActive` 只保存 correlation/token，不共享其 entry、timer 或 resolver。

### 5.4 状态迁移不变量

```text
outbound:
  absent -> active -> tombstone -> evicted

inbound:
  absent -> active -> tombstone -> evicted
  absent -> tombstone -> evicted  // valid-id pre-dispatch error
```

每个 terminal path 都必须先：

1. active path 比较 exact entry/execution token；pre-dispatch error 则在同一临界区确认 key absent；
2. 从所有 active index 删除；
3. 清 timer/abort listener；
4. 写入本方向 tombstone；
5. 才向 runtime、dispatcher 或 peer 发出 terminal/cancel。

任一第 5 步失败都不得把 entry 放回 active。所有 late callback 只能看见 tombstone/absent，不能二次完成。

## 6. `jsonrpc-2.0-text` 完整 classifier

一个 WebSocket text frame 只承载一个 JSON value。平台限制 text bytes、JSON depth/node/string size；
超限以 `1009` 关闭。WebSocket binary data 以 `1003` 关闭。ping/pong/close 由 WebSocket 协议栈处理。
由 peer oversize/binary/malformed response 导致的平台 protocol close，把该 generation 的 outbound
pending 结算为 `protocolError`；普通网络/peer close 才结算为 `transportUnavailable`。所有 inbound
execution 都只 abort，不在 closing socket 上写 error。

Control object 使用 exact field set；重复 control member、未知 top-level member 或未知 error member
fail closed。params/result/error.data 作为受总量限制的 opaque JSON value，不做业务 schema 检查。
control string limit 必须预留 fixed result/error envelope overhead，保证任何已接纳 typed id 都能编码最小
platform error；不能先安装 execution 再发现 id 本身使 terminal frame 超限。

分类顺序：

| 输入 | 条件 | 行为 |
| --- | --- | --- |
| 非法 JSON text | parser 失败 | 单个 `-32700 Parse error`, `id:null`；不关闭健康 socket |
| batch | top-level array，包括空 array | 单个 `-32600 Invalid Request`, `id:null`；不执行成员 |
| 非 object | null/string/number/bool | `-32600`, `id:null` |
| response candidate | top-level 含 `result`/`error`，或含 `id` 且不含 `method` | 进入严格 response 分支；不得回落为 request |
| request/notification candidate | 含 `method` 且不含 result/error | 进入 request 分支 |
| 其它 object | 无 method/result/error | `-32600`, `id:null` |

Batch 即使包含当前 outbound id、合法 request 或 cancel 也不查任何状态表，只产生上述单个 invalid-request
reply。

### 6.1 Request

有 id 的 request exact fields 为：

```json
{"jsonrpc":"2.0","id":"non-empty-or-safe-integer","method":"non-empty","params":{}}
```

规则：

- `jsonrpc` 必须精确为 `"2.0"`；
- id 只允许非空 string 或 JavaScript safe integer；`null`、空 string、fraction、越界 number、
  boolean/object/array 非法；
- safe integer key 按数学整数 canonicalize：`-0` 与 `0`、`1e0` 与 `1` 是同一 id，response 回显 canonical
  JSON number；不得按原始数字 token spelling 建不同 execution；
- method 必须是非空 string；
- params 必须存在且是 object 或 array；
- 外层/version/id/method 结构非法属于 `-32600 id:null`；
- 外层、id、method 已合法后，params 缺失或 shape 非法为 `-32602` 并回显原 typed id；
- 先检查同方向 active/tombstone duplicate。命中任一项以 `1002` 关闭，不发送同 id error，并取消该
  generation 的所有 inbound execution；
- 合法但未命中 pinned method table 先写 tombstone，再返回 `-32601` 并回显 id；
- inbound capacity 满返回 `-32000` 并回显 id，先写 tombstone 后写 error；
- 命中后只把 opaque params 交给 `RuntimeDispatcher`。

只要已识别合法 typed id，所有 method/params/capacity/timeout/internal/cancel terminal 都必须在写
result/error 前进入 inbound tombstone；parse、batch、`id:null` invalid request 没有可建 tombstone 的
typed id。

### 6.2 Notification

notification 没有 `id`：

- 普通 notification 的 `jsonrpc`/method 必须合法；params 可省略，存在时只能是 object/array；
- 除 `$/cancelRequest` 外，合法 notification 一律忽略（可做有界 telemetry），即使 method 与 declared
  request method 同名；
- `$/cancelRequest` 必须 exact 为
  `{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":<typed-id>}}`；
- cancel id 使用 inbound typed-id 规则，只查 inbound active。active 命中则先 settled，再 abort
  dispatcher，并 best-effort 回 `-32800`；unknown/tombstoned id 静默忽略；
- malformed notification 是 `-32600 id:null`，不进入用户代码。

有 id 的 `$/cancelRequest` 不是 control notification；它是未声明 reserved request，返回
`-32601` 并回显 id。

### 6.3 Response

success exact fields：

```json
{"jsonrpc":"2.0","id":"platform-string-id","result":<opaque-json>}
```

error exact fields：

```json
{"jsonrpc":"2.0","id":"platform-string-id",
 "error":{"code":-32603,"message":"bounded string","data":<optional-opaque-json>}}
```

规则：

- id 必须是 non-empty string；safe integer 不能回复 platform outbound request；
- `result` 与 `error` 必须恰好一个；
- error code 必须是 JavaScript safe integer，message 是受限 string，data 可省略或为任意受限 JSON；
- malformed response、wrong socket/generation/profile、非 string id 都以 `1002` 关闭，并把该 generation
  的 outbound pending 终结为 `protocolError`；
- exact active id 只 terminal 一次；success/result 或 valid remote error 被交回原 runtime；
- outbound tombstone id 静默丢弃；
- unknown 且不在 tombstone 的 id 以 `1002` 关闭；
- 即使同值 id 当前存在于 inbound active，response 也只能查 outbound 表；找不到仍是 unknown response。

平台生成 outbound request 始终带 object/array params，永不让 peer 覆盖 id。

### 6.4 固定 platform errors

| code | message | id |
| --- | --- | --- |
| `-32700` | `Parse error` | null |
| `-32600` | `Invalid Request` | null |
| `-32601` | `Method not found` | 原 typed id |
| `-32602` | `Invalid params` | 原 typed id |
| `-32603` | `Internal error` | 原 typed id |
| `-32000` | `Server busy` | 原 typed id |
| `-32001` | `Request timed out` | 原 typed id |
| `-32800` | `Request cancelled` | 原 typed id |

这些错误默认省略 data，不包含 Skiff exception、stack、artifact path、runtime id 或 payload。

## 7. Outbound runtime suspension 与 wire

### 7.1 公开 surface

`std/websocket.skiff` 增加：

```skiff
type WebSocketRequestError discriminator "tag" =
  { tag: "connectionUnavailable", message: string }
  | { tag: "transportUnavailable", message: string }
  | { tag: "protocolError", message: string }
  | { tag: "resourceLimit", message: string }
  | { tag: "remote", code: integer, message: string, data: Json? }

native function requestJsonToConnection<TRequest, TResponse>(
  connectionId: string,
  method: string,
  value: TRequest
) -> TResponse
```

并从 `std/api.yml` 导出 error 与 callable。前四个 branch 的 message 是固定、脱敏平台文本；remote branch
保留经过大小/shape 检查的 peer code/message/data。

`artifact-model/src/native_signature.rs` 冻结：

- `type_param_count=2`；
- params `[string, string, T0]`，return `T1`；
- required context `Websocket`；
- callable semantics `may_suspend=true`，其它 caller-alias/write effects 为 false。

本地 outcome：

| 情况 | caller 可见结果 |
| --- | --- |
| `TRequest` encode 失败 | `std.json.DecodeError` |
| encoded params 非 object/array | `std.json.DecodeError` |
| success `TResponse` decode 失败 | `std.json.DecodeError` |
| 空 method / malformed local framing | `WebSocketRequestError.protocolError` |
| connection 不存在/已关闭/不属 exact generation | `connectionUnavailable` |
| Router/runtime transport 在已接纳后丢失 | `transportUnavailable` |
| peer malformed/forged response | `protocolError` |
| local outbound pending/encoded params/method-size limit | `resourceLimit` |
| valid peer JSON-RPC error | `remote` |
| operation deadline | `TimeoutError` |
| ancestor/request cancellation | 不可捕获 terminal，无普通 error |

runtime error materialization 必须为这五个 named-union branch 建立 exact catch identity；不能把它们扁平化为
`RuntimeError::ProviderUnavailable`。可复用 `RuntimeTypePlan` 已标注的 named-union branch
`CatchIdentity`，由 native/eval boundary 按 std symbol + discriminator 选择 exact branch。对应实现属于
T0；若走 platform builtin registry，也必须保留 branch identity，而不是只注册整个 union nominal。

挂起/恢复链固定为：

```text
std requestJsonToConnection<TRequest,TResponse>
  -> runtime native 用调用点 TRequest plan encode + object/array check
  -> ConnectionRequestRegistry 先安装 lease/timer/cancel-on-drop
  -> Host queue connection.request
  -> RuntimeEndpoint source trust
  -> Router broker 分配 peer id / 安装 OutboundPending
  -> profile 写 exact socket JSON-RPC request
  -> profile classify peer response
  -> broker detach/tombstone / connection.response 回原 runtime session
  -> ConnectionRequestRegistry terminal CAS 唤醒 NativeCapabilityFuture
  -> runtime 用调用点 TResponse plan decode
  -> eval 恢复原 execution segment
```

encode/shape 失败发生在 lease/frame 前；decode 失败发生在 broker 已成功 settlement 后。后者只抛
`std.json.DecodeError`，不得重开 pending、重试 peer 或改写为 transport error。

### 7.2 内部 frame

新增 strict typed-binary frame：

| 方向 | type | header 核心字段 | payload |
| --- | --- | --- | --- |
| Runtime→Router | `connection.request` | runtime correlation id、serviceId、websocketEntryId、connectionId、profile、method、deadline | 必须存在；UTF-8 encoded opaque params JSON |
| Runtime→Router | `connection.request.cancel` | runtime correlation id、bounded reason | 无 payload |
| Router→Runtime | `connection.response` | correlation id、outcome、remote code/message/dataPresent | success result或remote data按 presence 规则携带；其它 outcome 无 payload |

字段 spelling 冻结如下；三者都另含现有
`schemaVersion = skiff-runtime-frame-v1`，decoder 必须 deny unknown fields：

```text
connection.request {
  requestId: non-empty runtime-session correlation string
  serviceId: non-empty string
  websocketEntryId: canonical physical entry id
  connectionId: non-empty exact target
  profile: "jsonrpc-2.0-text"
  method: non-empty string
  deadline?: { timeoutMs: u64, expiresAt: RFC3339 string }
}

connection.request.cancel {
  requestId: exact original correlation
  reason: existing RequestCancelReason closed wire enum
}

connection.response {
  requestId: exact original correlation
  outcome: ConnectionResponseOutcome
  remote?: { code: safe integer, message: bounded string, dataPresent: boolean }
}
```

同一 runtime session 的 `requestId` 必须 session-lifetime 不复用，而不只是 active lease 内唯一；
registry 先安装 lease 再 queue frame，且不得把重连后的同名 runtime 当作同一 session。这样晚到 cancel
不能命中新请求，也不需要把 runtime correlation 混入 peer tombstone。`connection.request.cancel` 不携带
remote metadata；cancel reason 只用于内部 cleanup，不投影到 peer business value。

`connection.response.outcome` exact 为：

```text
success
deadlineExceeded
connectionUnavailable
transportUnavailable
protocolError
resourceLimit
remote
```

success 必须有 payload，即使 JSON result 为 `null`；remote 只有 `dataPresent=true` 时有 payload。
`deadlineExceeded` 在 runtime 映射为 `TimeoutError`，不进入 `WebSocketRequestError`。
`connection.request` 必须有非空 payload，cancel 必须为空。response 的 `remote` header 当且仅当 outcome
是 `remote` 时存在；`dataPresent=true` 时 payload 必须存在（JSON `null` 也算存在），false 时必须为空；
所有其它非-success outcome 的 header 无 `remote` 且 payload 为空。

不复用 `request.cancel` 表达 outbound connection cancel。现有 `request.cancel` 仍属于
Router→runtime gateway/service dispatch；权威专用 frame `connection.request.cancel` 可以防止同
correlation spelling 误触 `RuntimeDispatcher.pending`。

runtime `ConnectionRequestRegistry` 以 correlation id 持有 suspended call，Router broker 以同 correlation
持有 peer id；两者是跨 transport 的两个本地 owner。Router 必须把 response 发回原 runtime socket/session，
不能发给重连后的同名 runtime。

### 7.3 Runtime source trust

`RuntimeEndpoint` 必须把已注册 sender/session token 与 frame 一起交给 broker。broker 在分配 peer id 或写
socket 前依次验证：

1. `requestId` 非空且没有 active duplicate；session-lifetime nonreuse 由 runtime correlation generator
   保证，不在 Router 建无界历史表；profile 精确受 physical entry 支持，method/deadline/payload合法；
2. sender 的 registered activation 精确拥有 `serviceId`、`websocketEntryId` 与 connection 保存的
   assembly identity/generation；
3. connection 的 service/entry 与 frame 完全相同；有 receipt 的 generation 还必须是该 receipt 的 exact
   runtime sender；
4. 只有纯 path-only、无 receipt 的 generation 才允许同 service/assembly generation 的 registered
   replica 发起 exact-connection request。

不存在或归属不匹配的 connection 对合法 caller 统一返回 `connectionUnavailable`，不泄漏其它 service 的
connection。重复 correlation、伪造注册归属、unknown profile 或 malformed typed-binary header 是内部
runtime protocol violation：不写 peer frame；若原 correlation 可安全识别则回 `protocolError`，随后按
RuntimeEndpoint trust policy 关闭 offending runtime session 并由 registry settle 其余 lease。

## 8. Inbound RuntimeDispatcher / runtime adapter

```text
peer text frame
  -> profile classify + typed id/opaque params
  -> broker duplicate/capacity check + pinned method-table lookup
  -> InboundActive(peer id <-> dispatcher correlation)
  -> RuntimeDispatcher exact connection receipt / request.start
  -> Host exact generation pin + sibling method route
  -> linked RuntimeTypePlan params decode / handler / result encode
  -> dispatcher response terminal
  -> broker detach/tombstone
  -> profile writes exactly one result/platform error on original socket/id
```

### 8.1 Router→runtime request

`RuntimeDispatcher` 增加 sibling API：

```ts
dispatchAssemblyWebSocketJsonRpc(
  request: RuntimeAssemblyWebSocketJsonRpcRequest,
  timeoutMs: number,
  connectionReceipt: RuntimeDispatchConnectionReceipt,
  options: { signal: AbortSignal }
): Promise<RuntimeAssemblyWebSocketJsonRpcResponse>
```

receipt 由 upgrade 阶段的 eager generation-pin handshake 产生；broker 不接收 raw runtime socket，也不
二次调用 registry 选 owner。`RuntimeDispatcher` 从自己的 receipt weak map 取回 exact runtime
connection，继续使用自己的 internal request id、pending map、exact runtime socket、timeout 与
detach-before-cancel。broker 在 `InboundActive` 中只保存 dispatcher correlation/execution token。

新增 `request.start` runtimeAssembly branch：

```text
routing.kind = runtimeAssembly
routing.assemblyIdentity / assemblyGeneration = socket pin
routing.gatewayEntryIdentity = pinned method entry
routing.ingress.host / path = pinned method selector
routing.ingress.protocol = webSocket
routing.ingress.method = exact external method
mode = unary

websocketJsonRpc {
  profile = jsonrpc-2.0-text
  connectionId
  websocketEntryId
  gatewayEntryIdentity
  optional businessIdentity
}

payload = opaque params JSON bytes
```

header 不含 peer transport id。Router dispatcher request id、trace id 也不进入 payload 或 adapter source。

runtime response 使用 `response.end.websocketJsonRpc.outcome`：

```text
success          payloadPresent=true, opaque encoded result
invalidParams    payloadPresent=false
internalError    payloadPresent=false
deadlineExceeded payloadPresent=false
```

Router 映射为 result、`-32602`、`-32603`、`-32001`。expected business failure 是 typed result union，
仍走 `success`。peer cancel/disconnect 不增加 `cancelled` response outcome：broker 先 settled 并 abort，
`RuntimeDispatcher` 发既有 `request.cancel`，required cancellation checkpoint 保证 Host 清 work 且不回普通
response。

### 8.2 Runtime pin 与 exact target

runtime 收到 method request 时：

1. 用 `(routerSessionId, connectionId, assembly identity/generation, websocketEntryId)` 查现有
   `WebSocketGenerationRegistry`；
2. 从 pin 内 physical route 持有的 immutable active candidate 解析 request 中的 method selector；新增
   API 返回 sibling `ActiveAssemblyRoute`，而不是只返回 connect route；
3. exact join GatewayEntryKey、GatewayEntryIdentity、`websocketJsonRpc` surface、adapter plan、handler；
4. 验证 profile、unary、source 阶段和 physical WebSocketEntryId；
5. 创建 sibling `RuntimeAssemblyWebSocketJsonRpcTarget`。

Sibling lookup 必须同时要求同一 deployment owner、同一 host/path、`protocol=WebSocket`、
`method=Some(...)` 与同一 physical `WebSocketEntryId`；它不是任意 selector 的 pin bypass。

不得调用 current active assembly lookup。旧 generation 在 replacement 后仍能完成；只有其 runtime pin
丢失/释放才关闭对应 socket。若 method-bearing socket 的 receipt/pin 缺失，Router 必须在 attach 前拒绝
upgrade；runtime method ingress 也必须 fail closed，不能降级到 current assembly。

### 8.3 Typed adapter

runtime adapter：

- 由 linked handler signature 为每个 formal param 建 `RuntimeTypePlan`；
- `websocket.jsonRpcParams` 用 full params payload decode；profile 已验证 object/array，runtime 再防御性
  验证；
- `websocket.connectionId` 构造 string；
- `websocket.businessIdentity` 构造 `string?`，peer payload 不能覆盖；
- handler 普通 unary 执行，可按其真实 callable summary 挂起；
- normal return 用 linked return plan encode；`void` 明确编码 JSON `null`；
- decode/shape 失败在 handler 前返回 `invalidParams`；
- return encode 失败或 uncaught throw 只返回 `internalError`，不泄漏名义 error/message/stack；
- encoded result 超过 runtime/gateway payload limit 也返回小型 `internalError`，由 broker 写
  `-32603`；不得把 oversized result 部分写入 socket；
- expected failure 必须由 handler return union 表达。

## 9. Race 与唯一 terminal owner

### 9.1 Outbound

| race/event | 原子动作 | 唯一语义 owner / 结果 |
| --- | --- | --- |
| 双向同值 id | response 只查 outbound；cancel/request 只查 inbound | 两表隔离；互不影响 |
| response 乱序 | exact OutboundPeerKey lookup | broker；各自恢复原 runtime correlation |
| 首个 success/error | remove both indexes → tombstone → `connection.response` | broker；最多一次 |
| duplicate/late response | tombstone hit | broker；静默丢弃 |
| tombstone 已驱逐的 late response | active miss + tombstone miss | profile/broker；`1002`, 不恢复调用 |
| runtime cancel 先到 | remove/tombstone → best-effort peer cancel | broker；无 `connection.response` |
| response 先于 runtime cancel | response 已 tombstoned；late cancel no-op | broker；已完成结果保持 |
| broker deadline | remove/tombstone → best-effort cancel → `deadlineExceeded` | broker；runtime 映射 `TimeoutError` |
| runtime cancel 与 broker deadline 同时竞争 | broker terminal token 的首个 transition 唯一生效；另一事件只见 tombstone | broker；runtime registry 的 biased cancel/CAS 仍阻止 deadline response 注入已取消 execution |
| ancestor cancel 与 deadline 同 ready | runtime required cancellation checkpoint biased 赢；lease drop 发 `connection.request.cancel` | runtime execution terminal；不可捕获 |
| socket send callback 失败 | exact active token remove/tombstone | broker；`transportUnavailable` |
| 普通 socket disconnect | generation 批量 remove/tombstone | connection lifecycle 触发，broker settle `transportUnavailable` |
| peer protocol/size violation 导致 close | generation 批量 remove/tombstone | broker 发起 close `1002`/`1003`/`1009`，outbound settle `protocolError` |
| origin runtime disconnect | 只移除该 session 的 runtime index，并把对应 peer keys tombstone；best-effort peer cancel | broker；不尝试回复死 runtime |
| eager generation acquire/ack 在 attach 前失败 | release expectation/receipt，拒绝 upgrade；broker generation 尚未安装 | gateway + generation lifecycle；不产生 peer request |
| assembly active replacement | 老 Connection/table/pin 不变 | generation pin owner；pending 继续 |
| generation-pinned runtime owner 丢失 | lifecycle 关闭老 socket，再批量清表 | generation lifecycle + broker，各自只清本地 owner |
| 纯 path-only connection 遇到 replacement | socket 仍属旧 generation；新 generation runtime target 被 exact owner check 拒绝，原 runtime pending 可完成或随其 disconnect 清除 | connection lifecycle + broker；该 entry 没有 inbound dispatch/pin |
| local outbound capacity/payload/method-size limit | 在写 socket 前拒绝；id counter即使已分配也不回退 | broker；`resourceLimit` |

runtime `ConnectionRequestRegistry` 也用自己的 terminal CAS：本地 cancellation 已赢时，晚到
`connection.response` 只清 registry/丢弃，不能把 value 注入已终止 execution。

### 9.2 Inbound

| race/event | 原子动作 | 唯一语义 owner / 结果 |
| --- | --- | --- |
| active duplicate id | 关闭 `1002`，settle/abort generation 全部 inbound | broker；重复 request 无 error response |
| tombstoned duplicate id | 同上 | broker |
| 两个不同 id 乱序完成 | execution token + dispatcher correlation | dispatcher 完成 runtime leg；broker各写一次原 typed id |
| peer cancel 先到 | remove/tombstone → abort dispatcher → best-effort `-32800` | broker 是 socket terminal owner；Host 只清 work |
| completion 先到 | remove/tombstone → result/error；late cancel tombstone no-op | broker |
| deadline | `RuntimeDispatcher` timeout先 detach/cancel；promise terminal 回 broker | broker 写 `-32001`；dispatcher 只拥有 runtime leg |
| peer cancel 与 deadline 同时竞争 | broker execution token 的首个 transition 唯一生效；另一事件只见 tombstone | broker；只写 `-32800` 或 `-32001` 之一 |
| runtime explicit deadline outcome | dispatcher exact response先完成；其 timer 随 detach 清除 | broker 写同一个 `-32001` |
| peer disconnect | generation remove/tombstone → abort all | broker；不写 response |
| pinned runtime disconnect | generation lifecycle 先关闭 socket并同步 detach generation；dispatcher reject随后只清 runtime leg | broker 不写 response；gateway close `1011`，execution token 阻止晚完成 |
| assembly active replacement | 继续使用 Connection 捕获的旧 method table | generation pin owner；不迁移 handler |
| late handler completion | dispatcher pending 已 detach或 broker execution token 不匹配 | 丢弃；无第二次 socket write |
| capacity | duplicate 检查后、dispatch 前 remove-to-tombstone | broker；`-32000` |
| tombstone TTL/容量驱逐 | FIFO 删除 settled record | broker；允许 id 后续复用 |
| 驱逐后旧 dispatcher completion | unique execution token/correlation 不匹配 | 丢弃，不完成新 request |

JSON-RPC cancel wire 只有 peer id。tombstone 驱逐后若 peer 主动复用同 id，随后到达的旧 cancel 与新 cancel
在 wire 上不可区分，将按协议取消当前 active id；这是允许复用的固有边界，peer 应使用 connection-lifetime
唯一 id。平台不增加业务可见 epoch 来改变 wire。

## 10. TS/Rust schema、fixture、README/checker 与测试

### 10.1 必须成对修改的 schema

| 事实 | Rust owner | TypeScript owner |
| --- | --- | --- |
| authoring/adapter/source/surface | `artifact-model/src/{ecosystem_authoring,gateway}.rs` | Router 只消费 generated deployment strict DTO |
| deployment selector/join | `artifact-model/src/{deployment,runtime_assembly}.rs`、`artifact-identity/**` | `router/src/router/runtimeAssembly{Deployment,}Snapshot.ts` |
| connection request/cancel/response | `runtime/capability-context/**`、`runtime/transport/src/{protocol,control_mapper}.rs` | `router/src/protocol/{envelope,runtimeProtocol}.ts`、`runtimeEndpoint.ts` |
| inbound runtimeAssembly request/response | `runtime/transport/src/runtime_assembly_request.rs` | `router/src/protocol/runtimeAssemblyRequest*.ts`、`runtimeProtocol.ts` |
| generation pin | `runtime/transport/src/websocket_generation_lifecycle.rs`、Host registry | `webSocketGenerationLifecycleRouter.ts`、gateway Connection |

所有 decoder deny unknown fields，payload presence 和 enum branch 必须逐项匹配；不能用宽松
`Record<string, unknown>` 跳过 final validation。

### 10.2 Direct 与 negative fixture

新增一个 direct positive fixture，至少同时证明：

- 独立 `service.yml` + `websocket.yml`；
- handler 绑定 params、connectionId、businessIdentity；
- record params、array params、void→null、typed result union；
- 无 connect handler 但有 declared method 时，upgrade 仍建立 generation pin/receipt 且不执行用户 connect
  callable；
- `requestJsonToConnection<Record, Response>` 的 public ABI/effects；
- compiler projection 的 physical key/ID、method keys/selectors/identities；
- Rust/TS 对同一 `connection.*` 和 `runtimeAssembly websocketJsonRpc` corpus byte/JSON 一致。

建议 owner：

```text
test-runner/fixtures/package-service-websocket-jsonrpc/**
cross-system-fixtures/package-service-ecosystem/runtime-websocket-jsonrpc-wire.json
cross-system-fixtures/package-service-ecosystem/websocket-bidirectional-jsonrpc-profile.json
```

negative legacy fixture 必须包含旧 inline `service.yml.websocket`、`receive`、transport id source、business
notification handler、batch/binary 字段中的至少一项，并断言 parser/projection 失败；不能把失败 fixture
静默迁移成新 shape。另加 malformed/forged peer frame corpus，覆盖 null/fraction/unsafe id、both/neither
result/error、重复/unknown control member、unknown response、active/tombstoned duplicate、batch、binary。

### 10.3 README/checker

- 更新 `runtime/README.md`：raw send 非挂起；删除 receive 返回 void 的旧说明；增加 request suspension 与
  runtime adapter boundary。
- 更新 `router/README.md`：删除 `ConnectionMessage` / raw receive prototype；记录 broker/profile/
  dispatcher 三 owner。
- 扩展 `scripts/check-skiff-source-layout.mjs`：锁定五个 std exports、旧 receive symbols 缺失、新 broker/
  profile 单 owner。
- 扩展 `cross-system-fixtures/package-service-ecosystem/verify.mjs` 和 Rust corpus reader，要求 TS/Rust exact
  round-trip。

### 10.4 聚焦验证矩阵

| 面 | selector |
| --- | --- |
| shared artifact/identity | `cargo test -p skiff-artifact-model websocket_jsonrpc`; `cargo test -p skiff-artifact-identity --test gateway --test deployment` |
| compiler authoring/projection | `cargo test -p skiff-compiler --test websocket_ingress`；新增 external-manifest direct/negative selectors |
| std effects | `cargo test -p skiff-compiler-source missing_dynamic_mutable_and_capability_semantics_remain_fail_closed`；新增 new-request suspension selector |
| runtime native/codec | `cargo test -p skiff-runtime-native websocket`; encode/params/result decode/error-branch tests |
| Rust wire | `cargo test -p skiff-runtime-transport connection_`; runtimeAssembly JSON-RPC corpus |
| runtime target/adapter | `cargo test -p skiff-runtime-request websocket_jsonrpc`; `cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc`; `cargo test -p skiff-runtime-host websocket_jsonrpc`；含 handlerless eager pin |
| Router profile | `npm --prefix router exec -- vitest run tests/websocket-jsonrpc-profile.test.ts` |
| Router broker races | `npm --prefix router exec -- vitest run tests/websocket-request-broker.test.ts` |
| Router pinned ingress | `npm --prefix router exec -- vitest run tests/websocket-gateway.test.ts tests/router-websocket-trust-dispatch.test.ts`；含无 connect handler 的 eager pin、attach 前失败和 replacement |
| Router wire/trust | `npm --prefix router exec -- vitest run tests/protocol.test.ts tests/runtime-endpoint-connection-send-trust.test.ts` |
| fixture/checker | `node cross-system-fixtures/package-service-ecosystem/verify.mjs`; `node scripts/check-skiff-source-layout.mjs` |
| direct fixture | `cargo test -p skiff-test-runner --test package_service_contract_deployment websocket_jsonrpc` |

Broker race tests必须使用 fake profile adapter 证明核心测试不访问 JSON 字段；JSON-RPC classifier 测试独立于
broker state tests。

## 11. 互斥实现 DAG

```text
P0 manifest shared checkpoint + required cancellation checkpoint
  |
  v
S0 shared schema/compiler
  |
  v
T0 std/runtime transport
  |\
  | \----> R0a Router profile/core unit implementation
  v
E0 runtime typed inbound/outbound execution
  |                 |
  +-------> R0b Router dispatcher/gateway hookup
                    |
                    v
              F0 fixture/tooling
                    |
                    v
              C0 focused combined
                    |
                    v
              A0 independent acceptance
```

R0a 可与 E0 并行；R0b 必须等待 E0 的 frozen response outcome 与 cancellation behavior。每个节点只有一个
write owner：

### P0 — prerequisite gate

- write set：无。
- 首次动作：确认 external manifest shared checkpoint 和 required cancellation checkpoint 的 commit/tree
  与 focused tests。
- selector：各 checkpoint 自己声明的 artifact/compiler/cancel terminal gates。
- 证据失效：任一 checkpoint 改变 authoring shape、cancel public surface 或 ordinary response behavior，
  S0/T0 不得开始。

### S0 — shared schema/compiler

- 独占 write set：
  - `artifact-model/src/{ecosystem_authoring,gateway,deployment,runtime_assembly,lib}.rs`
    （明确排除 `native_signature.rs`）；
  - `artifact-identity/**`、`deployment/**`；
  - `compiler/input/**`、`compiler/driver/**` 及对应 compiler projection tests。
- 首次修改：在 `artifact-model/src/gateway.rs` 先加 failing strict serde/identity test，再增加
  `websocketJsonRpc` kind/source/surface。
- selector：artifact/identity/deployment tests、`compiler/tests/websocket_ingress.rs`、direct/negative
  authoring projection tests。
- 最早风险探针：path-only physical entry、两个 method、HTTP/key collision、method rename不改 method
  entry identity但改 deployment selector、schema change必改 identity。
- 证据失效：authoring、selector、source 阶段或 identity preimage 任一变化，T0/E0/R0/F0 全部重新取证。

### T0 — std/runtime transport shared RPC checkpoint

- 依赖：S0 + required cancellation checkpoint。
- 独占 write set：
  - `std/**`、`artifact-model/src/native_signature.rs`；
  - `runtime/model/src/service_error.rs` 中 WebSocket error identity；
  - `runtime/native-contract/**`、`runtime/native/**`；
  - `runtime/capability-context/**`、`runtime/request-contract/**`、`runtime/transport/**`；
  - outbound WebSocket capability、RuntimeHost registry、Router session frame demux/health 的明确文件。
- 首次修改：先在 native registry/eval effect tests 中断言旧四个 false、新 request true，再加入 std
  callable/error。
- selector：std publication/effects、native registry/error materialization、`connection.*` Rust protocol、
  registry lease cancel/deadline tests。
- 最早风险探针：remote named-union branch exact catch、ancestor cancel无 response、deadline仍是
  `TimeoutError`、origin runtime reconnect不能接旧 response。
- 证据失效：公开 signature/error branch或任一 `connection.*` header/outcome 改变，E0/R0/F0/C0 全失效。

### E0 — runtime execution

- 依赖：T0。
- 独占 write set：
  - 新 `runtime/request/src/websocket_jsonrpc_*`；
  - 新 `runtime/eval/src/runtime_websocket_jsonrpc*`；
  - `runtime/host/src/host/request_entry/{assembly,assembly_wire,websocket_jsonrpc*}.rs`；
  - `runtime/host/src/loader/**`、generation pin route resolution；
  - 这些模块的 colocated tests。
- shared host wiring file若已归 T0，E0 不并行编辑；由 T0 owner 接入 E0 暴露的 API。
- 首次修改：先加“无 connect handler 也 eager pin”与 pinned old-generation method target tests，再实现
  physical-entry no-op admission、sibling target/adapter。
- selector：request target exact join、linked params/result codec、void/union/throw、peer cancel/disconnect no
  ordinary response、generation replacement。
- 最早风险探针：current snapshot 已替换但旧 socket仍命中旧 handler；method-bearing socket 在 attach 前
  必有 receipt/pin；transport id 永不出现在 `RuntimeValue`。
- 证据失效：runtime request metadata、adapter outcome或 cancel completion改变，只使 R0b/F0/C0 失效；
  若反向要求 artifact surface变化则退回 S0。

### R0a/R0b — Router broker/profile

- 依赖：R0a 等 T0 wire；R0b 等 E0。
- 独占 write set：`router/src/**` 与 `router/tests/**`，明确排除 `router/README.md`。
- 首次修改：先建立 fake-profile broker race tests和 JSON-RPC classifier corpus，再写
  `WebSocketRequestBroker` / `JsonRpc20TextProfile`。
- selector：profile classifier、两表/tombstone/property tests、RuntimeEndpoint source trust、
  pinned-generation method table、RuntimeDispatcher abort/timeout、gateway close cleanup。
- 最早风险探针：无 connect handler 的 eager pin、同值双向 id、duplicate close、
  cancel-vs-complete、runtime/socket disconnect、tombstone eviction、generation replacement；每项检查
  active/pending/timer 归零和最多一次 write。
- 证据失效：profile action 或 state key/terminal order改变，使 F0/C0 失效；若要求 wire outcome改变则退回
  T0，若要求 method artifact改变则退回 S0。

### F0 — fixture/tooling

- 依赖：S0/T0/E0/R0b。
- 独占 write set：
  - `cross-system-fixtures/package-service-ecosystem/**`；
  - `test-runner/**`；
  - `scripts/check-skiff-source-layout.mjs` 与相关 checker；
  - `runtime/README.md`、`router/README.md`。
- 首次修改：先加入 direct positive 和 negative legacy fixture；禁止通过放宽 reader 使旧 fixture通过。
- selector：fixture verify、test-runner direct fixture、source-layout checker、README reverse grep。
- 最早风险探针：同一 corpus 同时由 Rust/TS decode，legacy receive/id/batch/binary 仍失败。
- 证据失效：任一上游 schema/wire变化都必须重生成/重审 fixture；fixture hash不能手调绕过。

### C0 — focused combined

- write set：无 production；只运行 frozen selectors。
- 首次动作：clean worktree 中按 S0→T0→E0→R0→F0 顺序执行矩阵，最后跑 Router type-check 和相关
  package-service ecosystem smoke。
- 失败路由：按最早失效边界退回单一 owner；C0 不现场修改 production。
- gate：所有 pending/timer/generation pin 归零，raw send false/new request true，TS/Rust fixture exact。

### A0 — independent acceptance

- write set：无。
- 首次动作：新 clean worktree/clone 从合流 tree 重跑 C0 及 direct/negative fixture。
- 必验：双向 race matrix、generation replacement、取消无普通 response、无旧 receive symbol。
- 不授权 push、stable instance、watch registry 或 live 环境。

## 12. 本次只读验证

| 检查 | 结果 |
| --- | --- |
| 初始 branch/status | clean，branch 精确匹配任务 |
| `cargo test -p skiff-compiler --test websocket_ingress` | PASS，5 passed |
| `cargo test -p skiff-compiler-source missing_dynamic_mutable_and_capability_semantics_remain_fail_closed` | PASS，1 passed |
| `cargo test -p skiff-runtime-transport connection_send` | PASS，3 passed |
| `node scripts/check-skiff-source-layout.mjs` | PASS |
| `cargo test -p skiff-runtime-eval connection_send_stays_inside_the_current_synchronous_segment` | baseline test build blocked：`runtime/eval/src/runtime_http_gateway/tests.rs:384` 对 `Option<PackageCallableId>` 调用 `as_str()`；审计未修改该文件。raw-send false 同时由 compiler/public ABI、source effects 与源码 exact registry 静态证据覆盖 |
| Router focused tests | 未运行：本 worktree 无 `router/node_modules/.bin/vitest`；实现 gate 必须安装既定依赖后运行本文 selectors |

现有 baseline 编译缺口不改变公开/wire 决策，也没有迫使共享 pending 或业务 JSON 进入 broker core；它应由
后续对应 runtime test owner 在 focused combined 前修复，不能由本只读 leaf 顺手修改。
