# P5-F440X Router WebSocket RPC hookup preflight result

状态：`PASS / R0B_SPLIT_AND_SHARED_CALLS_PROVEN`。

本节点只读追踪F440W1/F440T/F440R之后的Router真实入口。没有修改文件、运行测试或启动服务。
R0b必须拆成两个顺序节点：

```text
RuntimeDispatcher WebSocket JSON-RPC sibling API
  -> gateway / RuntimeEndpoint / broker / snapshot hookup
```

## 1. Dispatcher当前缺口

- `RuntimeDispatcher`只有`dispatchAssemblyWebSocketConnect`；
- receipt dispatch仍会把JSON-RPC sibling判为unavailable；
- unary response validator把所有`protocol=webSocket`当connect；
- `RuntimeFrameSender`/`RuntimeEndpoint.sendFrame`尚未把F440T sibling纳入executable union；
- method-bearing request会被connect-only type guard误分类。

第一节点只建立receipt-pinned dispatcher runtime leg，不接gateway/broker。

## 2. Gateway/hookup当前缺口

- Gateway只在connect handler存在时dispatch/pin；handlerless method table仍zero acquire；
- peer data一律关闭`1003`，没有进入broker；
- snapshot reader仍只接受method-null/connect及旧deployment/gateway/artifact identity版本；
- RuntimeEndpoint缺source-disconnect callback与Endpoint-owned isolation API；
- WebSocket lifecycle raw send不观察callback failure；
- broker API已经完整，但AbortSignal reason、production limits/timeout owner尚未在hookup冻结。

第二节点必须显式拥有这些shared call-site；只写“gateway/server”不可执行。

## 3. 两个顺序owner

### Dispatcher sibling

生产owner：

- `runtimeDispatcher.ts`
- `runtimeEndpoint.ts`仅widen outbound sender type
- runtimeAssembly request/response type gate module

冻结API：

```ts
dispatchAssemblyWebSocketJsonRpc(
  request,
  timeoutMs,
  exact RuntimeDispatchConnectionReceipt,
  { signal }
) -> typed response
```

receipt只从dispatcher WeakMap取captured socket，不做current registry selection。timeout/abort先detach
pending，再发既有`request.cancel`；late/wrong socket不能完成新pending。

### Gateway/broker hookup

生产owner：

- WebSocket gateway/lifecycle与新bridge；
- broker/broker types；
- RuntimeEndpoint callback/disconnect/isolation；
- server composition；
- current runtime assembly snapshot/readers与新WebSocket snapshot join；
- runtime protocol metadata的integration-only call-site。

它消费dispatcher sibling，不再修改其behavior。

## 4. Hookup冻结策略

这些是current实现策略，不新增public配置/语言语义：

- broker capacity/tombstone/TTL使用`WebSocketRequestBroker`唯一Router-internal default owner；server不得复制
  第二组magic constants；
- inbound runtime timeout取Router `requestTimeoutMs`与captured deployment policy timeout的较小正值；
- AbortSignal reason使用current canonical `RequestCancelReason`：
  peer cancel为caller cancel，peer disconnect/protocol close按已有对应reason；未知reason降级
  `caller_cancel`，不新增wire spelling；
- socket write必须经observed lifecycle writer，callback failure进入broker terminal；
- runtime protocol isolation由`RuntimeEndpoint` owner执行，Gateway只能请求isolate；
- production snapshot reader/identity hard cut属于hookup；cross-system fixture、README/checker属于F0；
- Router direct tests中因current production identity升级失效的call-site不能留ownerless，应在hookup范围机械
  刷新；其它仓库fixture留F0。

## 5. 最终owner矩阵

- handlerless eager pin：Gateway判断`connect handler || method table nonempty`并dispatch connect；Host
  F440U/W1产生synthetic accept与receipt；
- old-generation method table：connection attach时复制immutable map；之后不读current snapshot；
- outbound runtime request/cancel：Endpoint做source trust，bridge做connection join，broker做pending/id/
  deadline/tombstone/peer write与唯一terminal；
- inbound request：profile/broker分类与lease，bridge投影captured method，dispatcher只拥有runtime leg，
  Host执行old handler，broker唯一写peer response；
- notification不进入dispatcher；
- peer cancel/disconnect先由broker detach/abort，generation lifecycle随后release；
- origin runtime disconnect由Endpoint exact-session callback触发broker清该source；
- protocol violation由Endpoint isolate source；
- socket callback failure由lifecycle观察，broker完成terminal；
- shutdown先disconnect broker/release generation，再关闭Endpoint/runtime sessions。

## 6. 依赖

```text
F440Y dispatcher sibling
  -> F440Z gateway/broker/snapshot hookup
  -> F0 fixtures/tooling
  -> combined gate
```

两个节点不能并行修改`runtimeEndpoint.ts`与runtimeAssembly type files。
