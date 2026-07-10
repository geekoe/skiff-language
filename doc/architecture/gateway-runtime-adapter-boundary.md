# Gateway / Runtime Adapter 边界契约

日期：2026-06-21

本文定义 router 中 gateway、runtime 注册/调度和 gateway adapter 的长期内部边界。它是目标态架构契约，不是用户可见语言规范，也不是迁移 checklist。当前实现偏差和落地步骤见 `../implementation/gateway-runtime-adapter-refactor.md`。

Skiff 尚未发布，本文不要求兼容历史 manifest 字段、std.websocket 字段或 router 协议别名。旧字段在迁移切片中应 fail closed 或直接删除。

## 范围

本文负责：

- HTTP / WebSocket gateway 与 runtime 关系的模块边界。
- HTTP `handlerArgs` 和 WebSocket `bind` 的统一目标模型。
- WebSocket connection context、business identity 和 connection policy 的归属。
- router、runtime、compiler 在 payload codec 和 type/schema metadata 上的职责。
- `RuntimeRegistry`、`RuntimeDispatcher`、runtime endpoint 的目标边界。

本文不负责：

- HTTP route 语法、service.yml 语法和用户参考文档。
- 具体代码重排步骤。
- Sample、Sample 等具体业务服务的业务身份模型。
- runtime 内部 `RuntimeValue` 布局。

## 术语

### Gateway

Gateway 是 router 的外部协议入口。HTTP gateway 处理 HTTP socket、route 选择、body 限制、CORS、HTTP response 写回。WebSocket gateway 处理 upgrade、物理连接生命周期、pending message buffer、close、下行写回和连接索引。

Gateway 可以理解平台事实：

- HTTP method、path、query、headers、cookies、body bytes。
- WebSocket connection id、upgrade request、message frame、close 状态。
- service id、version、build id、WebSocket entry id、gateway entry identity、operation target。
- request id、deadline、trace、telemetry。
- WebSocket `businessIdentity` 这个 opaque string 作为连接管理 key。

Gateway 不可以理解业务事实：

- `user`、`host`、tenant、device、session principal 等业务 subject kind。
- 业务 connection context 的字段。
- 业务 request/response record、union、map、representation 的字段布局。
- 某个业务 cookie/header 是否表示登录身份，除非它只是原样传给业务 connect/pre handler。

### Runtime Endpoint

Runtime endpoint 是 router 对 runtime 暴露的内部 WebSocket listener，通常是 `/runtime`。它负责：

- 接收 runtime 连接。
- 编码/解码 runtime frame envelope。
- 接收 `runtime.register`、`runtime.capabilities`、`runtime.health`、`response.*`、runtime-originated `request.start`、`request.cancel`、`connection.send`。
- 将已验证 frame 交给 `RuntimeRegistry` 或 `RuntimeDispatcher`。

Runtime endpoint 不负责选择业务 runtime，也不持有长期 pending request 策略。

Runtime endpoint 拥有物理 runtime WebSocket writer。其它模块只能通过窄接口 `RuntimeFrameSender` 发送 frame，不能直接持有 runtime socket。

### RuntimeRegistry

`RuntimeRegistry` 只保存 runtime 注册状态和可寻址能力：

- runtime connection、runtime id、revision state。
- service/build/target/capability 索引。
- active revision、draining/retained/retired 状态。
- version -> build id 索引。
- runtime capability snapshot。

`RuntimeRegistry` 不负责 request pending map、timeout、cancel、stream response sequencing、service-to-service forward request id 映射。

### RuntimeDispatcher

`RuntimeDispatcher` 是 gateway 和 runtime 之间的内部路由/管理器。它负责：

- 从 `RuntimeRegistry` 选择目标 runtime。
- 发出 `request.start` frame。
- 维护 pending request、deadline、abort/cancel cleanup。
- 处理 unary、frame unary、server stream 的 response lifecycle。
- 处理 runtime-originated service-to-service request forwarding。
- 在 runtime disconnect 时完成 pending request 的失败、取消或转移。

Gateway 依赖 `RuntimeDispatcher`，不直接依赖 `RuntimeRegistry` 做 dispatch。

