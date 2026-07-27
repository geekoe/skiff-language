# P5-F440Z3A Router WebSocket RPC bridge core

状态：Ready。R0b2c；用纯connection接口连接Endpoint、broker与dispatcher，不改Gateway/server。

## 直接父节点

- `P5-F440Z1-router-websocket-rpc-snapshot-result.md`
- `P5-F440Z2-router-runtime-source-lifecycle-result.md`
- `P5-F440Y-router-runtime-dispatcher-websocket-rpc-result.md`
- `P5-F440R-router-websocket-rpc-profile-broker-core-result.md`
- `P5-F440X-router-websocket-rpc-hookup-preflight-result.md`

父节点已冻结immutable method table、observed writer/Endpoint source lifecycle、receipt-pinned dispatcher和
broker/profile core。本leaf只实现它们之间的profile-neutral bridge；真实upgrade/attach留Z3B。

实现基线为`e67e1b9d`对应的current integration tree。

## 目标

新建：

```text
router/src/gateway/webSocketRpcBridge.ts
```

bridge通过显式captured connection接口完成：

- 注册/注销Endpoint `connection.request`、`connection.request.cancel`与source disconnect；
- outbound runtime request exact connection/source join后交broker，response回captured Endpoint source；
- peer text/binary/disconnect交broker；
- inbound method request只从captured immutable method table构造F440Y dispatcher request；
- peer cancel/deadline/disconnect通过AbortSignal/request.cancel关闭runtime leg；
- protocol violation请求Endpoint隔离offending runtime source；
-所有race最多一次runtime/peer terminal。

本leaf不用raw WebSocket/Gateway/server/current snapshot；direct tests用fake captured connection。

## 唯一写集

生产：

- 新建`router/src/gateway/webSocketRpcBridge.ts`
- `router/src/router/webSocketRequestBroker.ts`
- `router/src/router/webSocketRequestBrokerTypes.ts`
- `router/src/router/runtimeEndpoint.ts`只允许消费Z2已存在API所需的type/export机械修改
- `router/src/protocol/runtimeAssemblyRequest.ts`
- `router/src/protocol/runtimeAssemblyRequestMetadata.ts`
- `router/src/protocol/runtimeProtocol.ts`
- `router/src/index.ts`仅public bridge/type export

测试：

- 新建`router/tests/websocket-rpc-bridge.test.ts`
- `router/tests/websocket-request-broker.test.ts`
- `router/tests/runtime-endpoint-connection-send-trust.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`只做bridge/dispatcher contract回归
- 本leaf result

禁止修改`webSocketGateway.ts`、`webSocketConnectionLifecycle.ts`、server、snapshot/reader、
RuntimeDispatcher behavior、Rust、其它task/result。不得派子Agent，不得启动server/network/live。

## Captured connection contract

bridge接受一个attach-time immutable context，至少包括：

- bridge-owned socket generation token；
- external connection id；
- service/deployment owner；
- exact old assembly identity/generation；
- physical `WebSocketEntryId`、host/path；
- supported profile与profile adapter；
- readonly captured method table；
- captured business identity；
- Z2 observed peer writer与close/isolate callback；
- generation release callback。

method table entry包含exact external method、method gateway identity、profile及F440Y request header所需facts。
peer id不进入该context的runtime/business字段。

bridge暴露Z3B可消费的最小handle：

- attach；
- peer text；
- peer binary；
- peer disconnect/finalize；
- debug/cleanup仅供direct test。

不得暴露broker mutable maps或要求Gateway拼request.start wire。

## Outbound runtime leg

Endpoint request到达时：

-验证exact source session、connection id、service、physical entry、profile、assembly generation；
- pure path-only replica只在同old generation/owner规则下允许；
- unknown/foreign/stale connection返回F440P exact
  `connectionUnavailable | transportUnavailable | protocolError`，不得泄漏其它service连接；
-交broker `handleRuntimeRequest`，peer write只经captured observed writer；
- broker terminal经Endpoint `sendConnectionResponse`回original source；
- runtime cancel交global broker cancel；detach/tombstone后best-effort peer cancel，零普通response；
- source disconnect先`handleRuntimeDisconnect`，只清该source；
- protocol violation由bridge调用Endpoint `isolateConnectionRequestSource`，不直接关runtime socket。

## Inbound peer leg

- text/binary分别交profile/broker；binary按current profile拒绝；
- request只从captured method table exact lookup；
-构造F440Y request：old assembly identity/generation、physical/method identity、host/path/method/profile、
  connection id、captured business identity、opaque params；
- dispatcher receipt来自captured generation pin，不能current select；
- success payload经profile adapter `fromRuntimePayload(..., inboundResult)`；
- invalidParams/internalError/deadlineExceeded映射固定profile error；
- unavailable/protocol reject映射`runtimeUnavailable`；
- ordinary notification不进入dispatcher、不建立response terminal；
- peer cancel先由broker detach/tombstone，再abort dispatcher并发送唯一cancel response；
- signal reason使用current canonical `RequestCancelReason`；
- late Promise completion在broker execution token失效后无write。

## Limits/timeout与terminal

- broker limits使用R0a唯一default object，不在bridge复制magic constants；
- inbound timeout为attach context提供的
  `min(router requestTimeoutMs, captured deployment timeoutMs)`；bridge验证positive canonical值；
- peer/runtime disconnect、deadline、cancel、send callback error各自按F440X owner矩阵最多一次terminal；
- finalize先broker disconnect，再调用generation release callback；
- cleanup后active/timer/tombstone/dispatcher abort listener达到规定状态。

## Test-first与验证

先用fake connection/Endpoint/dispatcher新增RED，证明bridge不存在。至少覆盖：

- outbound success/remote/cancel/deadline/source disconnect/protocol isolation；
- exact service/entry/generation/source negatives；
- inbound request success、invalid/internal/deadline/unavailable；
- notification零dispatcher；
- peer cancel/disconnect、late completion；
- same-value inbound/outbound ids隔离；
- old captured method table在current replacement后仍构造old request；
- business identity来自capture，params同名字段不能覆盖；
- observed writer callback failure；
- finalize顺序broker disconnect -> generation release；
- active/pending/timer最终归零、最多一次write。

必跑：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
pnpm --dir router type-check
git diff --check
```

必须记录non-zero listing/count。

## 停止与交付

若bridge需要修改Gateway/server/snapshot或Dispatcher behavior，提交pure bridge有效部分并返回
`TASK_SCOPE_EXPANDED`；不得吞入Z3B。若captured context缺少不可由父checkpoint唯一提供的字段，停止并列
精确owner。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440z3a-router-rpc-bridge`
- branch：`codex/p5-f440z3a-router-rpc-bridge`
- result：`P5-F440Z3A-router-websocket-rpc-bridge-core-result.md`

Implementation与result分开提交；不merge/rebase/push。
