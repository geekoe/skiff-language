# Router Rust Migration C-model-request：request wire 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-model-request / W-dispatch /
`M-request` 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.2
  （`RequestDispatcher` 拥有 ordinary unary/stream 与 derived function-spawn
  correlation、terminal、reservation token）、§3.8（boundedness、business
  payload 为 immutable opaque bytes）、§5.3（C-model-request →
  W-model-request → M-request）、§5.4（contract pack 必填项、
  C-dispatch + M-request）、§5.5（Request family sink）。冲突时以权威设计
  为准。
- 父批次：`doc/implementation/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-request-leaf.md`。
- 同链契约：`router-rust-migration-c-routing-query-contract.md`、
  `router-rust-migration-c-dispatch-contract.md`。
- M0 决策：`doc/implementation/router-rust-migration-m0-decisions.md`
  （closed frame-family registry：Request family direction=Either、
  payload presence=Optional）。
## 1. 冻结范围

冻结 Router↔Runtime **ordinary request wire**（HTTP unary 与 serverStream）：

- `request.start`（`runtime_assembly_request` HTTP 形态，mode =
  `unary | serverStream`）；
- `request.cancel`（双向：Router 取消 pending，Runtime 也可取消请求）；
- `response.start` / `response.chunk` / `response.end`（stream 与 unary
  response 终态）；
- `response.error`（`fixedService` opaque 与 `control` 两种 errorKind）；
- frame 级 direction、payload presence、stream 顺序与终态语义、cancel
  reason 词表（`RequestCancelReason` 的 9 项 wire reason）。

非目标：不冻结 `RuntimeAssemblySpawnRequestStartFrameHeader`
（C-model-spawn）、WebSocketConnect/WebSocketJsonRpc request/response
（C-model-connection）、`connection.request.cancel`（connection family）、
legacy `RequestStartFrameHeader`（envelope_type 形态，仅记录差异）；
不定义 admission/pending/terminal 状态机（C-dispatch）；不定义 candidate
投影（C-routing-query）；不写 transport production。

## 2. 冻结帧集合、direction 与 payload presence

| 帧 | wire type | direction | payload presence | canonical codec（skiff-runtime-transport） |
| --- | --- | --- | --- | --- |
| `request.start`（HTTP unary/serverStream） | `request.start` | Router→Runtime | Optional（unary/serverStream 均可带 body；空 body 合法） | `decode_runtime_assembly_request_start_frame` / `encode_binary_frame` |
| `request.cancel` | `request.cancel` | Either | Empty（**必须空**；runtime host 已强制，codec 层尚未强制 → W-model 翻转 `currentEnforced`） | `decode_typed_binary_frame::<RequestCancelFrameHeader>` |
| `response.start` | `response.start` | Runtime→Router | Empty | `decode_typed_binary_frame::<ResponseStartFrameHeader>` |
| `response.chunk` | `response.chunk` | Runtime→Router | Optional（chunk 字节；空 chunk 合法） | `decode_typed_binary_frame::<ResponseChunkFrameHeader>` |
| `response.end` | `response.end` | Runtime→Router | Optional，但必须与 `payloadPresent` 一致；serverStream 终态恒空 | `decode_typed_binary_frame::<ResponseEndFrameHeader>`（phase 校验见 §5.2） |
| `response.error`（fixedService） | `response.error` | Runtime→Router | Required（opaque `ServiceErrorEnvelope` 字节，非空） | `decode_response_error_frame` |
| `response.error`（control） | `response.error` | Runtime→Router | Empty | `decode_response_error_frame` |

Request family 的 family 级规则（M0 冻结）保持：
`RuntimeFrameFamily::Request.direction() == Either`、
`payload_presence() == Optional`、wire type 前缀 `request.`；本 pack 冻结的是
**frame 级** direction/payload 规则（上表），W-model/W-dispatch 按上表强制。

## 3. request.start HTTP 形态（冻结）

### 3.1 判别与公共字段

`request.start` 的 typed 解码入口是
`RuntimeAssemblyRequestStartFrameWireHeader`（untagged 四分支）；HTTP
unary/serverStream 分支为 `RuntimeAssemblyRequestStartFrameHeader`，由
`routing.ingress.protocol == "http"` 判别。字段（camelCase、
`deny_unknown_fields`）：