### Gateway Adapter

Gateway adapter 是 runtime 侧的入口适配逻辑。它把 gateway 提供的平台 metadata 和 payload bytes 组装成用户 handler 参数。

HTTP typed JSON route、raw HTTP route、WebSocket connect、WebSocket receive 都是 gateway adapter 场景。它们应共享同一类 manifest 概念：`adapterArgs`。

`gatewayEntryIdentity` 必须覆盖该 gateway entry 的 platform contract：entry id、handler/pre/guard callable identity、`adapterArgs`、WebSocket context expectation、HTTP typed body/response metadata，以及影响 gateway/runtime adapter frame shape 的字段。它不包含业务实现体。

### Business Identity

`businessIdentity` 是业务 connect handler 返回给 gateway 的 opaque string。Gateway 只用它做连接索引、fan-out、connection policy 和下行投递，不知道它代表 user、host、browser session、native app、actor 还是其它业务主体。

`identity` 这个旧名字不再作为目标态字段名使用。

## 目标模块边界

目标依赖方向：

```text
HttpGateway / WebSocketGateway
  -> RuntimeDispatcher
      -> RuntimeRegistry
      -> RuntimeFrameSender

RuntimeEndpoint
  -> RuntimeRegistry
  -> RuntimeDispatcher
  -> WebSocketGatewayOutbound
```

更具体地说：

- Gateway 构造 `request.start` 的平台 header 和 opaque payload bytes。
- Gateway 调 `RuntimeDispatcher.dispatch(...)`。
- `RuntimeDispatcher` 从 `RuntimeRegistry` 选择 runtime connection。
- `RuntimeDispatcher` 通过 `RuntimeFrameSender` 发 runtime frame。`RuntimeFrameSender` 由 runtime endpoint 实现，但 dispatcher 不依赖具体 endpoint class。
- `RuntimeRegistry` 保存 runtime connection handle、capability 和 routing index，不直接发 frame。
- Runtime response 回到 `RuntimeEndpoint` 后，由 `RuntimeDispatcher` 完成 pending request。
- `connection.send` 回到 `RuntimeEndpoint` 后转给 WebSocket gateway 的 outbound handler。

禁止的依赖：

- Gateway 不直接读写 `RuntimeRegistry.pending`。
- Gateway 不调用业务 payload codec。
- `RuntimeRegistry` 不处理 HTTP/WebSocket 外部协议。
- `RuntimeRegistry` 不知道 WebSocket business identity 连接索引。
- `RuntimeRegistry` 不依赖 concrete `RuntimeEndpoint` 或 gateway。

## Adapter 参数模型

旧 WebSocket `bind: Record<string, string>` 和旧 HTTP `handlerArgs: [{ kind }]` 目标态统一为结构化 `adapterArgs`。

目标 shape：

```ts
type GatewayAdapterArg = {
  param: string;
  source: GatewayAdapterSource;
};

type GatewayAdapterManifest = {
  kind: 'typedJson' | 'rawHttp' | 'websocketConnect' | 'websocketReceive';
  handler: GatewayAdapterCallable;
  guard?: GatewayAdapterCallable;
  pre?: GatewayAdapterCallable;
  adapterArgs: GatewayAdapterArg[];
};
```

`param` 是 runtime handler 参数名。`source` 是 gateway/platform 提供的标准值，不是字符串路径。

HTTP source：

```ts
type HttpGatewayAdapterSource =
  | { kind: 'http.request' }
  | { kind: 'http.body' }
  | { kind: 'http.context' };
```

WebSocket source：

```ts
type WebSocketGatewayAdapterSource =
  | { kind: 'websocket.connectRequest' }
  | { kind: 'websocket.receiveEvent' }
  | { kind: 'websocket.connection' }
  | { kind: 'websocket.connectionContext' }
  | { kind: 'websocket.message' }
  | { kind: 'websocket.messageBody' }
  | { kind: 'websocket.connectionId' }
  | { kind: 'websocket.businessIdentity' };
```

规则：

