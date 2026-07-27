# Gateway / Runtime Adapter 边界契约

日期：2026-06-21

更新：2026-07-27。第一版WebSocket明确为**Skiff主动发送的通用双向transport**：外部peer主动发起的业务
请求统一使用HTTP；WebSocket负责连接建立、连接身份/policy、服务端单向通知，以及Skiff向精确connection
发起并等待peer响应的request/response。Request identity与pending state由编码无关的平台broker拥有；
第一版内置`jsonrpc-2.0-text`编码配置，peer response直接恢复原Skiff调用，不形成新的service ingress。
第一版仍不存在用户可声明的`receive`、client-initiated业务消息selector、typed message handler或消息级
gateway identity。旧版相关设计已撤回，不保留兼容surface。

本文定义 router 中 gateway、runtime 注册/调度和 gateway adapter 的长期内部边界。它是目标态架构契约，不是用户可见语言规范，也不是迁移 checklist。当前实现偏差和落地步骤见 `../implementation/gateway-runtime-adapter-refactor.md`。

Skiff 尚未发布，本文不要求兼容历史 manifest 字段、std.websocket 字段或 router 协议别名。旧字段在迁移切片中应 fail closed 或直接删除。

## 范围

本文负责：

- HTTP / WebSocket gateway 与 runtime 关系的模块边界。
- HTTP adapter参数与WebSocket connect回调的目标模型。
- WebSocket business identity、connection policy、主动发送和平台request/response broker的归属。
- router、runtime、compiler 在 payload codec 和 type/schema metadata 上的职责。
- `RuntimeRegistry`、`RuntimeDispatcher`、runtime endpoint 的目标边界。

本文不负责：

- HTTP route 语法、service.yml 语法和用户参考文档。
- 具体代码重排步骤。
- Sample、Sample 等具体业务服务的业务身份模型。
- runtime 内部 `RuntimeValue` 布局。

## 术语

### Gateway

Gateway 是 router 的外部协议入口。HTTP gateway 处理 HTTP socket、route 选择、body 限制、CORS、HTTP response 写回。WebSocket gateway 处理 upgrade、物理连接生命周期、close、Skiff主动发送和连接索引。其专用request broker为Skiff主动发起的request维护有界pending table；它不为peer主动发送的业务消息建立buffer或dispatch。

Gateway 可以理解平台事实：

- HTTP method、path、query、headers、cookies、body bytes。
- WebSocket connection id、upgrade request、control/data frame类别和close状态。
- 已选择WebSocket RPC编码配置的控制字段；第一版是JSON-RPC 2.0的`jsonrpc`、opaque string `id`、
  `method`、`params`、`result`与`error`外形。
- service id、deployment revision、WebSocket entry id、gateway entry key与gateway entry identity。
- request id、deadline、trace、telemetry。
- WebSocket `businessIdentity` 这个 opaque string 作为连接管理 key。

Gateway 不可以理解业务事实：

- `user`、`host`、tenant、device、session principal 等业务 subject kind。
- 业务 request/response record、union、map、representation 的字段布局；第一版配置adapter只把
  `method`、`params`、`result`与error `data`当作opaque JSON值转发。
- 某个业务 cookie/header 是否表示登录身份，除非它只是原样传给业务 connect/pre handler。

### Runtime Endpoint

Runtime endpoint 是 router 对 runtime 暴露的内部 WebSocket listener，通常是 `/runtime`。它负责：

- 接收 runtime 连接。
- 编码/解码 runtime frame envelope。
- 接收 `runtime.register`、`runtime.capabilities`、`runtime.health`、`response.*`、runtime-originated
  `request.start`、`request.cancel`、`connection.send`以及专用的`connection.request` /
  `connection.request.cancel` frame。
- 把WebSocket request broker返回的`connection.response` frame写回原runtime。
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

### WebSocket Request Broker

`WebSocketRequestBroker`是router内Skiff-originated peer request的唯一pending owner。它负责：

