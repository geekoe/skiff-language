# P5-F445H I7 M6 AIHub G2 后诊断账本

## 范围与基线

- 可集成 Skiff 基线：`51487de4733bf42ee4f97d75695e5eaa2b5e381c`
  （tree `b18fa9e7357d723a7fff2de3ac3401fa769e01ff`）。
- Internals：`9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）。
- 官方 packages：`b06d7aaf16b6914837de1f74920fd3f626040472`
  （tree `fb9db28a7d1bd3babafd1dfa7a23687e393ff856`）。
- 目标：`agine.ai/aihub` 测试服务默认发现的 51 个隔离测试。
- 排除：`defaultRun false` 的 live test；未使用真实密钥，未访问外网。

诊断运行临时叠加了 S3 的环境开关 trace，诊断分支提交为
`8b174dba`，来源提交为 `bfec329d1bc55113865074871544d0b098e6702e`。
二者只增加 trace，不是 production fix，也不属于本账本提交的祖先，禁止集成。

所有 case 共用 assembly：

```text
skiff-runtime-assembly-v3:sha256:6a3d51895ecd6a8b5d02c965204ae847af69582faafb5f2fc51e357ae8d74ce1
```

## 结果

默认 51 个测试完整执行：**47 pass，4 fail，0 skip**。

- M5 的 34 个 `unsupported native target std.json.encode`：**归零**。
- M5 case 28/29 的 `unknown Stream value`：**归零**，两条 case 均通过。
- S3 审计 case 28/29 的 runtime/scope 28、29 后确认：
  `stream-0..3` 创建、查询、结束完整，`active=0` 后关闭 scope/owner；
  判定为 `NO_PRODUCTION_CHANGE`。
- 剩余四条失败只有同一错误：

```text
HTTP 409:
{"error":{"code":"AssemblyActivationRejected","message":"std.http.emitResponseStream used outside a raw HTTP streaming response context"}}
```

## 剩余四条 case

共同测试源码：
`aihub/service-tests/internal/aihub_service.test.skiff`。

| 默认序号 | runtime trace | case | 测试源码 |
| ---: | ---: | --- | --- |
| 44 | 45 | `chat events HTTP route returns structured event body` | `1129-1171` |
| 45 | 46 | `chat event stream preserves per-item chunk order and full event projection` | `1173-1223` |
| 46 | 47 | `chat event stream keeps emitted items before each post-start failure` | `1225-1249`，helper `307-311` |
| 48 | 49 | `chat event stream consumer break cancels the provider ancestor chain` | `1311-1323` |

### 是否经过 raw HTTP route

四条失败的实际测试调用都没有经过 Router/raw HTTP entry。

1. 序号 44 虽然名称含 `HTTP route`，但测试在 `1152-1156` 直接调用
   `subjectImpl/internal.aihub_service.handleAihubEventsHttp(...)`，再由
   `collectHttpEventStream` 消费返回流。它调用的是生产 handler，但绕过了
   Router 和 raw HTTP adapter，因此当前执行上下文没有 response stream sink。
2. 序号 45、46、48 直接调用测试可见的
   `subjectImpl/internal.aihub_service.streamChatEventResponseForTest(...)`；
   它们既不经过 Router，也不经过生产 handler。

生产服务的 `aihub/service/http.yml` 确实有两条 raw HTTP entry：

```yaml
v1ChatEventsPost:
  method: POST
  path: /v1/chat/events
  kind: rawHttp
  handler: internal.aihub_service.handleAihubEventsHttp
  adapterArgs:
    - param: request
      source: { kind: http.request }
chatEventsPost:
  method: POST
  path: /chat/events
  kind: rawHttp
  handler: internal.aihub_service.handleAihubEventsHttp
  adapterArgs:
    - param: request
      source: { kind: http.request }
