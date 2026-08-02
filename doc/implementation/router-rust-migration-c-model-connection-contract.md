# Router Rust Migration C-model-connection：client WS wire 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-model-connection / W-WebSocket /
E-ws 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.4
  （`ClientSocketGeneration` 独立 newtype）、§3.8（boundedness：frame/byte
  permit、immutable opaque payload）、§5.3（C-model-connection lane →
  W-model-connection → M-connection）、§5.4（contract pack 必填项）、
  §7（E-ws：numeric id 词法验证与 canonicalize、business params/result/
  error 保持 lexical opaque slice）。
- 父批次：`doc/implementation/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-ws-leaf.md`。
- 同链契约：`router-rust-migration-c-client-lifecycle-contract.md`、
  `router-rust-migration-c-ws-contract.md`。

冲突时以权威设计为准；本文件只冻结契约，不写 production。

## 1. 范围

冻结 Router↔Runtime 的 client WebSocket wire：`connection.request` /
`connection.request.cancel` / `connection.response` 帧（M0 已有
`runtime/transport/src/connection_protocol.rs`）、client WS 身份
`ClientSocketGeneration`、JSON-RPC 2.0 text profile 的 numeric id 词法
验证与 canonicalization、profile 帧/字节预算。不定义 `ClientConnectionIndex`
（C-client-lifecycle）、pin ledger 与 broker（C-ws）。

本契约冻结现有 wire，不新增帧族；新增帧族属于 shared model 变更
（M0 closed registry），不是本 pack 的普通 feature。

## 2. Identity 与 typed 类型（§3.4，冻结）

```text
ClientSocketGeneration { connection_id: ConnectionId, generation: u64 }
ConnectionId             = connection.request.connectionId（wire 固定）
WebSocketRpcProfile      = "jsonrpc-2.0-text"（当前唯一 profile）
OpaquePeerId             = string | safe integer（词法验证后 canonical）
OpaquePayload            = immutable lexical JSON slice（不解析业务 schema）
ConnectionRequestId      = connection.request.requestId
```

- `ClientSocketGeneration` 是独立 newtype；禁止用通用
  `ConnectionGenerationFence` 或裸字符串在 client/Runtime 两个 domain
  之间传递后重新猜 owner。
- `connection_id` + `generation` 唯一标识一个 physical client socket
  世代；business replacement 产生新 generation，旧 generation 的捕获
  writer 不再有资格写新 socket（C-client-lifecycle §4）。
- `OpaquePayload` 是词法 slice；Router 不解码业务 params/result/error
  的 schema。

## 3. connection wire（现有 codec，冻结）

### 3.1 connection.request（Runtime→Router）

Header 字段（serde camelCase，deny_unknown_fields）：

```text
schema_version: "skiff-runtime-frame-v3"
type:          "connection.request"
request_id:    非空、无控制字符、≤ 1024 字节
service_id:    非空、无控制字符、≤ 1024 字节
websocket_entry_id: WebSocketEntryId（sha256 v1 identity）
connection_id: 非空、无控制字符、≤ 1024 字节
profile:       "jsonrpc-2.0-text"
method:        非空、≤ 256 字节
deadline?:     { timeout_ms: u64（1..=MAX_SAFE_INTEGER）, expires_at: RFC3339 }
```

Payload：非空 JSON object 或 array，UTF-8，≤ 1 MiB
（`CONNECTION_REQUEST_MAX_PAYLOAD_BYTES`）。

### 3.2 connection.request.cancel（Runtime→Router）

```text
schema_version: "skiff-runtime-frame-v3"
type:          "connection.request.cancel"
request_id:    非空、≤ 1024 字节
reason:        RequestCancelReason（现有 cancel_reason codec）
payload:       必须为空
```

Cancel 只按 requestId 终止 exact outbound correlation（C-ws §4.3），
cancel 不携带 connection_id（correlation 由 broker 的 runtime key
持有）。

### 3.3 connection.response（Router→Runtime）