- 验证发起runtime、service、WebSocket entry与精确connection归属。
- 生成peer-visible transport request identity，并委托所选编码配置写入frame。
- 将pending绑定到原runtime correlation和物理socket/generation。
- 通过所选编码配置匹配peer response，向原runtime返回`connection.response`。
- 处理cancel、deadline、socket/runtime disconnect、容量上限与短期settled tombstone。

Broker核心不解释JSON或binary业务payload，不选择service handler、不创建runtime ingress、不解码业务
类型，也不与`RuntimeDispatcher`共享service-call / gateway-dispatch pending state。编码配置adapter只
拥有frame framing与控制字段校验；新增binary RPC必须增加独立adapter并显式定义版本、codec与协商规则，
不能把任意binary frame自动当作RPC。

### Gateway Adapter

Gateway adapter 是 runtime 侧的入口适配逻辑。它把 gateway 提供的平台 metadata 和 payload bytes 组装成用户 handler 参数。

HTTP typed JSON route、raw HTTP route和WebSocket connect是第一版全部gateway adapter场景。
WebSocket peer response由平台broker处理，不被分派为runtime request，也没有message adapter。

这些external ingress entry由`service.yml`拥有，不是`serviceCalls`从`api.yml` public graph选择的
service-call operation。Handler、
pre和guard可以是当前Package中的非public callable；compiler直接解析其精确callable identity和linked
signature。External selector绑定owner-local `gatewayEntryKey`，entry另带用于协议一致性校验的
`gatewayEntryIdentity`；两者都不生成或借用`ContractOperationId`，也不进入
`ServiceProtocolIdentity`。若同一source function同时公开为service call，两个surface仍各自拥有独立
identity与校验。

`gatewayEntryIdentity`只覆盖external protocol surface：entry kind、外部参数来源、公开
request/response/stream shape与影响gateway wire兼容性的metadata。HTTP identity覆盖HTTP wire surface；
WebSocket identity覆盖connect request/result shape、允许的Skiff主动发送frame类别、固定平台
request/response envelope版本及connection policy shape，不包含业务`method`或payload协议。
Identity不包含handler/pre/guard callable identity、source selector、Package build或业务实现体。
具体callable binding和内部执行plan由ServiceDeployment及其revision覆盖；只替换实现不会伪装成
external protocol变化。

### Business Identity

`businessIdentity` 是业务 connect handler 返回给 gateway 的 opaque string。Gateway 只用它做连接索引、fan-out、connection policy 和下行投递，不知道它代表 user、host、browser session、native app、actor 还是其它业务主体。

`identity` 这个旧名字不再作为目标态字段名使用。

## 目标模块边界

目标依赖方向：

```text
HttpGateway
  -> RuntimeDispatcher
      -> RuntimeRegistry
      -> RuntimeFrameSender

WebSocketGateway
  -> WebSocketRequestBroker
      -> RuntimeFrameSender

RuntimeEndpoint
  -> RuntimeRegistry
  -> RuntimeDispatcher
  -> WebSocketGatewayOutbound
  -> WebSocketRequestBroker
```

更具体地说：

- Gateway 构造 `request.start` 的平台 header 和 opaque payload bytes。
- Gateway 调 `RuntimeDispatcher.dispatch(...)`。
- `RuntimeDispatcher` 从 `RuntimeRegistry` 选择 runtime connection。
- `RuntimeDispatcher` 通过 `RuntimeFrameSender` 发 runtime frame。`RuntimeFrameSender` 由 runtime endpoint 实现，但 dispatcher 不依赖具体 endpoint class。
- `RuntimeRegistry` 保存 runtime connection handle、capability 和 routing index，不直接发 frame。
- Runtime response 回到 `RuntimeEndpoint` 后，由 `RuntimeDispatcher` 完成 pending request。
- `connection.send` 回到 `RuntimeEndpoint` 后转给 WebSocket gateway 的 outbound handler。
- `connection.request`回到`RuntimeEndpoint`后转给`WebSocketRequestBroker`。Broker验证source runtime、
  service、entry与精确connection的归属，生成peer-visible request id并保存原runtime correlation。
- Peer response先由`WebSocketGateway`交给broker；精确匹配后，broker通过`RuntimeFrameSender`向原runtime
  返回`connection.response`，不经过`RuntimeDispatcher`创建新的service request。