- 不支持任意字符串路径。
- 不支持 `connection.context.foo`。
- 不支持 `identity` 或 `connection.identity`。
- 不支持 `query.foo`、`header.foo`、`cookie.foo` 作为 handler 参数绑定。业务需要这些值时，应接收完整 request 并在业务代码中解析。
- `http.context` 是 HTTP `pre` 或 adapter pipeline 产生的业务对象，gateway 只能整体传递。
- `websocket.connectionContext` 是 connect accept 产生的业务对象，gateway 只能整体保存和传递。
- `websocket.message` 是 Skiff 平台标准对象 `std.websocket.ConnectionMessage`，携带 text/binary 原始区分。handler 想要平台层 message 时绑定它，runtime adapter 据 `message.encoding` 构造 `ConnectionMessage`。
- `websocket.messageBody` 是 message payload 解成 handler 声明业务类型后的值，与 HTTP 的 `http.body` 对称。handler 写 `receive(msg: ChatMessage)` 时绑定它；gateway 仍只发 message 原始 bytes，runtime adapter 从 linked program 取该参数类型把 bytes 解成业务值。这让 WebSocket 也有 typed handler，而不必每个 handler 都接 `WebSocketReceiveEvent` 再手动 decode。text frame 走 UTF-8 文本 → JSON → 目标类型（与 `http.body` 同机制）。**binary frame 只支持 `bytes` 参数**（原样 bytes）：binary 不解成业务 record/union——要结构化的 binary 消息，接 `websocket.message`（平台 `ConnectionMessage`）或声明 `bytes` 自行解。compiler 在 binary-only entry 绑定非 `bytes` 的 `messageBody` 时报错。`websocket.message` 和 `websocket.messageBody` 不同时表达字段拆分——前者给平台 message，后者给整解后的业务值，二者都是各自完整的 whole value。
- 不恢复 gateway 字符串路径绑定。若将来需要 `text` 这种平台便利参数，由 compiler 生成 runtime adapter wrapper 在 runtime 内从 `websocket.message` 解构，不在 gateway manifest 表达 `message.text`。

Source 合法阶段：

| Source | HTTP typed | HTTP raw | WebSocket connect | WebSocket receive |
| --- | --- | --- | --- | --- |
| `http.request` | 可用 | 可用 | 不可用 | 不可用 |
| `http.body` | 可用 | 不可用 | 不可用 | 不可用 |
| `http.context` | 有 `pre` 时可用 | 有 `pre` 时可用 | 不可用 | 不可用 |
| `websocket.connectRequest` | 不可用 | 不可用 | 可用 | 不可用 |
| `websocket.receiveEvent` | 不可用 | 不可用 | 不可用 | 可用 |
| `websocket.connection` | 不可用 | 不可用 | 不可用 | 可用 |
| `websocket.connectionContext` | 不可用 | 不可用 | 不可用 | 可用 |
| `websocket.message` | 不可用 | 不可用 | 不可用 | 可用 |
| `websocket.messageBody` | 不可用 | 不可用 | 不可用 | 可用 |
| `websocket.connectionId` | 不可用 | 不可用 | 可用 | 可用 |
| `websocket.businessIdentity` | 不可用 | 不可用 | 不可用 | 可用 |

Source 合法性校验 owner：

- Compiler projection 必须只产出当前 adapter kind 合法的 source。
- Router manifest loader 必须 reject direct manifest 中当前 adapter kind 不合法的 source。
- `adapterArgs[].param` 必须唯一，且必须对应 handler 参数名。
- 同一 source 可以绑定给多个不同参数；runtime adapter 对每个参数提供同一个 whole source value。重复 source 不允许表达字段拆分。
- Gateway 可以重复校验 source 阶段合法性作为防御，但不能因为 source 的目标参数类型去解码业务值。
- Runtime adapter 仍必须 fail closed，因为 runtime frame 可能来自旧 router、测试 fixture 或损坏输入。

handler 参数构造属于 runtime adapter。

`adapterArgs` 只描述 handler 参数。`guard` / `pre` 不复用 handler `adapterArgs`：