```json
{
  "schemaVersion": "skiff-runtime-frame-v3",
  "type": "request.start",
  "requestId": "<bounded non-empty string>",
  "mode": "unary | serverStream",
  "caller": { "kind": "gateway" },
  "routing": {
    "kind": "runtimeAssembly",
    "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 lowercase hex>",
    "assemblyGeneration": "<safe u64>",
    "deployment": {
      "serviceId": "<non-empty>",
      "contractVersion": "<non-empty>",
      "deploymentRevision": "<non-empty>",
      "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:<64 lowercase hex>"
    },
    "gatewayEntryIdentity": "skiff-gateway-entry-v2:sha256:<64 lowercase hex>",
    "ingress": { "protocol": "http", "method": "<non-empty>", "path": "<absolute /-prefixed>" }
  },
  "clientSession": { "id": "<string>" },
  "deadline": { "timeoutMs": "<safe u64>", "expiresAt": "<ISO-8601 string>" },
  "trace": {
    "traceId": "<string>",
    "spanId": "<string>",
    "parentSpanId": "<optional string>",
    "sampled": "<optional bool>"
  },
  "httpRequest": {
    "method": "<string>",
    "url": "<string>",
    "path": "<string>",
    "query": [{ "name": "<string>", "value": "<string>" }],
    "headers": [{ "name": "<string>", "value": "<string>" }]
  },
  "testEffectsEnabled": false,
  "testCaseCapability": "<optional test correlation token>",
  "testCaseParentRequestId": "<optional test correlation token>"
}
```

`clientSession` / `deadline` / `testCaseCapability` /
`testCaseParentRequestId` 为可选字段（缺席与显式 `null` 归一为 same typed
Option）；`trace.parentSpanId` / `trace.sampled` 同理。

### 3.2 冻结校验规则（既有 `decode_runtime_assembly_request_start_frame`
实现，本 pack 冻结 corpus 语义）

- `schemaVersion == "skiff-runtime-frame-v3"`、`type == "request.start"`；
- `mode ∈ {unary, serverStream}`；
- `caller.kind == "gateway"`、`routing.kind == "runtimeAssembly"`；
- `assemblyIdentity` / `deployment.deploymentArtifactIdentity` /
  `gatewayEntryIdentity` 分别通过各自 strict ref/identity 校验；
- `assemblyGeneration` ≤ Number.MAX_SAFE_INTEGER；
- `ingress.method` 非空、`ingress.path` 以 `/` 开头；
- HTTP 变体：`testEffectsEnabled == testCaseCapability.is_some()`；
  `testCaseParentRequestId.is_some()` 要求 `testCaseCapability.is_some()`；
- HTTP 变体 payload 为 optional opaque bytes（GET 空 body、POST body 均合法；
  长度上限由连接级 ingress budget 归 C-session/W-session，本 pack 不重复定义）；
- 全部嵌套对象 `deny_unknown_fields`、重复 JSON key 拒绝（strict canonical
  JSON decode）。

`WebSocketConnect` / `WebSocketJsonRpc` / `Spawn` 分支的 payload presence 与
校验分属 C-model-connection / C-model-spawn，本 pack 只冻结
`decode_runtime_assembly_request_start_frame` 的判别行为不改变。

## 4. request.cancel 与 cancel reason 词表（冻结）

```json
{
  "schemaVersion": "skiff-runtime-frame-v3",
  "type": "request.cancel",
  "requestId": "<pending request id>",
  "reason": "<wire cancel reason>"
}
```

- payload 必须为空（Runtime host 已强制拒绝非空 payload；transport codec
  层当前不检查 → W-model-request 交付后 `currentEnforced` 翻转）。
- wire reason 是 `RequestCancelReason` 的 snake_case 值；冻结词表
  `CONTRACT_H`（9 项）：
  `caller_cancel`、`client_disconnect`、`timeout`、`deadline_exceeded`、
  `backpressure`、`protocol_error`、`stream_dropped`、`runtime_disconnect`、
  `router_shutdown`。未知 reason 不是本词表的合法 wire 值（W-model 对
  unknown reason 的行为：拒绝帧并 terminal，见 C-dispatch §3.3）。
- 方向 Either：Router→Runtime 是 dispatcher 发起的取消；Runtime→Router 是
  runtime 主动取消（TS `handleRuntimeCancel` 语义，C-dispatch 冻结 terminal
  映射）。