禁止的依赖：

- Gateway 不直接读写 `RuntimeRegistry.pending`。
- Gateway 不调用业务 payload codec。
- `RuntimeDispatcher`的service dispatch pending与`WebSocketRequestBroker`的peer response pending是
  两个不同owner，不能共用一张无类型pending map。
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
- Raw WebSocket message、message body和business identity都不是用户`adapterArgs` source；第一版没有
  WebSocket message handler。

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
- WebSocket connect没有`guard` / `pre`，也不存在可复用connect `adapterArgs`的消息阶段。

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
type WebSocketConnectResult discriminator "tag" =
  { tag: "accept", businessIdentity: string?, connectionPolicy: WebSocketConnectionPolicy? }
  | { tag: "reject", code: integer, reason: string }
```

Runtime WebSocket adapter解码用户connect result。Gateway只接收平台connect result metadata：

```ts
type WebSocketConnectResponseMetadata =
  | {
      result: 'accept';
      businessIdentity?: string;
      connectionPolicy?: WebSocketConnectionPolicy;
    }
  | {
      result: 'reject';
      code: number;
      reason: string;
    };
```

没有connect handler时，**不向runtime发connect dispatch**：gateway直接合成accept（无
`businessIdentity`、无`connectionPolicy`），省去一次runtime往返。需要business identity或connection
policy的entry必须声明connect handler。

### WebSocket主动发送与request/response

第一版业务方向固定为：

```text
client / host business request
  -> HTTP gateway
  -> runtime HTTP adapter
  -> user HTTP handler

user code
  -> std.websocket outbound send
  -> runtime connection.send
  -> WebSocket gateway
  -> client / host

user code
  -> std.websocket.requestJsonToConnection
  -> runtime connection.request
  -> WebSocket request broker
  -> host / peer
  -> JSON-RPC 2.0 response
  -> WebSocket request broker
  -> runtime connection.response
  -> resume original user call
```

只有精确匹配pending request、且符合当前编码配置的response data frame会被broker消费；它不产生runtime
ingress dispatch。Peer主动发送的request、notification或其它text/binary data frame以close code
`1003`关闭。畸形或伪造的profile response以`1002`关闭。WebSocket ping、pong和close control frame由协议栈处理，
不进入用户代码。由此：

- `service.yml`没有`receive`、message entry、selector、envelope或fallback authoring；
- shared artifact、compiler、router和runtime没有WebSocket business-message operation；
- WebSocket拥有编码无关的outbound request/response broker和一个内置JSON文本配置，但不拥有用户可声明的
  client-initiated request/response route或entry-local typed message schema；
- Agine、AIHub及其它service的业务上行必须使用HTTP；HTTP server stream仍可承担流式响应。

## WebSocket connection model

WebSocket connect handler可以按普通函数语义挂起；每次upgrade最多创建一次connect dispatch，runtime
等待handler完成后才结束该dispatch，不把它隐式拆成detached work，也不重复执行。连接建立后没有业务
message dispatch。连接关闭时gateway同步移除连接索引；关闭后到达的下行发送按已关闭连接正常失败。

`std.websocket` 的 connection send 操作本身保持非挂起；它只尝试把 frame 交给 gateway，
不等待客户端消费或为慢客户端提供 backpressure await。

`std.websocket.requestJsonToConnection<TRequest, TResponse>`是不同操作。Runtime按`TRequest`编码payload，
并要求顶层结果是JSON object或array；broker通过`jsonrpc-2.0-text`配置向精确connection发送JSON-RPC 2.0
request。调用在等待匹配response时挂起，runtime按`TResponse`解码success `result`。它继承当前execution
deadline/cancel，不提供business-identity fan-out、自动重试、业务幂等或exactly-once。当前deadline产生
`TimeoutError`；ancestor cancellation是不可捕获的结构化控制。Connection/transport、protocol、
resource-limit和peer error投影成`std.websocket.WebSocketRequestError`的封闭分支。请求编码、params
shape与success response typed decode分别遵循`std.json.encode<TRequest>` /
`std.json.decode<TResponse>`并保留`std.json.DecodeError`；broker不参与业务typed decode。

Broker pending规则：

- Runtime transport correlation与peer-visible JSON-RPC string `id`是两个内部层次；broker生成后者并保存到
  原runtime connection/request correlation的映射，业务源码不能读取两者。
- Pending identity至少包含物理socket object或generation、connection id、编码配置id和peer-visible request id；
  response不能跨connection、重连generation或service/entry归属命中。
- 同一connection上的response可以乱序完成；每个pending最多完成一次。
- disconnect或原runtime断开会原子移除相关pending，并尽可能通知另一端取消/失败。
- deadline/cancel先移除pending，再best-effort通过当前配置发送取消；`jsonrpc-2.0-text`使用
  `$/cancelRequest` notification。Broker保留有界、短期
  settled tombstone，以静默丢弃与完成/取消竞态的晚到或重复response；未知且不在tombstone中的
  response id属于协议错误。
- Pending数量、单payload大小以及tombstone数量和生命周期必须受平台limit约束；pending或payload达到上限
  时新request fail closed。Tombstone达到容量时驱逐最旧项，不因settled记录拒绝新request，也不能无界缓存。
- Transport不自动retry。发送后断线不能证明peer是否已经执行，因此有外部副作用的业务仍需自己的
  `idempotencyKey`、attempt identity或补偿规则。

目标std surface：

```skiff
type WebSocketConnectionPolicy {
  maxConnections: integer,
  overflow: "close-oldest" | "reject-new",
  closeCode: integer?,
  closeReason: string?,
}