- HTTP `guard` 固定接收 `std.http.HttpRequest`，在 body decode 和 `pre` 前执行。
- HTTP `pre` 固定接收 `std.http.HttpRequest`，返回 `http.context`。
- HTTP handler 才使用 `adapterArgs` 接收 `http.request`、`http.body`、`http.context` 的组合。
- WebSocket connect/receive 本轮没有 `guard` / `pre`。未来若需要可配置参数，必须显式增加 `guardArgs` / `preArgs`，不能复用 handler `adapterArgs`。

### 示例：HTTP typed JSON

Manifest：

```json
{
  "kind": "typedJson",
  "handler": { "kind": "serviceFunction", "modulePath": "internal.todos", "symbol": "create" },
  "adapterArgs": [
    { "param": "body", "source": { "kind": "http.body" } }
  ]
}
```

Flow：

```text
HTTP gateway
  reads raw body bytes
  sends request.start { httpRequest, httpAdapter } + payload bytes

runtime HTTP adapter
  decodes body using handler body type
  calls handler(body)
  encodes HTTP response metadata + response body bytes
```

Gateway 不解码 body record，也不构造 handler args object。它只把 `httpAdapter.adapterArgs` 和 raw body bytes 发给 runtime。

### 示例：HTTP pre context

Manifest：

```json
{
  "kind": "typedJson",
  "pre": { "kind": "serviceFunction", "modulePath": "internal.account", "symbol": "pre" },
  "handler": { "kind": "serviceFunction", "modulePath": "internal.account", "symbol": "me" },
  "adapterArgs": [
    { "param": "context", "source": { "kind": "http.context" } }
  ]
}
```

`context` 是业务类型。Gateway 不知道它是否包含 `userId`。HTTP context 由 runtime adapter 内的 `pre` 调用产生，并在同一个 HTTP request 生命周期内传给 handler；它不需要 gateway 保存。

### 示例：WebSocket connect

Manifest：

```json
{
  "kind": "websocketConnect",
  "handler": { "kind": "serviceFunction", "modulePath": "internal.socket", "symbol": "connect" },
  "adapterArgs": [
    { "param": "request", "source": { "kind": "websocket.connectRequest" } }
  ]
}
```

Connect request metadata：

```ts
type WebSocketConnectRequestMetadata = {
  connectionId: string;
  url: string;
  query: Array<{ name: string; value: string }>;
  headers: Array<{ name: string; value: string }>;
  cookies: Array<{ name: string; value: string }>;
  version?: string;
  websocketEntryId: string;
  gatewayEntryIdentity: string;
};
```

`websocket.connectRequest` 和 connect 阶段的 `websocket.connectionId` source 都来自这个 metadata。

Connect result：

```skiff
type WebSocketConnectResult<Context> discriminator "tag" =
  { tag: "accept", context: Context, businessIdentity: string?, connectionPolicy: WebSocketConnectionPolicy? }
  | { tag: "reject", code: integer, reason: string }
```

Runtime WebSocket adapter 解码用户 connect result。Gateway 只接收平台 connect result metadata 和 opaque context bytes：

```ts
type WebSocketConnectResponseMetadata =
  | {
      result: 'accept';
      businessIdentity?: string;
      connectionPolicy?: WebSocketConnectionPolicy;
      contextCodec?: WebSocketContextCodec;
      contextPayloadPresent: boolean;
    }
  | {
      result: 'reject';
      code: number;
      reason: string;
    };

type WebSocketContextCodec = {
  kind: 'skiff-runtime-payload';
  contextTypeIdentity: string;
  operationAbiId: string;
};
```

`contextCodec` 对 gateway 完全 opaque——gateway 只把它原样保存并在 receive 时回传，从不读它的字段。它存在的唯一目的，是让 runtime receive adapter 校验"这份 context bytes 确实来自本 entry 的 connect"。`contextTypeIdentity` 是 `Context` 类型的 identity（由现有类型 identity 机制产出，不是新引入的 descriptor identity），指向 `Context` 本身而非整个 `WebSocketConnectResult<Context>`。`operationAbiId` 是产生该 context 的 connect operation ABI id。两者都用既有的类型/operation 身份表达，不需要 router 或 gateway 理解类型结构。

WebSocket entry manifest 必须保存 receive 期望的 context 身份：

