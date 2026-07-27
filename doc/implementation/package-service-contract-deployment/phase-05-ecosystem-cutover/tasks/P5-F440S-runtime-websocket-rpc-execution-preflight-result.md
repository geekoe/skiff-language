# P5-F440S Runtime WebSocket RPC execution preflight result

状态：`BLOCKED_BY_W0 / INBOUND_RUNTIME_ASSEMBLY_WIRE_MISSING`。

本节点只读检查F440Q后的E0真实入口。没有修改文件、运行测试或启动服务。结论：artifact DTO和outbound
`connection.*` wire已存在，但Router→runtime的inbound runtimeAssembly WebSocket JSON-RPC request/response
DTO完全缺失；E0不能安全越过transport owner实现它。

## 1. 已有owner

已有artifact/compiler surface：

- `GatewayAdapterKind::WebSocketJsonRpc`；
- `GatewayWebSocketRpcProfile::JsonRpc2_0Text`；
- WebSocket JSON-RPC protocol surface；
- params/business identity/connection id sources；
- compiler projection、identity与deployment validation。

已有outbound runtime→Router wire与Host：

- `connection.request`；
- `connection.request.cancel`；
- `connection.response`；
- F440Q已接current Router session/writer、registry、cancellation、deadline与exact ordinary error owner。

## 2. 缺失的shared wire

Rust runtimeAssembly request enum当前只有HTTP与`WebSocketConnect`；decoder把所有
`routing.ingress.protocol == "webSocket"`都解释成connect，没有按`method: null|string`区分。
response mapper只有connect end header，没有WebSocket JSON-RPC outcome。

TypeScript mirror同样只有`"http" | "websocketConnect"`，不存在：

- `RuntimeAssemblyWebSocketJsonRpcRequest`；
- `RuntimeAssemblyWebSocketJsonRpcResponse`；
- dispatcher metadata或strict response outcome。

因此必须先建立窄W0：

- request.start sibling `websocketJsonRpc`；
- response.end sibling `websocketJsonRpc`；
- Rust/TypeScript exact DTO与strict decoder/encoder parity。

W0属于共享transport，不实现handler lookup、typed codec、Host dispatch、broker或gateway。

## 3. W0之后的E0 owner

W0完成后E0拥有：

- `runtime/request` JSON-RPC target/execution；
- `runtime/eval` linked params/result codec；
- Host request-entry dispatch/outcome/cancel；
- loader method-bearing admission；
- generation pin sibling method resolver；
- handlerless method-only WebSocket synthetic accept/acquire。

R0b随后拥有Router immutable method table、真实RuntimeDispatcher/broker/gateway hookup与socket lifecycle。

## 4. 关键执行不变量

### Handlerless method-only eager pin

只要physical WebSocket entry拥有connect handler或非空method table，就必须在attach前完成generation
acquire/receipt。没有connect handler时Host合成默认accept，但仍先pin；path-only且无handler/method的entry
保持不触发runtime。

### Old generation

method resolver只能从pin持有的`Arc<ActiveAssembly>`及其physical route解析sibling method，校验同一
deployment、host/path、protocol、physical `WebSocketEntryId`与`method=Some`。不得重查current active
assembly。

### Cancel/disconnect

peer cancel/disconnect由broker先detach/tombstone并abort；Host cancellation terminal不调用response
encoder。JSON-RPC runtime outcome只有
`success | invalidParams | internalError | deadlineExceeded`，不增加`cancelled`。

## 5. 依赖顺序

```text
W0 shared inbound runtimeAssembly wire
  -> E0 loader/pin/typed execution/Host outcome
  -> R0b Router dispatcher/gateway hookup
  -> fixture/tooling
```

当前唯一执行决策是恢复窄W0，而不是授权E0同时修改transport并把TS mirror留到R0b。该选择保持F440B的
共享owner和Rust/TS成对验证。