```text
schema_version: "skiff-runtime-frame-v3"
type:          "connection.response"
request_id:    非空、≤ 1024 字节
outcome:       success | deadlineExceeded | connectionUnavailable
               | transportUnavailable | protocolError | resourceLimit | remote
remote?:       { code: i64（safe integer）, message: 非空 ≤ 4096 字节,
                 data_present: bool }
payload:       ≤ 1 MiB
```

Payload/remote 组合规则：

| outcome | payload | remote |
| --- | --- | --- |
| success | 必须非空 | 禁止 |
| remote | data_present == payload 非空；payload 为 JSON | 必须 |
| deadlineExceeded / connectionUnavailable / transportUnavailable / protocolError / resourceLimit | 必须为空 | 禁止 |

### 3.4 现有 codec 的既有约束（冻结，不重排）

- 所有帧走 skiff binary frame（magic/version/JSON header + optional
  payload），`decode_typed_binary_frame` 严格解码。
- connection family 属 M0 `RuntimeFrameFamily::Connection`，direction
  Either、payload presence Optional；本 wire 按 3.1–3.3 的 per-frame
  规则进一步收紧。
- 帧目录 `runtime/transport/testdata/client-ws/frames.json` 冻结
  byte-exact 正例与负例（负例由 mutation 测试覆盖）。

## 4. websocket.generation.lifecycle wire（现有 codec，冻结）

M0 已冻结 `runtime/transport/src/websocket_generation_lifecycle.rs`，
本 pack 只把行为引用进契约，不重复定义：

```text
type: "websocket.generation.lifecycle"
Acquire: Runtime→Router（sender=runtime）
Release: Router→Runtime（sender=router）
Ack/Reject: 方向与 sender 按 operation（acquire→Router、release→Runtime）
tuple: { router_session_id, service_id, assembly_identity,
         assembly_generation, websocket_entry_id, connection_id }
Reject code: generation-unavailable | not-acquired | request-conflict
             | sender-mismatch | tuple-mismatch
```

- response 必须 exact-echo operation/request_id/tuple
  （`assert_websocket_generation_lifecycle_response_matches`）。
- tuple 的 service/assembly/websocket-entry/connection identity 校验沿用
  现有严格校验；corpus 正例与 mutation 负例见
  `client-ws/frames.json`。

## 5. JSON-RPC 2.0 text 词法契约（§7 E-ws，冻结）

### 5.1 分类规则

Router 对 peer text frame 只做词法分类，不解析业务 schema：

1. 帧必须是 UTF-8 文本，≤ `maxTextBytes`（1 MiB）；否则 close 1009。
2. 必须是单个 JSON object；array/scalar/重复成员 → `invalidRequest`
   （id=null）。
3. response 候选：含 `result` 或 `error`，或含 `id` 且无 `method`；
   否则 request/notification 候选：含 `method` 且无 `result`/`error`。
4. request/notification 只允许 `jsonrpc`/`id`/`method`/`params` 字段；
   `jsonrpc` 必须精确 `"2.0"`；method 必须非空字符串；params 必须
   object 或 array。
5. notification（无 id）：不 dispatch，不产生 terminal；仅
   `observeNotification` 诊断，observer 失败不影响 broker 状态。
6. response：`result`/`error` 恰有一个；id 必须非空字符串；error 为
   object 且只含 `code`/`message`/可选 `data`；code 必须是 safe
   integer；message 非空。非法 response → close 1002。
7. `platformError`：id=null 时写 `-32700/-32600` 错误；id 合法时先
   tombstone 再写错误（C-ws §4.5）。

### 5.2 numeric id：lexeme 验证后 canonicalize

peer id 仅两种：非空字符串，或数值（词法验证 safe integer 后
canonicalize）。数值 id 按 lexeme（原始拼写）处理，不先经浮点
round-trip：

- lexeme 匹配 `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?` 之外拒绝；
- 数值必须精确等于 JavaScript safe integer（±9,007,199,254,740,991）；
  不能精确表示的（如 `1.5`、`9007199254740992`、`1e-324`、
  `1.0000000000000000001`）拒绝；
- 先 canonicalize 再使用：`1e0` → `1`、`-0` → `0`（`-0.0e+3` 等其它
  零拼写同样 → `0`）；指数/小数展开后 canonical 十进制整数；