type WebSocketConnectResult discriminator "tag" =
  { tag: "accept", businessIdentity: string?, connectionPolicy: WebSocketConnectionPolicy? }
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

第一版每个service最多声明一个WebSocket entry；即使只有一个entry，内部索引仍使用完整的
`(serviceId, websocketEntryId, businessIdentity)` key。`std.websocket.sendTextToBusinessIdentity(...)`
和binary等价操作从当前`ActivationContext`取得service deployment，并解析其唯一WebSocket entry id。
没有active service、没有WebSocket entry或artifact损坏地产生多个entry时，发送fail closed。发送不要求
当前执行起源于WebSocket connect，因此HTTP handler、service call、actor或其它携带同一
`ActivationContext`的执行都可以下发。

Runtime 获取当前 entry id 的规则：

- Gateway dispatch WebSocket connect时必须在request header中携带当前connection entry key/identity。
- Runtime request context 保存当前 WebSocket entry id。
- `std.websocket.sendTextToBusinessIdentity(...)` / `sendBinaryToBusinessIdentity(...)`优先使用connect
  request中的entry id；其它执行从当前service deployment解析唯一entry。
- 第一版不暴露字符串entry id参数，也不允许service声明多个WebSocket entry。以后确有多entry需求时，
  必须先设计可静态校验的entry reference，而不是让gateway按path或字符串猜。

Connection policy validation:

- `maxConnections` 必须是正整数。
- `overflow` 必须是 `"close-oldest"` 或 `"reject-new"`。
- `closeCode` 缺省时使用 `1008`；存在时必须是 WebSocket application-acceptable close code。
- `closeReason` 缺省时使用平台默认原因；存在时必须满足 WebSocket close reason 字节长度限制。
- Reject-new 使用同一 close code/reason 返回给新 socket；close-oldest 使用同一 close code/reason 关闭被移除的旧 socket。

Gateway不定义`ConnectionSubjectKind`。业务connect handler自行从完整request判断身份，只向gateway返回
opaque `businessIdentity`；gateway不知道它表示user、host、tenant或其它主体。

## Payload 和 schema 边界

长期边界：

