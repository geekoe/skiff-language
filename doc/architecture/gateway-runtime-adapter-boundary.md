# Gateway / Runtime Adapter 边界契约

日期：2026-06-21

更新：2026-07-26。WebSocket raw frame `receive`被重新归类为平台transport阶段，不再是用户可声明的
service业务入口。本文关于连接生命周期、opaque context、generation pin、发送与队列的约束仍然有效；
旧版把`websocketReceive`、`websocket.receiveEvent`或`websocket.messageBody`直接绑定到一个用户
`receive` handler的设计已撤回。业务消息入口必须与HTTP业务route处于同一抽象层，其具体authoring、
selector/envelope、typed handler与identity模型尚待单独冻结。

本文定义 router 中 gateway、runtime 注册/调度和 gateway adapter 的长期内部边界。它是目标态架构契约，不是用户可见语言规范，也不是迁移 checklist。当前实现偏差和落地步骤见 `../implementation/gateway-runtime-adapter-refactor.md`。

Skiff 尚未发布，本文不要求兼容历史 manifest 字段、std.websocket 字段或 router 协议别名。旧字段在迁移切片中应 fail closed 或直接删除。

## 范围

本文负责：

- HTTP / WebSocket gateway 与 runtime 关系的模块边界。
- HTTP adapter参数与WebSocket connect回调的目标模型。
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
- service id、deployment revision、WebSocket entry id、gateway entry key与gateway entry identity。
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

HTTP typed JSON route、raw HTTP route和WebSocket connect是当前已确认的gateway adapter场景。Raw
WebSocket frame receive由平台接收、排队和分派，不直接对应一个用户handler。未来选中的业务消息handler
仍由runtime adapter构造typed参数，但不能把transport receive与业务入口混成同一个entry。

这些external ingress entry由`service.yml`拥有，不是`api.yml`公开的service-call operation。Handler、
pre和guard可以是当前Package中的非public callable；compiler直接解析其精确callable identity和linked
signature。External selector绑定owner-local `gatewayEntryKey`，entry另带用于协议一致性校验的
`gatewayEntryIdentity`；两者都不生成或借用`ContractOperationId`，也不进入
`ServiceProtocolIdentity`。若同一source function同时公开为service call，两个surface仍各自拥有独立
identity与校验。

`gatewayEntryIdentity`只覆盖external protocol surface：entry kind、外部参数来源、公开
request/response/stream shape与影响gateway wire兼容性的metadata。HTTP identity已经冻结；WebSocket
identity必须等业务消息入口层级确定后再冻结，不能用`connect/receive`两个transport phase代替。Identity不包含
handler/pre/guard callable identity、source selector、Package build或业务实现体。具体callable binding、
内部类型与codec execution plan由ServiceDeployment及其revision覆盖；只替换实现不会伪装成external protocol
变化。

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
  kind: 'typedJson' | 'rawHttp' | 'websocketConnect';
  handler: GatewayAdapterCallable;
  guard?: GatewayAdapterCallable;
  pre?: GatewayAdapterCallable;
  adapterArgs: GatewayAdapterArg[];
};
```

`param` 是 runtime handler 参数名。`source` 是 gateway/platform 提供的标准值，不是字符串路径。

HTTP response shape由`kind`与linked handler signature共同确定，不能只看返回类型是否写成
`Stream<T>`：`typedJson`只允许unary return，由runtime wrapper编码一个JSON response；
`rawHttp`允许单个`std.http.HttpResponse`，或精确的
`Stream<std.http.HttpResponseStreamEvent>`。只有后一种产生external HTTP server-stream frames。
其它`typedJson + Stream<T>`或`rawHttp + Stream<非HttpResponseStreamEvent>`组合必须在compiler projection
阶段fail closed。

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
  | { kind: 'websocket.connectionId' };
```

规则：

- 不支持任意字符串路径。
- 不支持 `identity` 或 `connection.identity`。
- 不支持 `query.foo`、`header.foo`、`cookie.foo` 作为 handler 参数绑定。业务需要这些值时，应接收完整 request 并在业务代码中解析。
- `http.context` 是 HTTP `pre` 或 adapter pipeline 产生的业务对象，gateway 只能整体传递。
- connect accept产生的业务connection context由gateway opaque保存；它不是connect handler输入，也不作为
  authoring source逐字段暴露。
