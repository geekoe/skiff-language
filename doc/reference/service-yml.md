# Skiff Service Authoring YAML Reference

## 1. 文件职责

Service首先是Package，因此root仍必须包含`package.yml`与`api.yml`。Service authoring额外使用：

- `service.yml`：service identity与service-to-service callable选择；
- 可选`http.yml`：HTTP external ingress；
- 可选`websocket.yml`：唯一WebSocket connection entry、connect与declared JSON-RPC methods；
- 零个或多个`config.<profile>.yml`：已声明requirement的deployment binding与deployment policy。

`http.yml`或`websocket.yml`只能在同root存在合法`service.yml`时出现。三个authoring文件都不参与
`root.*` namespace，不能被Skiff源码import。`service.yml`中的`http`/`websocket`字段非法；Skiff尚未发布，
不读取旧内联格式。

## 2. service.yml

第一版shape：

```yaml
id: example.com/users

serviceCalls:
  - users.get
  - users.managed
```

测试service还可以按testing reference写`kind: test`。除此以外：

- `id`必填，是稳定service id；version仍来自`package.yml`。
- `serviceCalls`可省略或为空，只能列`api.yml`已有public function或public instance root。
- dependency、public symbol、handler、route、JSON schema、artifact binding和平台limit不在本文件声明。
- timeout、quota、state/resource binding等deployment policy/value由所选`config.<profile>.yml`提供；
  例如`timeout: 30000`投影为可选`DeploymentPolicy.timeoutMs`，不在`service.yml`声明。

只改变`serviceCalls`会改变ServiceContract/ServiceProtocolIdentity，但不改变PackageArtifact。HTTP或
WebSocket文件变化不改变ServiceContract。

## 3. http.yml

`http.yml`顶层就是entry mapping，不再使用`http:`或`routes:`包装：

```yaml
createUser:
  method: POST
  path: /users
  kind: typedJson
  handler: http.createUser
  adapterArgs:
    - param: input
      source: { kind: http.body }

relay:
  method: POST
  path: /relay
  kind: rawHttp
  handler: http.relay
  adapterArgs:
    - param: request
      source: { kind: http.request }
```

规则：

- Mapping key是service-owner-local稳定`GatewayEntryKey`，必须唯一。
- `method`与`path`共同形成external selector，同一service内不得重复。
- `kind`只能是`typedJson`或`rawHttp`。
- `handler`以及可选`pre`/`guard`是当前Package source callable selector，不要求出现在`api.yml`。
- `adapterArgs`按handler参数名绑定标准source；参数名必须唯一并与linked signature精确一致。
- `typedJson`可使用`http.body`、`http.request`及存在`pre`时的`http.context`，只允许unary return。
- `rawHttp`使用`http.request`或`http.context`，返回`std.http.HttpResponse`，或精确返回
  `Stream<std.http.HttpResponseStreamEvent>`。
- `guard`固定接收`std.http.HttpRequest`并在decode/pre前运行；`pre`固定接收
  `std.http.HttpRequest`并产生整个`http.context`。
- 文件可省略；需要显式空surface时写`{}`。空文件解析为`null`，非法。

Compiler把每个record拆成
`IngressSelector -> GatewayEntryKey -> GatewayEntryIdentity`，并从linked handler生成typed adapter plan和
entry-local external schema。作者不能手写重复schema。

## 4. websocket.yml

第一版每个service最多一个WebSocket entry，因此文件本身就是entry：

```yaml
path: /ws

connect:
  handler: websocket.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }

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

顶层只允许`path`、`connect`和`jsonRpc`：

- `path`必填；文件可以只声明path，供Skiff主动send/request使用。
- `connect`可省略。存在时handler返回`std.websocket.WebSocketConnectResult`；可用source只有
  `websocket.connectRequest`与`websocket.connectionId`。
- `jsonRpc`可省略或为空mapping。每个mapping key是稳定`GatewayEntryKey`，`method`是external selector；
  两者在各自namespace内必须唯一。
- 业务method必须是非空string，不能以平台保留前缀`$/`开头。
- JSON-RPC handler第一版没有`guard`/`pre`，只能unary return。
- Handler必须且只能绑定一个完整`websocket.jsonRpcParams`，并可另外绑定平台提供的
  `websocket.connectionId`和`websocket.businessIdentity`。Transport `id`、raw frame、分拆字段路径和
  任意event name都不是source。
- Params的JSON顶层必须是object或array，并按linked handler参数type解码；return按linked return type编码
  为`result`，`void`为`null`。
- `std.websocket.requestJsonToConnection`是outbound host operation，不要求把其method列入`jsonRpc`。
- Raw `sendText*`/`sendBinary*`也不需要method声明；它们不创建inbound handler。

一个peer request按
`(websocket entry id, jsonrpc-2.0-text, method) -> GatewayEntryKey -> GatewayEntryIdentity`
在socket pin住的deployment generation中路由。业务handler看不到transport id。除平台
`$/cancelRequest`外，第一版没有业务notification handler；即使notification的`method`与已声明request
method同名也不dispatch。第一版也不支持JSON-RPC batch或binary RPC。

## 5. 错误与取消

Inbound JSON-RPC固定使用以下platform codes：

| Code | Meaning |
| --- | --- |
| `-32700` | Parse error |
| `-32600` | Invalid Request |
| `-32601` | Method not found |
| `-32602` | Invalid params |
| `-32603` | Internal error |
| `-32000` | Server busy |
| `-32001` | Request timed out |
| `-32800` | Request cancelled |

未捕获Skiff throw统一脱敏为`-32603`，不把名义错误、stack或私有字段发给peer。预期业务失败使用typed result
union。Peer `$/cancelRequest`触发不可捕获的结构化取消；disconnect取消该connection/generation上的全部
inbound request。每个有id的已接纳request最多写一个result/error。Parse、batch或无法识别合法id的Invalid
Request用`id: null`；其余request错误回显原string/safe-integer id。同方向重复active id以`1002`关闭连接；
settled后允许复用，两个方向的同值id始终互不冲突。

## 6. Fail-closed

以下情况必须在authoring/projection阶段失败：

- 普通Package出现`http.yml`/`websocket.yml`，或`service.yml`仍内联HTTP/WebSocket；
- unknown/重复top-level key、entry key、HTTP selector或JSON-RPC method；
- handler无法解析、generic handler、adapter source阶段非法或参数与linked signature不匹配；
- typed JSON/JSON-RPC handler返回`Stream<T>`；
- WebSocket method使用`$/`保留前缀、声明raw receive/notification/event fallback或试图绑定transport id；
- manifest手写业务schema或要求Router按业务type结构选择handler。