```ts
type WebSocketContextExpectation =
  | {
      kind: 'context';
      connectOperationAbiId: string;
      contextTypeIdentity: string;
    }
  | {
      kind: 'null';
    };
```

有 connect handler 时，compiler 从 connect return `WebSocketConnectResult<Context>` 派生 `connectOperationAbiId` 和 `contextTypeIdentity`，两者必须从同一个 lowered connect 返回类型派生（不一致即 compiler bug）。没有 connect handler 时，expectation 是 `{ kind: "null" }`。Runtime receive adapter 在 decode context bytes 前必须比较 `contextCodec.operationAbiId` 和 `contextTypeIdentity` 与 entry expectation；不匹配 fail closed。Gateway 不参与该比较。

Accept response 中，`contextPayloadPresent = true` 时 `response.end.payloadBytes` 是已编码 connection context，且 `contextCodec` 必填；否则 context 为 null，且 `contextCodec` 不出现。`contextPayloadPresent = false` 只在没有 connect handler、或 connect context 类型接受 null 时合法；非 nullable `Context` 必须产生 context bytes。这个 nullability 校验由 runtime WebSocket adapter 在投影 platform metadata 前完成。Gateway 只校验 `result`、`businessIdentity`、`connectionPolicy`、close code/reason、context byte presence 和 `contextCodec` presence 是否一致。Gateway 把 `contextBytes` 和 `contextCodec` 当作 opaque connection state 保存，绝不解码。

没有 connect handler 时，gateway 保存的 connection context 是 null，不保存 `contextCodec`，receive request 不包含 `websocket.context` segment。Compiler 必须拒绝“没有 connect handler 但 receive handler 需要 non-null context”的 entry；runtime adapter 也必须在损坏 manifest 或 frame 下 fail closed。

没有 connect handler 时，**不向 runtime 发 connect dispatch**：gateway 直接合成 accept（context = null、无 `businessIdentity`、无 `connectionPolicy`、无 `contextCodec`），省去一次 runtime 往返。runtime 不存在的 connect 行为不应被 round-trip。需要 `businessIdentity` / connection policy / context 的 entry 必须声明 connect handler。

### 示例：WebSocket receive

Manifest：

```json
{
  "kind": "websocketReceive",
  "handler": { "kind": "serviceFunction", "modulePath": "internal.socket", "symbol": "receive" },
  "adapterArgs": [
    { "param": "event", "source": { "kind": "websocket.receiveEvent" } }
  ]
}
```

业务代码从 event 里读取业务 context：

```skiff
function receive(event: std.websocket.WebSocketReceiveEvent<ConnectionContext>) -> null {
  const context = event.connection.context
  return null
}
```

如果业务想把 handler 写成 `receive(context, message)`，compiler 可以生成 runtime-side wrapper，但 manifest 仍表达为标准 source，而不是 gateway field path。

Receive request 把保存的 context 作为 platform payload segment 带回 runtime。目标协议：

```ts
type RuntimePayloadSegment = {
  name: 'websocket.context' | 'websocket.message';
  offset: number;
  length: number;
  codec?: WebSocketContextCodec;
};

type WebSocketReceiveRequestMetadata = {
  connectionId: string;
  businessIdentity?: string;
  message: { tag: 'text' | 'binary'; encoding: 'utf8' | 'raw' };
  payloadSegments: RuntimePayloadSegment[];
};
```

`request.start.payloadBytes` 是已声明 segment 的串联。Gateway 可以按 segment offset 切分和拼接 bytes，但不解码 `websocket.context`。Runtime adapter 使用 `contextCodec` 解码 `websocket.context` 并构造 `WebSocketReceiveEvent<Context>`。

`websocket.message` segment 的编码：

- text frame 使用原始 UTF-8 frame bytes，metadata 为 `{ tag: "text", encoding: "utf8" }`。Runtime adapter 负责 UTF-8 decode 并构造 text message。
- binary frame 使用原始 binary frame bytes，metadata 为 `{ tag: "binary", encoding: "raw" }`。Runtime adapter 负责映射到当前 `std.websocket.ConnectionMessage` binary representation。
- Gateway 不做业务 JSON decode，也不把 text frame 解释成 route payload。