- Raw WebSocket message、message body、business identity和connection context都不是当前用户
  `adapterArgs` source。未来消息handler的参数模型必须随业务消息路由设计一起冻结。

Source 合法阶段：

| Source | HTTP typed | HTTP raw | WebSocket connect |
| --- | --- | --- | --- |
| `http.request` | 可用 | 可用 | 不可用 |
| `http.body` | 可用 | 不可用 | 不可用 |
| `http.context` | 有 `pre` 时可用 | 有 `pre` 时可用 | 不可用 |
| `websocket.connectRequest` | 不可用 | 不可用 | 可用 |
| `websocket.connectionId` | 不可用 | 不可用 | 可用 |

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
- WebSocket connect本轮没有`guard` / `pre`。未来业务消息入口若需要guard/pre或可配置参数，必须在其
  抽象冻结时显式定义，不能复用connect的`adapterArgs`。

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

HTTP CORS 的所有权按 literal path 决定。若选中的 service/version/build 为该 path 声明了显式
`OPTIONS` route，router 必须把预检 dispatch 给 runtime，且不得为该 path 的普通响应预先注入
或覆盖 CORS header；service 返回的 exact Origin 策略是唯一结果。没有显式 `OPTIONS` route 的
path 继续使用 router 的自动预检与兼容 CORS header。

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
  contextCodecIdentity: string;
};
```

`contextCodec` 对 gateway 完全 opaque——gateway只把它原样保存，并在平台选中某个业务消息handler后随
该次dispatch回传，从不读它的字段。它存在的唯一目的，是让runtime校验“这份context bytes确实由当前
deployment中与该消息handler兼容的connect codec产生”。`contextCodecIdentity`由compiler生成的精确
context type/codec plan计算，属于deployment adapter execution plan，不是`ContractOperationId`、
service operation ABI或external `GatewayEntryIdentity`。

WebSocket消息路由冻结后，deployment必须为每个需要context的业务消息entry记录与connect一致的
`contextCodecIdentity`，runtime在decode前比较，不匹配fail closed；gateway不参与该比较。这个内部事实
不要求现在冻结一个用户可见的receive manifest或`WebSocketContextExpectation` DTO。

Accept response 中，`contextPayloadPresent = true` 时 `response.end.payloadBytes` 是已编码 connection context，且 `contextCodec` 必填；否则 context 为 null，且 `contextCodec` 不出现。`contextPayloadPresent = false` 只在没有 connect handler、或 connect context 类型接受 null 时合法；非 nullable `Context` 必须产生 context bytes。这个 nullability 校验由 runtime WebSocket adapter 在投影 platform metadata 前完成。Gateway 只校验 `result`、`businessIdentity`、`connectionPolicy`、close code/reason、context byte presence 和 `contextCodec` presence 是否一致。Gateway 把 `contextBytes` 和 `contextCodec` 当作 opaque connection state 保存，绝不解码。

没有connect handler时，gateway保存的connection context是null，不保存`contextCodec`。业务消息模型冻结
后，compiler必须拒绝“没有connect handler但消息handler需要non-null context”的entry；runtime也必须在
损坏artifact或frame下fail closed。

没有 connect handler 时，**不向 runtime 发 connect dispatch**：gateway 直接合成 accept（context = null、无 `businessIdentity`、无 `connectionPolicy`、无 `contextCodec`），省去一次 runtime 往返。runtime 不存在的 connect 行为不应被 round-trip。需要 `businessIdentity` / connection policy / context 的 entry 必须声明 connect handler。

### WebSocket业务消息入口：待冻结

Raw frame receive是平台内部阶段。目标链路只能先冻结到：

```text
client frame
  -> gateway接收、限流、排队并保持connection/generation
  -> 平台消息层解码并选择一个业务消息entry
  -> runtime按该entry的linked signature构造typed参数
  -> 用户业务消息handler