- Runtime 拥有业务 payload encode/decode。
- Router 转发 opaque bytes或opaque JSON值，**不解析任何业务类型表示**，既不用 JsonSchema 也不用单独的类型 descriptor。WebSocket编码配置adapter可以验证控制字段并提取opaque payload；broker核心不把JSON或任何未来binary codec写死，也不得按业务schema解释payload字段。
- 业务类型的权威表示是 compiler 产出、runtime 加载的 linked program 类型（`TypeRefIr` / `LinkedTypeRef`）。runtime HTTP adapter 从 linked program 取 handler 参数/响应类型构造 `RuntimeTypePlan` 做 payload codec（`from_linked(&params[index].ty, …)`）。WebSocket connect只使用固定平台metadata；outbound request/response codec来自`requestJsonToConnection<TRequest, TResponse>`调用点的concrete类型，不来自service entry manifest。**runtime payload codec 不依赖 manifest。**
- JsonSchema 保留给外部协议校验、文档、diagnostics 和 HTTP JSON contract，不作为 runtime 二进制 payload codec 的 source of truth，也不进入 router 的 dispatch 决策。

Compiler只为adapter plan中真正连接external ingress source/sink的值派生entry-local schema：typed HTTP
body、query/path/header参数和HTTP response。WebSocket没有client-initiated业务message schema；outbound
request/response的concrete codec由调用点类型决定，不进入`service.yml`或gateway entry schema。
Pre产生的业务context和guard内部值不属于外部协议。私有named type可以贡献结构，但其Skiff
source/public/nominal名字不自动出现在schema；entry-local component key必须由canonical external shape或显式
external documentation metadata产生，不能借用Package public identity。

关于"router 不理解业务类型"，这里要明确一个分界：

- **External entry的协议身份是平台事实，router可以知道。** 例如某个entry是`rawHttp`、
  `typedJson`还是`websocketConnect`，以及它的`gatewayEntryIdentity`。这些是字符串标签，router用来
  寻址和分流，不需要展开业务类型结构。
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
- `GatewayEntryIdentity`只按external protocol surface计算；具体target和内部type/codec plan由当前
  ServiceDeployment绑定并在runtime admission时精确校验。

目标态 router production code 不解析业务类型，也不引用 Skiff business payload codec。任何业务 payload 的 encode/decode 都在 runtime adapter 内完成。

### HTTP关联与业务ID

一次HTTP request天然只对应自己的unary response或server stream，transport已经拥有精确关联、取消和
trace metadata。业务payload、HTTP response envelope和stream item不得再声明只用于模拟WebSocket
req/res correlation的`requestId`、`correlationId`或同义字段。Router/runtime内部request id只用于
dispatch、cancel、telemetry和diagnostics，不投影为用户业务字段。

真正拥有独立业务生命周期的ID仍然合法，但必须按语义命名和验证，例如：

- 可重试mutation使用`idempotencyKey`；
- 异步任务/轮询使用`jobId`；
- 业务run使用`runId`；
- 已持久化资源使用其资源ID。

不能为了兼容旧WebSocket envelope把这些ID统一命名为`requestId`。第一版HTTP server stream不复用一条
response multiplex多个独立业务请求，因此stream event也不需要transport correlation id。

上述限制只针对HTTP业务schema。平台WebSocket request/response必须拥有transport request identity；
第一版由broker生成并作为JSON-RPC string `id`发送，仅用于同一connection/generation与编码配置内配对，
不进入handler参数、返回类型、持久化状态或业务日志模型。

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

Downlink：

```text
user code calls std.websocket.sendTextToBusinessIdentity(...)
  -> runtime emits connection.send target { serviceId, websocketEntryId, businessIdentity }
  -> RuntimeEndpoint
  -> WebSocketGateway
  -> matching sockets
```

Request/response：

```text
user code calls std.websocket.requestJsonToConnection<TRequest, TResponse>(connectionId, method, value)
  -> runtime encodes value and emits connection.request
  -> WebSocketRequestBroker validates exact service/entry/connection ownership
  -> broker generates an opaque string id and selects jsonrpc-2.0-text
  -> profile adapter sends one JSON-RPC 2.0 request object
  -> peer returns one JSON-RPC 2.0 response object on the same socket
  -> broker matches connection/generation/profile/id and emits connection.response to the original runtime
  -> runtime decodes TResponse and resumes the original call
```

Application notifications may use business identity fan-out or exact connection id；request/response只允许精确
connection id，因为多个socket不能共同拥有一个unary response。