- canonical 后的 id 用于 peer key（`n:<value>`）与编码 terminal；
- response id 不接受 numeric（仅字符串）；numeric response id →
  invalid response → close 1002。

corpus：`runtime/transport/testdata/client-ws/jsonrpc-ids.json`，含
`1e0`/`-0` 正例与全部负例。

### 5.3 词法预算（frame/byte budget）

| 常量 | 默认值 |
| --- | --- |
| maxTextBytes | 1 MiB |
| maxJsonDepth | 64 |
| maxJsonNodes | 100,000 |
| maxStringBytes | 64 KiB |
| request payload | ≤ 1 MiB |
| method | ≤ 256 字节 |
| token（requestId/serviceId/connectionId/…） | ≤ 1024 字节 |
| remote message | ≤ 4096 字节 |

超出任一预算 → close 1009 或 `resourceLimit`/`protocolError` 终态
（按 §5.1/§5.4 分派）。

## 6. §5.4 contract pack 必填项

### 6.1 owner / invariant

- Owner：shared `skiff-runtime-transport` connection codec +
  `connection_protocol.rs` / `websocket_generation_lifecycle.rs` 模块；
  peer 词法分类归 `WebSocketRpcProfile`（当前 `jsonrpc-2.0-text`）。
- Invariant：wire 字节精确；方向/sender/payload 组合严格校验；
  numeric id 按 lexeme 验证并 canonicalize，不引入浮点噪音；opaque
  payload 不被 Router 解码。

### 6.2 typed inputs / outputs

- Inputs：`ConnectionRequestFrameHeader` + payload bytes、
  `ConnectionRequestCancelFrameHeader`、`ConnectionResponseFrameHeader` +
  payload bytes、`WebSocketGenerationLifecycleControl`、peer text frame。
- Outputs：对应 strict codec 的 typed header + payload、`OpaquePeerId`
  （canonical）、`ProfileAction`、codec 错误（`BinaryFrameError`）。

### 6.3 capacity

- 单帧：payload ≤ 1 MiB；peer text ≤ 1 MiB；method ≤ 256B；token ≤
  1024B；remote message ≤ 4096B；profile 节点/深度/字符串预算见 §5.3。
- 无帧数上限在 codec 层（队列/连接级上限归 C-client-lifecycle/C-ws）。

### 6.4 queue full

- codec 不排队；超预算/超深/超节点直接产生对应 terminal（close 1009
  或 error frame）。队列满语义归 owner（C-client-lifecycle/C-ws）。

### 6.5 timeout / disconnect / replacement / shutdown terminal

- deadline 字段验证：`timeout_ms` 必须 1..=MAX_SAFE_INTEGER、
  `expires_at` 必须 RFC3339（UTC 或带 offset，真实日历日期）；非法 →
  `protocolError`。deadline 的 broker 行为归 C-ws。
- disconnect/replacement/shutdown 不改变 wire 字节；它们触发 owner 的
  terminal（C-client-lifecycle/C-ws），wire 只保证“已发出帧字节不可变”。

### 6.6 health fields

- codec 层无状态，不发布 health；本 pack 冻结以下可观测量由 owner
  聚合：per-generation outbound/inbound pending、tombstone、writer
  frame/byte 占用、释放 ACK 计数（见 C-client-lifecycle/C-ws health）。

### 6.7 fake seam

- `FakeProfileAdapter`（可注入 limits/clock 的 JSON-RPC 词法分类）、
  `FakeConnectionSocket`（text/binary/close 记录）、`FakeRuntimeSource`
  （captured respond 记录）、`FakeClock`；corpus 参考模型在
  `runtime/transport/tests/client_ws_corpus.rs` 等 test-only 文件。

### 6.8 real boundary probe（定义）

- 真实 client WS socket → Router → fake Runtime：完成 admit/acquire/
  outbound RPC roundtrip，发送 `1e0`/`-0` id 请求并断言 canonical 终态
  帧字节；发送超预算/非法 id 帧并断言 close 码。该 probe 由
  W-WebSocket/E-ws 执行，成为 `router-rust-ws-live` 的一部分。