```

用户不声明一个接收全部frame的`receive` handler。业务消息handler才与HTTP的`createUser`一类route对等。
现有AIHub以`type`字段分发，Agine以`eventName`字段分发，证明平台若要接管这一步，必须先冻结消息协议，而
不是简单隐藏原回调。

根因是HTTP已经标准化了每次请求的`method + path`，而WebSocket只在HTTP Upgrade握手时有path；连接建立后
的frame没有标准业务route。`Sec-WebSocket-Protocol`只能为整条连接选择协议，不能选择单个frame对应的
业务handler。因此平台若要提供业务级消息入口，就必须额外定义一层Skiff application-message routing
protocol；没有这层约定时，单一raw `receive`是唯一不猜业务语义的通用接口。

后续设计至少要同时回答：

- 使用平台统一envelope，还是由entry显式声明discriminator字段/值，或从typed literal union派生；
- text JSON、binary和无法decode的frame分别允许什么；
- unknown message由平台关闭连接、发送固定错误，还是交给显式fallback entry；
- connection context如何进入每个typed handler，以及各handler是否各自拥有key/identity；
- requestId、ack和response correlation是否属于可选平台协议；下行主动push仍由显式send完成。

这些问题冻结前，`service.yml`、shared artifact、compiler和runtime不得新增
`websocketReceive`、`websocket.receiveEvent`、`websocket.message`或
`websocket.messageBody`用户surface。旧实现可以作为迁移事实继续存在，但不是目标设计依据。

## WebSocket connection model

WebSocket connect和未来的业务消息handler可以挂起；例如一个消息handler在处理期间顺序消费
上游 stream，并通过非挂起的 `connection.send` 发送多个下行事件。挂起不改变 ingress 的
unary 边界：每个入站 connect 或 message 仍只创建一次 dispatch，Runtime 等待该 handler
完成后才结束本次 dispatch，不把它隐式拆成 detached work，也不重复执行。

同一物理连接同时最多有一个业务消息dispatch处于active状态。后续消息按到达顺序进入
有界队列，只有前一条 operation 完成后才开始下一条，从而使 operation 内的挂起不会改变
消息顺序。连接关闭时 gateway 移除连接索引、丢弃尚未开始的排队消息，并终止与该连接绑定的
active transport dispatch。该关闭属于整个 ingress request 的生命周期收尾，不是对 operation
暴露独立 cancel handle；operation 仍声明 `NotCancellable`，active dispatch 只结算一次，也不会
因为关闭而重新 dispatch。关闭后才到达 gateway 的下行发送会按已关闭连接正常失败。

`std.websocket` 的 connection send 操作本身保持非挂起；它只尝试把 frame 交给 gateway，
不等待客户端消费或为慢客户端提供 backpressure await。

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

- Gateway dispatch WebSocket connect时必须在request header中携带当前connection entry key/identity。未来
  business-message dispatch还必须携带选中的message entry key/identity；两层具体命名与identity组合待
  消息路由设计冻结，不能只复用一个raw receive identity。
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
- 业务类型的权威表示是 compiler 产出、runtime 加载的 linked program 类型（`TypeRefIr` / `LinkedTypeRef`）。runtime adapter 从 linked program 取 handler 参数/响应类型构造 `RuntimeTypePlan` 做 payload codec——HTTP typed body 已经这样（`from_linked(&params[index].ty, …)`）；未来typed WebSocket业务消息handler也必须复用该机制。**runtime payload codec 不依赖 manifest。**
- JsonSchema 保留给外部协议校验、文档、diagnostics 和 HTTP JSON contract，不作为 runtime 二进制 payload codec 的 source of truth，也不进入 router 的 dispatch 决策。

Compiler只为adapter plan中真正连接external source/sink的值派生entry-local schema。当前冻结的是typed
HTTP body、query/path/header参数和HTTP response；未来typed WebSocket业务消息也属于该闭包，但必须等
消息路由模型冻结后再投影。Pre产生的业务context、guard内部值和WebSocket connection context不属于外部
协议。私有named type可以贡献结构，但其Skiff
source/public/nominal名字不自动出现在schema；entry-local component key必须由canonical external shape或显式
external documentation metadata产生，不能借用Package public identity。

关于"router 不理解业务类型"，这里要明确一个分界：

- **External entry的协议身份是平台事实，router可以知道。** 例如某个route是`rawHttp`还是
  `typedJson`、它的`gatewayEntryIdentity`以及opaque`contextCodecIdentity`。这些是字符串标签，router
  用来寻址、分流或原样保存，不需要展开业务类型结构。
- **类型的字段布局是业务事实，router 不持有。** 某个业务 record 有哪些字段、union 有哪些分支、怎么编解码——只有 runtime 知道。router 看到的永远是 opaque bytes。

因此 router 不需要、也不应引入一个能描述任意业务类型结构的 closed-vocabulary descriptor。早期方案里的 `RuntimeTypeDescriptor`（让 compiler/runtime/router 三处都 parse 同一份类型 JSON）是不必要的，并且会引入第三份必须逐字节对齐的类型编码，叠加到已有的 `TypeRefIr` 和 build-id 投影上。它不进入本契约。

生成的gateway protocol manifest：

```ts
type GatewayParameterManifest = {
  name: string;
  schema?: JsonSchema; // compiler-derived; external protocol/docs/diagnostics only
};