## 5. response 帧（冻结）

### 5.1 response.start / response.chunk

```json
{ "schemaVersion": "skiff-runtime-frame-v3", "type": "response.start",
  "requestId": "<pending request id>", "httpResponse": { "status": 200,
  "headers": [{ "name": "x", "value": "y" }] } }
```

```json
{ "schemaVersion": "skiff-runtime-frame-v3", "type": "response.chunk",
  "requestId": "<pending request id>", "seq": 0 }
```

- `response.start` payload 必须为空；只对 serverStream dispatch 合法。
- `response.chunk` seq 必须严格等于流内下一期望序号（从 0 起连续递增）；
  payload 为 chunk 字节（可为空）。

### 5.2 response.end

```json
{ "schemaVersion": "skiff-runtime-frame-v3", "type": "response.end",
  "requestId": "<pending request id>", "payloadPresent": true,
  "httpResponse": "<optional>" }
```

phase 规则（`response_mapper::validate_response_end_frame` 冻结）：

| phase | metadata | payloadPresent ↔ payload |
| --- | --- | --- |
| Payload（unary 普通终态） | 无 `httpResponse` | `payloadPresent == !payload.is_empty()` |
| Http（unary HTTP 终态） | `httpResponse` 存在 | `payloadPresent == !payload.is_empty()` |
| Stream 终态 | 无 `httpResponse` | 必须 `payloadPresent == false` 且 payload 为空 |

unary 收到 `response.end` 时，`httpResponse` 可选（Payload/Http 两种 phase）；
serverStream 的 `response.end` 必须是无 metadata、无 payload 的空终态。

### 5.3 response.error

```json
{ "errorKind": "control", "schemaVersion": "skiff-runtime-frame-v3",
  "type": "response.error", "requestId": "<pending request id>",
  "error": { "code": "<non-empty>", "message": "<non-empty>",
             "status": 503, "details": "<optional>" } }
```

```json
{ "errorKind": "fixedService", "schemaVersion": "skiff-runtime-frame-v3",
  "type": "response.error", "requestId": "<pending request id>" }
```

- `control`：payload 必须为空；`error.code`/`error.message` 非空；`status`
  若存在必须 ∈ [400, 599]。
- `fixedService`：payload 必须非空且可被 `OpaqueServiceError::decode` strict
  decode（`ServiceErrorEnvelope` canonical JSON 字节）。
- 两个变体都拒绝 schemaVersion/type 错误与空 requestId。

### 5.4 stream 顺序语义（wire 层冻结；dispatcher 强制）

```text
waitingStart: response.start（空 payload）-> streaming
streaming:    response.chunk seq == nextSeq（nextSeq 从 0 递增）或 response.end（空）
terminal:     任何终态后不得再有同 requestId 的 response 帧
```

违反项（wire 层必须被终端化）：

- unary 收到 `response.start` / `response.chunk`（UnexpectedStart /
  UnexpectedChunk）；
- stream 在 `response.start` 前收到 `response.chunk` / `response.end`
  （chunk-before-start / end-before-start）；
- 重复 `response.start`（duplicate-start）；
- chunk seq 不等于 nextSeq（chunk-seq-mismatch）；
- stream `response.end` 带 payload 或 metadata（stream-end-payload）；
- 任何 response 帧对应不存在的/已终态的 pending requestId（stale response，
  dispatcher 忽略或按 protocol terminal，C-dispatch 冻结：stale 帧被
  exact-socket fence 忽略，不产生副作用）。

## 6. Byte-exact corpus 规格

位置：`runtime/transport/testdata/request-wire/`。

### 6.1 frames.json（帧目录）

```json
{
  "schemaVersion": 1,
  "corpus": "request-wire-v1",
  "sharedCorpus": "cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json",
  "cancelReasons": ["caller_cancel", "client_disconnect", "timeout",
    "deadline_exceeded", "backpressure", "protocol_error", "stream_dropped",
    "runtime_disconnect", "router_shutdown"],
  "frames": {
    "<frame-name>": {
      "direction": "RouterToRuntime | RuntimeToRouter | Either",
      "frameType": "request.start | request.cancel | response.start | response.chunk | response.end | response.error",
      "decodeAs": "RequestStartHttpUnary | RequestStartHttpStream | RequestCancel | ResponseStart | ResponseChunk | ResponseEnd | ResponseErrorControl | ResponseErrorFixedService",
      "payloadRule": "optional | empty | required",
      "payloadHex": "<payload hex，可为空串>",
      "frameHex": "<完整二进制帧 hex，SKBF magic + version + encoding + 长度 + JSON header + payload>",
      "header": { "...": "typed header 语义 JSON" }
    }
  }
}
```