无论 handler 绑定的是 `websocket.message`（平台 `ConnectionMessage`）还是 `websocket.messageBody`（typed 业务值），gateway 发出的 segment 都相同——同一份原始 message bytes。绑定 `websocket.messageBody` 时，runtime adapter 从这份 message bytes、按 linked program 里 handler 参数的类型解成业务值：text frame 按 UTF-8 → JSON → 目标类型；binary frame 只接受 `bytes` 参数并原样给出，不解成业务 record/union。gateway 不参与该选择，也不知道结果类型。

Payload segment validation:

- `offset + length` 必须落在 `payloadBytes` 范围内。
- segment 不允许重叠。
- 同名 segment 最多出现一次。
- receive 阶段必须有 `websocket.message` segment；有 non-null context 时必须有 `websocket.context` segment。
- `websocket.context` segment 必须携带 `contextCodec`；没有 context 时不得出现 `websocket.context` segment。
- Gateway 负责生成合法 segment table；runtime adapter 必须重复校验并 fail closed。

## WebSocket connection model

目标 std surface：

```skiff
type WebSocketConnection<Context> {
  id: string,
  businessIdentity: string?,
  context: Context,
}

type WebSocketConnectionPolicy {
  maxConnections: integer,
  overflow: "close-oldest" | "reject-new",
  closeCode: integer?,
  closeReason: string?,
}

type WebSocketConnectResult<Context> discriminator "tag" =
  { tag: "accept", context: Context, businessIdentity: string?, connectionPolicy: WebSocketConnectionPolicy? }
  | { tag: "reject", code: integer, reason: string }
```

Connection policy 规则：

- `connectionPolicy` 只在 `businessIdentity` 存在时合法。
- policy key 是 `(serviceId, websocketEntryId, businessIdentity)`。
- `scope` 字段不存在。policy 挂在 connect accept 上，作用域天然是本次业务连接身份。
- `overflow = "close-oldest"` 时，gateway 接受新连接，并在新连接进入 business identity fan-out 前同步移除旧连接索引，再关闭旧 socket。
- `overflow = "reject-new"` 时，gateway 保留现有连接并拒绝新连接。
- 未返回 `connectionPolicy` 时，多个同一 `businessIdentity` 连接仍可 fan-out。
- 当 `maxConnections > 1` 且新连接会超过上限时，`close-oldest` 按 verified-at/accepted-at 从旧到新移除足够多的旧 socket，直到包含新 socket 后总数不超过 `maxConnections`；`reject-new` 只拒绝新 socket，不移除任何旧 socket。
- version 和 build id 不进入 policy key。这样同一 service 的滚动构建或本地 reload 后，新连接仍能替换同一业务身份的旧连接。
- `websocketEntryId` 进入 key，避免同一 service 将不同 WebSocket entry 的连接互相 fan-out 或互相踢掉。当前只有一个 entry 的服务也必须按这个完整 key 建索引。

Downlink fan-out key 与 policy key 相同：

```ts
type WebSocketBusinessDeliveryTarget = {
  serviceId: string;
  websocketEntryId: string;
  businessIdentity: string;
};
```

`std.websocket.sendTextToBusinessIdentity(...)` 由 runtime 填入当前 WebSocket entry id。若未来允许没有当前 WebSocket entry 上下文的后台任务发送到 business identity，compiler/runtime 必须要求显式 entry id，不能让 gateway 猜。

Runtime 获取当前 entry id 的规则：

- Gateway dispatch WebSocket connect/receive 时必须在 request header 中携带 `websocketEntryId` 和 `gatewayEntryIdentity`。
- Runtime request context 保存当前 WebSocket entry id。
- `std.websocket.sendTextToBusinessIdentity(...)` / `sendBinaryToBusinessIdentity(...)` 只能在有当前 WebSocket entry context 的 request 内省略 entry id。
- 没有当前 WebSocket entry context 的后台任务、process 或普通 service call 需要显式 entry id API；否则编译或 runtime fail closed。

Connection policy validation:

- `maxConnections` 必须是正整数。
- `overflow` 必须是 `"close-oldest"` 或 `"reject-new"`。
- `closeCode` 缺省时使用 `1008`；存在时必须是 WebSocket application-acceptable close code。
- `closeReason` 缺省时使用平台默认原因；存在时必须满足 WebSocket close reason 字节长度限制。
- Reject-new 使用同一 close code/reason 返回给新 socket；close-oldest 使用同一 close code/reason 关闭被移除的旧 socket。

Gateway 不定义 `ConnectionSubjectKind`。业务可以在自己的 `Context` 中放 `userId?`、`hostIdHash?`、`tenantId?` 等字段，并在业务代码中判断。

## Payload 和 schema 边界

长期边界：

- Runtime 拥有业务 payload encode/decode。
- Router 转发 opaque bytes，**不解析任何业务类型表示**，既不用 JsonSchema 也不用单独的类型 descriptor。
- 业务类型的权威表示是 compiler 产出、runtime 加载的 linked program 类型（`TypeRefIr` / `LinkedTypeRef`）。runtime adapter 从 linked program 取 handler 参数/响应类型构造 `RuntimeTypePlan` 做 payload codec——HTTP typed body 已经这样（`from_linked(&params[index].ty, …)`），WebSocket 同此。**runtime payload codec 不依赖 manifest。**
- JsonSchema 保留给外部协议校验、文档、diagnostics 和 HTTP JSON contract，不作为 runtime 二进制 payload codec 的 source of truth，也不进入 router 的 dispatch 决策。

关于"router 不理解业务类型"，这里要明确一个分界：

- **类型的名字/身份是平台事实，router 可以知道。** 例如某个 route 是 `rawHttp` 还是 `typedJson`、某个 operation 的 `operationAbiId`、某个 connect context 的类型 identity。这些是字符串标签，router 用来寻址、分流、校验同源，不需要展开结构。
- **类型的字段布局是业务事实，router 不持有。** 某个业务 record 有哪些字段、union 有哪些分支、怎么编解码——只有 runtime 知道。router 看到的永远是 opaque bytes。

因此 router 不需要、也不应引入一个能描述任意业务类型结构的 closed-vocabulary descriptor。早期方案里的 `RuntimeTypeDescriptor`（让 compiler/runtime/router 三处都 parse 同一份类型 JSON）是不必要的，并且会引入第三份必须逐字节对齐的类型编码，叠加到已有的 `TypeRefIr` 和 build-id 投影上。它不进入本契约。

目标 operation manifest：

```ts
type OperationParameterManifest = {
  name: string;
  schema?: JsonSchema; // display / external-protocol / diagnostics only
};

type OperationManifest = {
  operation: string;
  operationAbiId: string;
  target: string;
  mode: DispatchMode;
  parameters: OperationParameterManifest[];
  responseSchema?: JsonSchema; // display / external-protocol / diagnostics only
};
```

类型表示规则：

- manifest **不**携带业务 payload codec 用的类型。runtime adapter 从 linked program 取 handler 参数/响应类型（`TypeRefIr` → `RuntimeTypePlan`）做编解码。manifest 里的 `schema` 是注解过的 `JsonSchema`，仅供外部协议/文档/diagnostics。
- 因此 router 看到的 operation 参数只有 `name` 和 display `schema`，二者都不用于 dispatch 决策。router 不解析业务类型结构，也不需要 `TypeRefIr`。
- `operationAbiId` 沿用现有计算口径（参数名 + 参数/返回类型 + mode/target 等），不因为本次重构改变 ABI 输入；JsonSchema 本来就不进 ABI。
- runtime 侧某个类型需要的"身份"（例如 connect context 同源校验，见下）用现有的类型 identity / `operationAbiId` 表达，不引入新的 descriptor identity domain。

目标态 router production code 不解析业务类型，也不引用 Skiff business payload codec。任何业务 payload 的 encode/decode 都在 runtime adapter 内完成。

## HTTP flow

```text
client
  -> HttpGateway
  -> RuntimeDispatcher
  -> runtime HTTP adapter
  -> user handler / pre / guard
  -> runtime HTTP adapter
  -> RuntimeDispatcher
  -> HttpGateway
  -> client
```