```

`http.yml` 只定义真实 ingress 的 adapter；它不会把 package/test-service 对 handler
或 helper 的直接调用自动提升为 raw HTTP ingress 调用。

## 精确 callable 链

生产 handler 路径（只由序号 44 直接调用 handler，但未经过 ingress）：

```text
test collectHttpEventStream
-> internal.aihub_service.handleAihubEventsHttp                 (2454-2474)
-> streamChatEventResponse                                     (2404-2419)
-> emitChatEventItems                                          (2386-2402)
-> emitChatEventItemsProtocolHandled                           (2368-2384)
-> emitChatEventItemsJsonDecodeHandled                         (2350-2366)
-> emitChatEventItemsLlmDecodeHandled                          (2332-2348)
-> emitChatEventItemsUnsafe                                    (2307-2330)
-> emitHttpEventChunk                                          (2194-2199)
-> std.http.emitResponseStream
```

测试 helper 路径（序号 45、46、48）：

```text
test / postStartHttpEventError
-> internal.aihub_service.streamChatEventResponseForTest       (2421-2452)
-> streamChatEventResponse
-> emitChatEventItems
-> emitChatEventItemsProtocolHandled
-> emitChatEventItemsJsonDecodeHandled
-> emitChatEventItemsLlmDecodeHandled
-> emitChatEventItemsUnsafe
-> emitHttpEventChunk
-> std.http.emitResponseStream
```

序号 46 在第一个 fixture item 上就进入上述 native 调用，因此还没执行到 fixture
预设的 typed decode/protocol/unavailable error。序号 48 同样在 slow fixture 的
`start` item 上失败，不是 60 秒 sleep 或取消链失败。

## 完整可见错误链

`std.http.emitResponseStream` 的 binding key 是
`std.http.stream.emitResponse`。运行时路径为：

```text
runtime/native/src/dispatch/http.rs:181-194
-> response_stream_context.response_item_type(...)
-> runtime/host/src/capability_context/stream.rs:51-52
-> response_stream_sink(...)
-> runtime/host/src/capability_context/stream.rs:70-78
-> response_stream_sink == None
-> RuntimeError::Decode(
     "std.http.emitResponseStream used outside a raw HTTP streaming response context"
   )
```

对应 contract 实现还可见于
`runtime/capability-context/src/stream.rs:883-884,932-940`。

这不是语言 `throw` 出来的 typed exception，而是平台
`RuntimeError::Decode`。因此 HTTP 结果和
`runtime.assembly_request_error` 只输出错误字符串，不携带语言 exception stack；
本轮没有可再展开的隐藏语言堆栈。四条运行时错误按执行顺序为：

```text
request_id=aedd49a8-e486-46a1-add1-42017bc31e9f
request_id=d7b42705-512a-4fa8-9873-e67f71ad4c00
request_id=021bedc8-cea1-40b4-97e8-a4cb851777d3
request_id=1c902d28-3841-4611-a641-349a9841f599
error=std.http.emitResponseStream used outside a raw HTTP streaming response context
```

去敏原序 stream trace 共 822 行；四条失败的完整 trace 范围为：

- 序号 44 / runtime 45：`632-688`；
- 序号 45 / runtime 46：`689-715`；
- 序号 46 / runtime 47：`716-742`；
- 序号 48 / runtime 49：`791-816`。

序号 45、46、48 都在关闭 owner 前回到 `active=0`。序号 44 在 request error
关闭 owner 时仍有一个异步 stream，随后 finish 到 `active=0`；这属于失败请求的清理
证据，不改变“缺少 raw HTTP response sink”这一首个错误。

## 诊断边界与后续

- 本账本只记录证据，不修改 AIHub、test runner 或 runtime 行为。
- G2 已关闭原 34 条 JSON encode 失败。
- case 28/29 不需要新的 S3 production fix。
- 剩余四条应由后续任务决定测试应通过真实 raw HTTP ingress 调用，还是为测试运行显式建立
  raw HTTP response context；本账本不替该任务做设计决定。
- 隔离 Mongo、Router、Runtime、动态端口和 `skiff-test-runtime-*` 已清理。
  只保留无 secret 的原序 trace 和可复用 Cargo cache，供下一轮诊断/复验。