必选帧：`start.unary.req1`（带 body payload）、`start.stream.req2`（空 body）、
`cancel.req1.timeout`、`response.start.req2`、`response.chunk.req2.seq0`、
`response.chunk.req2.seq1`、`response.end.req1.payload`（unary Payload phase）、
`response.end.req2.empty`（stream 空终态）、`response.error.req1.control`、
`response.error.req1.fixed-service`。测试断言 `encode(decode(hex)) == hex`
（byte-exact roundtrip），并断言 `payloadHex` 与 decode 结果一致。另冻结
两个 codec 合法、序列语义负例专用的帧：`response.chunk.req2.seq2`
（seq gap）与 `response.start.req1.unexpected`（unary 收到 response.start）。

### 6.2 reject-cases.json（codec 级负例）

```json
{
  "schemaVersion": 1,
  "cases": [
    { "id": "<name>", "decodeAs": "RequestStartHttpUnary | ...",
      "json": { "...": "非法/变异 header" }, "expectErrorContains": "<子串>" }
  ]
}
```

必选负例：wrong schemaVersion、wrong type、invalid mode、caller kind、
routing kind、deployment artifact identity、ingress path 相对路径、
testEffectsEnabled/testCaseCapability 不一致、testCaseParentRequestId 无
capability、unknown field。