## Validation

Authoring/deployment manifest readers must fail closed:

- 旧 WebSocket `bind` field 非法。
- 新authoring中的用户`receive`、`websocketReceive`、`websocket.receiveEvent`、
  `websocket.message`与`websocket.messageBody`非法。
- `service.yml`不能声明WebSocket request/response route；平台outbound request能力不产生新的gateway
  entry、handler selector或entry-local schema。
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
- Connect accept payload containing legacy `context`、`contextCodec`或`contextPayloadPresent` fields is invalid.

WebSocket request broker validation must fail closed:

- `connection.request`的source runtime必须已注册目标service，并且entry与connection必须精确属于该
  service/deployment允许的WebSocket surface。
- `connection.request`必须选择已注册编码配置；第一版仅有`jsonrpc-2.0-text`。Profile adapter必须精确
  验证JSON-RPC 2.0 request/response shape，peer不能选择或覆盖平台request id；第一版拒绝batch。
- Request `params`必须是object或array；response `id`必须是与pending精确相等的string；error `code`
  必须是integer，`message`必须是string，`data`可省略或为任意受限JSON值。
- Response必须来自原物理socket/generation并命中pending或短期settled tombstone；未知response id、错误
  connection/generation和畸形envelope不得恢复任何runtime调用。
- Pending、payload size和settled tombstone都必须受配置limit约束；tombstone饱和时驱逐最旧项。

Because Skiff is unreleased, no compatibility aliases are required.

## Observability

Router telemetry may log:

- service id、deployment revision、gateway entry key与gateway entry identity。
- connection id。
- 平台WebSocket request的method、完成状态、deadline/cancel原因和pending计数；request id只按平台
  diagnostic策略记录，payload不得记录。
- presence of `businessIdentity` and a redacted/hash form if needed.
- connection policy decision。
- adapter source kind names。

Router telemetry must not log business context fields unless a business service explicitly logs them inside runtime.

## Verification contract

Target-state tests must prove:

- HTTP adapter args和WebSocket connect adapter args使用同一种`param + source`结构。
- Gateway 拒绝旧 `bind` / `handlerArgs` / `identity` / `scope` fields。
- 用户raw `receive`和任何WebSocket message source不能进入新artifact。
- Runtime adapter可以整体传递HTTP context；WebSocket connect不产生或保存业务connection context。
- `businessIdentity` fan-out works.
- `maxConnections=1, close-oldest` removes old sockets from fan-out before closing them.
- `maxConnections=1, reject-new` leaves old sockets active and rejects the new socket.
- Fan-out 和 policy 按 `(serviceId, websocketEntryId, businessIdentity)` 建 key，并有意忽略 version/build。
- Matching JSON-RPC response frame resumes exactly one pending call and produces no runtime ingress request.
- Peer-initiated request/notification和其它unsolicited text/binary data frame以`1003`关闭；
  malformed/forged profile response以`1002`关闭；ping/pong/close remains protocol-owned.
- Out-of-order responses match correctly；wrong connection/generation/unknown id不能恢复调用；cancel、
  deadline、runtime disconnect和socket disconnect清理pending；late/duplicate settled response只命中
  bounded tombstone并被丢弃；tombstone饱和驱逐最旧项而不拒绝新request。
- `connection.send`保持non-suspending；`requestJsonToConnection`等待response时是suspension point，
  并能由execution cancel/deadline终止；deadline产生`TimeoutError`，ancestor cancellation不可捕获。
- JSON-RPC success/error、非法params、batch拒绝、peer error与`$/cancelRequest`best-effort发送都有协议测试；
  broker状态机测试不依赖JSON字段，并证明未来配置无需复制pending owner。
- Router production code does not import business payload codec, does not parse `parameters[].type` / `responseType`, and makes no dispatch decision from business type structure.
- Runtime adapter tests cover typed HTTP body/context与WebSocket connect request arg construction。
- Outbound send can resolve the sole WebSocket entry from a non-WebSocket execution carrying the current
  service `ActivationContext`; zero/multiple entry states fail closed.