Gateway responsibilities:

- route selection。
- request body byte limit。
- `httpRequest` metadata。
- deadline、trace、telemetry。
- HTTP response socket write。

Runtime adapter responsibilities:

- typed body decode。
- pre/guard execution。
- handler arg construction from `adapterArgs`。
- handler response encoding。
- `httpResponse` platform metadata projection。

## WebSocket flow

Connect：

```text
client upgrade
  -> WebSocketGateway pending connection
  -> RuntimeDispatcher connect request
  -> runtime WebSocket adapter
  -> user connect handler
  -> accept/reject
  -> WebSocketGateway verifies connection
```

Receive：

```text
client frame
  -> WebSocketGateway
  -> RuntimeDispatcher receive request
  -> runtime WebSocket adapter
  -> user receive handler
```

Downlink：

```text
user code calls std.websocket.sendTextToBusinessIdentity(...)
  -> runtime emits connection.send target { serviceId, websocketEntryId, businessIdentity }
  -> RuntimeEndpoint
  -> WebSocketGateway
  -> matching sockets
```

Gateway may also support direct connection id sends as low-level diagnostics/control, but application-level delivery should use `businessIdentity`.

## Validation

Manifest readers must fail closed:

- 旧 WebSocket `bind` field 非法。
- 迁移后的旧 HTTP `handlerArgs` field 非法。
- `adapterArgs[].source` unknown kind is invalid.
- Any business context field binding is invalid.
- Router manifest reader 把 `parameters[].type` / `responseType` 当 opaque 转发，不解析其内部结构；任何要求 router 展开业务类型结构的 manifest 形态都不该存在。

Runtime connect response validation must fail closed:

- `connectionPolicy` without `businessIdentity` is invalid.
- `identity` and `connection.identity` are invalid field names.
- `scope` inside `WebSocketConnectionPolicy` is invalid.
- `maxConnections`、`overflow`、`closeCode`、`closeReason` must satisfy the connection policy rules above.
- Accept response with `contextPayloadPresent = true` but missing context bytes is invalid.
- Accept response with context bytes but `contextPayloadPresent = false` is invalid.
- Accept response with `contextPayloadPresent = true` but missing `contextCodec` is invalid.
- Accept response with `contextPayloadPresent = false` but present `contextCodec` is invalid.
- Runtime adapter must reject `contextPayloadPresent = false` for a non-nullable Context type before gateway sees the response.

Because Skiff is unreleased, no compatibility aliases are required.

## Observability

Router telemetry may log:

- service id、version、build id、gateway entry identity、operation target。
- connection id。
- presence of `businessIdentity` and a redacted/hash form if needed.
- connection policy decision。
- adapter source kind names。

Router telemetry must not log business context fields unless a business service explicitly logs them inside runtime.

## Verification contract

Target-state tests must prove:

- HTTP adapter args and WebSocket adapter args use the same manifest shape.
- Gateway 拒绝旧 `bind` / `handlerArgs` / `identity` / `scope` fields。
- `connection.context.foo` 不能在 manifest 中表达。
- Runtime adapter 可以整体传递 HTTP context；gateway 可以携带 WebSocket connection context bytes；gateway 路径不检查业务字段。
- `businessIdentity` fan-out works.
- `maxConnections=1, close-oldest` removes old sockets from fan-out before closing them.
- `maxConnections=1, reject-new` leaves old sockets active and rejects the new socket.
- Fan-out 和 policy 按 `(serviceId, websocketEntryId, businessIdentity)` 建 key，并有意忽略 version/build。
- Gateway opaque 保存 WebSocket context bytes，并在 receive 时不解码地送回 runtime。
- Router production code does not import business payload codec, does not parse `parameters[].type` / `responseType`, and makes no dispatch decision from business type structure.
- Runtime adapter tests cover typed body, context, connect request and receive event arg construction.
- Runtime adapter tests cover `websocket.messageBody` typed handler arg construction (message bytes 解成业务类型) 以及 `websocket.message` 平台 `ConnectionMessage` 构造，二者来自同一 message segment。