### 6.3 scenarios/*.json（序列语义）

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "events": [
    { "kind": "read|write", "frame": "<frame-name>", "payloadHex": "<可选覆盖>" }
  ],
  "expect": {
    "requestOutcomes": { "<requestId>": "completed | failed | cancelled | protocolError" },
    "terminalSources": { "<requestId>": "runtime_response_end | runtime_response_error | runtime_request_cancel | router_cancel | protocol_error" },
    "receivedChunks": { "<requestId>": ["<payload hex>", "..."] },
    "payload": { "<requestId>": "<response.end payload hex 或 null>" },
    "failStop": false
  }
}
```

必选场景：

- `unary-response-end`
- `unary-response-error-control`
- `unary-response-error-fixed-service`
- `stream-start-chunk-chunk-end`
- `stream-end-before-start-rejected`
- `stream-chunk-before-start-rejected`
- `stream-chunk-seq-gap-rejected`
- `stream-duplicate-start-rejected`
- `stream-start-on-unary-rejected`
- `stream-end-with-payload-rejected`
- `request-cancel-router-to-runtime`
- `request-cancel-runtime-to-router`
- `stale-response-ignored`

消费测试：`runtime/transport/tests/request_wire_corpus.rs`（byte-exact 校验 +
reference wire 状态机）。
## 7. 与当前 TS/Rust wire 的差异记录

| 表面 | 当前 wire（main@7683b7c8） | 目标 corpus（本契约） | 收敛动作 |
| --- | --- | --- | --- |
| Router→Runtime request.start | TS Router 发 `runtime_assembly_request` 形态；transport 同时保留 legacy `RequestStartFrameHeader`（envelope_type 形态，`request_mapper.rs` 消费） | HTTP unary/serverStream `runtime_assembly_request` 形态 | 本 pack 冻结目标形态；legacy 形态不删（既有 consumer），差异记录由 M-request/W-model 消费 |
| `request.cancel` payload | runtime host 强制空 payload；transport codec 不检查 | payload=Empty（codec 层强制） | W-model-request 实现后翻转 `currentEnforced` |
| `response.end` phase 校验 | `response_mapper::validate_response_end_frame` 在 mapper 层执行；`decode_typed_binary_frame` 不检查 | payloadPresent ↔ payload 一致性为 wire 层契约 | W-model/W-dispatch 按 corpus 强制 |
| cancel reason | `RequestCancelReason::CONTRACT_H` 已冻结 9 项 | 不变（本包引用） | M-request |
| WebSocket/Spawn request 分支 | 已存在 | 归 C-model-connection / C-model-spawn | 不在此实现 |

## 8. §5.4 contract pack 必填项

### 8.1 owner / invariant

- Owner：`skiff-runtime-transport`（frame DTO/codec，canonical owner）+
  `RequestDispatcher`（帧序列状态机与 correlation，C-dispatch 冻结）。
- Invariant：`request.start` 是普通 unary/stream 请求唯一入口；任何
  response 帧必须对应 dispatcher 持有的同一 requestId 且来自同一 session
  socket（exact fence）；stream 帧序错误 / payload presence 违规 / stale
  response 不产生业务副作用；wire 层 fail closed，无 fallback。

### 8.2 typed inputs / outputs

- Inputs：`RuntimeAssemblyRequestStartFrameHeader`（Http）、
  `RequestCancelFrameHeader`、`ResponseStartFrameHeader`、
  `ResponseChunkFrameHeader`、`ResponseEndFrameHeader`、
  `ResponseErrorFrameHeader`（+ payload bytes）。
- Outputs：typed response events（`OrdinaryResponseEvent` /
  `ResponseStreamEvent`）、`RequestCancel`、decoded opaque
  `OpaqueServiceError`；wire 层输出帧（encode 方向）由 W-model 的
  `response_mapper` 既有函数构造。

### 8.3 capacity

- 单帧 payload 长度上限：connection family 的
  `CONNECTION_REQUEST_MAX_PAYLOAD_BYTES`（1 MiB）冻结用于
  WebSocketJsonRpc 变体；HTTP 变体 body 上限由连接级 ingress budget
  （C-session/W-session 冻结，本包引用不重复）。
- requestId / 并发 pending 容量归 C-dispatch（`maxConcurrency`），本包不
  定义 mailbox。

### 8.4 queue full

- wire 层无 mailbox；帧写入失败（writer queue full / socket error）由
  C-session writer queue 契约处理：不等待 close frame，abort exact session；
  dispatcher 将写失败映射为 `callback_error` terminal（C-dispatch §3.3）。

### 8.5 timeout / disconnect / replacement / shutdown terminal

- 本包冻结 wire 贡献：`request.cancel` 是超时/客户端断开/backpressure/
  protocol_error/router_shutdown 时 Router→Runtime 的取消帧，reason 按
  §4 词表；response 终态（end/error）与 runtime disconnect 不发送 cancel 帧。
- deadline（`deadline.timeoutMs`/`expiresAt`）与 dispatcher timeout、
  disconnect/replacement/shutdown 的 pending terminal 归 C-dispatch。

### 8.6 health fields

- wire 层不新增 health 字段；可观测计数归 C-dispatch/W-dispatch：
  pending unary/stream、by-source terminal 计数、cancel 帧计数、
  protocol_error 计数。

### 8.7 fake seam

- `FakeFrameSender`（可注入写失败）、`FakeRuntimePeer`（按 frames.json 逐帧
  收发字节）、`FakeClock`（推进 deadline）。corpus 测试直接消费 fixtures +
  reference wire 状态机；W-model/W-dispatch 必须用同一 fixtures 通过真实
  codec。

### 8.8 real boundary probe（定义，W-model-request/W-dispatch 执行）

- `codec-live:request-wire`：对真实 `decode_runtime_assembly_request_start_frame`
  与 `decode_response_error_frame` / `decode_typed_binary_frame` 消费
  frames.json + reject-cases.json 全部正负例，byte-exact roundtrip；该探针
  在 `request_wire_corpus.rs` 中已对真实 codec 执行（codec 级真实边界）。
- `router-live:dispatch`（W-dispatch 交付后成为 `router-rust-dispatch-live`
  managed probe）：loopback 启动真实 listener，fake runtime peer 按
  request-wire corpus 收发字节，断言 unary/stream/cancel 的 wire 字节与
  terminal 行为与 corpus 一致（定义见 C-dispatch §8.8）。

## 9. W-model-request 交付义务（非本包实现）

1. 消费本 corpus：全部正例帧 decode/encode byte-exact roundtrip；全部
   reject 负例 fail closed。
2. 实现 `request.cancel` payload-empty codec 强制与
   `response.end` phase 强制（翻转 `currentEnforced`）。
3. 实现 frame 级 direction/payload presence 强制，供
   `RuntimeFrameDemux`/sink 装配。
4. 不改变既有 cross-system corpus 字节与 `runtime_assembly_request` 既有
   单元测试语义。