type GatewayEntryProtocolManifest = {
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
  kind: GatewayAdapterKind;
  mode: DispatchMode;
  parameters: GatewayParameterManifest[];
  responseSchema?: JsonSchema; // compiler-derived; external protocol/docs/diagnostics only
};
```

类型表示规则：

- manifest **不**携带业务payload codec用的类型。runtime adapter从linked program取handler参数/响应
  类型（`TypeRefIr` → `RuntimeTypePlan`）做编解码。manifest里的schema由compiler从同一精确handler
  signature和adapter source生成，只供外部协议、文档与diagnostics；`service.yml`不得手写重复业务schema。
- 因此router看到的entry参数只有name和display schema，二者都不用于选择业务callable。router不解析业务
  类型结构，也不需要`TypeRefIr`。
- `GatewayEntryIdentity`只按external protocol surface计算；具体target、内部type/codec plan和
  `contextCodecIdentity`由当前ServiceDeployment绑定并在runtime admission时精确校验。

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

业务消息：

```text
client frame
  -> WebSocketGateway
  -> platform message decode/select（authoring与协议待冻结）
  -> RuntimeDispatcher business-message request
  -> runtime typed message adapter
  -> user business-message handler
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

Authoring/deployment manifest readers must fail closed:

- 旧 WebSocket `bind` field 非法。
- 新authoring中的用户`receive`、`websocketReceive`、`websocket.receiveEvent`、
  `websocket.message`与`websocket.messageBody`在消息路由设计冻结前非法。
- 迁移后的旧 HTTP `handlerArgs` field 非法。
- `adapterArgs[].source` unknown kind is invalid.
- Any business context field binding is invalid.
- Router只消费compiler-derived external schema视图，不接收`parameters[].type`/`responseType`业务类型
  descriptor；任何要求router展开业务类型结构的manifest形态都不该存在。

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

- service id、deployment revision、gateway entry key与gateway entry identity。
- connection id。
- presence of `businessIdentity` and a redacted/hash form if needed.
- connection policy decision。
- adapter source kind names。

Router telemetry must not log business context fields unless a business service explicitly logs them inside runtime.

## Verification contract

Target-state tests must prove:

- HTTP adapter args和WebSocket connect adapter args使用同一种`param + source`结构。
- Gateway 拒绝旧 `bind` / `handlerArgs` / `identity` / `scope` fields。
- 用户raw `receive`以及尚未冻结的WebSocket message source不能进入新artifact。
- Runtime adapter可以整体传递HTTP context；gateway可以opaque保存WebSocket connection context bytes；
  gateway路径不检查业务字段。
- `businessIdentity` fan-out works.
- `maxConnections=1, close-oldest` removes old sockets from fan-out before closing them.
- `maxConnections=1, reject-new` leaves old sockets active and rejects the new socket.
- Fan-out 和 policy 按 `(serviceId, websocketEntryId, businessIdentity)` 建 key，并有意忽略 version/build。
- Gateway opaque保存WebSocket context bytes，并在平台选中业务消息entry后不解码地送回runtime。
- Router production code does not import business payload codec, does not parse `parameters[].type` / `responseType`, and makes no dispatch decision from business type structure.
- Runtime adapter tests cover typed HTTP body/context与WebSocket connect request arg construction。
- Typed WebSocket业务消息参数构造测试在消息路由设计冻结后补充；当前不得用raw receive fixture替代。
