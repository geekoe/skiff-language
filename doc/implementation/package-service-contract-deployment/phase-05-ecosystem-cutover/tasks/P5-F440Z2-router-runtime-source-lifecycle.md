# P5-F440Z2 Router runtime-source / socket writer lifecycle

状态：Ready。R0b2b；只建立Endpoint source disconnect/isolation与observed peer writer。

## 直接父节点

- `P5-F440X-router-websocket-rpc-hookup-preflight-result.md`
- `P5-F440P-websocket-rpc-transport-checkpoint-result.md`
- `P5-F440R-router-websocket-rpc-profile-broker-core-result.md`
- `P5-F440Y-router-runtime-dispatcher-websocket-rpc-result.md`

F440P已冻结captured runtime source/response API；F440X证明最终broker还需要exact source disconnect、
Endpoint-owned isolation和可观察`ws.send` callback failure。本leaf只建立这些生命周期接口，不接broker。

实现基线为`85ff1513`对应的current integration tree。

## 目标

`RuntimeEndpoint`增加：

```ts
onConnectionRequestSourceDisconnect(
  handler: (source: RuntimeConnectionRequestSource) => void
): () => void

isolateConnectionRequestSource(
  source: RuntimeConnectionRequestSource,
  reason: string
): void
```

WebSocket connection lifecycle增加broker可消费的observed writer，使`ws.send` callback error、closed/
closing socket与slow-client budget均成为明确terminal，而不是fire-and-forget。

本leaf不注册broker callback、不解析JSON-RPC、不接Gateway request dispatch。

## 唯一写集

生产：

- `router/src/router/runtimeEndpoint.ts`
- `router/src/gateway/webSocketConnectionLifecycle.ts`
- 上述模块需要的private type/helper

测试：

- `router/tests/runtime-endpoint-connection-send-trust.test.ts`
- `router/tests/websocket-connection-lifecycle.test.ts`
- 可新增`router/tests/runtime-endpoint-source-lifecycle.test.ts`
- 本leaf result

禁止修改Gateway主模块、broker、RuntimeDispatcher、server/snapshot/protocol wire、Rust、其它task/result。
不得派子Agent，不得启动server/network/live。

## Runtime source lifecycle

- source identity继续是F440P captured `{sender, sessionToken,...}`；
- WebSocket close/error/shutdown必须在删除session token与释放sender前同步通知exact source disconnect；
- 每个source生命周期最多一次disconnect callback；
- reconnect的同名runtime是新source，不能收到旧disconnect或完成旧pending；
- callback可以安全re-enter Endpoint只读状态，不看到已换成新session的source；
- handler unsubscribe幂等，不接收后续事件；
- handler异常不能阻止其它handler、session cleanup或socket close，但必须按current diagnostics记录。

`isolateConnectionRequestSource`：

- 只接受当前registered exact source；
- foreign/stale source无权关闭其它runtime；
-关闭offending runtime session并走同一disconnect cleanup；
- reason有界、只供platform diagnostics，不进入peer/business payload；
- Gateway后继只能请求Endpoint isolate，不能直接持有/关闭runtime sender。

## Observed peer writer

提供broker可注入writer：

- text write经现有WebSocket lifecycle/slow-client accounting；
- callback success才算send成功；
- callback error、同步throw、socket非OPEN、close竞态返回明确failure；
- send completion后lease/accounting精确释放；
- disconnect/finish使outstanding observed writes终止且不double-callback；
- binary/close behavior保持current；本leaf不放宽binary RPC；
-不得让后继bridge绕过lifecycle直接调用raw `ws.send`。

API可用Promise或单次callback，但必须保证最多一次terminal并可由broker测试注入。

## Test-first与验证

先增加source close或send callback failure RED。至少覆盖：

- exact source disconnect once、before token deletion；
- reconnect/session fencing；
- multiple handlers/unsubscribe/throw isolation；
- isolate current source成功，stale/foreign no-op/fail closed；
- send callback success/error、sync throw、non-open socket；
- close-vs-send race、outstanding accounting归零；
- existing text/binary/close/slow-client tests不回归。

必跑：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/websocket-connection-lifecycle.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/websocket-connection-lifecycle.test.ts
pnpm --dir router type-check
git diff --check
```

未新增可选文件时删除不存在路径；必须记录非零listing/count。

## 停止与交付

若source disconnect必须修改server/gateway composition，先提供Endpoint-local API/tests并返回
`TASK_SCOPE_EXPANDED`；不得接broker。若observed writer需要改变slow-client public policy，停止而非绕过。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440z2-router-source-lifecycle`
- branch：`codex/p5-f440z2-router-source-lifecycle`
- result：`P5-F440Z2-router-runtime-source-lifecycle-result.md`

Implementation与result分开提交；不merge/rebase/push。
