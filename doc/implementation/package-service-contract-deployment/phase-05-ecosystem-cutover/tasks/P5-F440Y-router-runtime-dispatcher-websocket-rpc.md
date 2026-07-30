# P5-F440Y Router RuntimeDispatcher WebSocket RPC sibling

状态：Ready。R0b1；只建立receipt-pinned RuntimeDispatcher runtime leg。

## 直接父节点

- `P5-F440X-router-websocket-rpc-hookup-preflight-result.md`
- `P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`
- `P5-F440W1-websocket-rpc-host-dispatch-result.md`
- `P5-F440R-router-websocket-rpc-profile-broker-core-result.md`

F440T/W1冻结request/response shape与Host behavior；F440X证明dispatcher sibling可独立完成并必须先于
gateway hookup。

实现基线为`484d55bea6b73c5dd776edb16a0b552a0f001448`。

## 目标与public sibling

增加current命名等价API：

```ts
interface RuntimeAssemblyWebSocketJsonRpcDispatchRequest {
  header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;
  payloadBytes: Uint8Array;
}

interface RuntimeAssemblyWebSocketJsonRpcDispatchResponse {
  header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader;
  payloadBytes: Uint8Array;
}

dispatchAssemblyWebSocketJsonRpc(
  request,
  timeoutMs,
  exact RuntimeDispatchConnectionReceipt,
  { signal: AbortSignal }
): Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>;
```

只完成Router→captured runtime socket→typed response的runtime leg。gateway/broker/peer socket仍不接。

## 唯一写集

生产：

- `router/src/router/runtimeDispatcher.ts`
- `router/src/router/runtimeEndpoint.ts`只允许widen outbound frame sender/executable type
- `router/src/protocol/runtimeAssemblyRequest.ts`
- `router/src/protocol/runtimeAssemblyRequestFrame.ts`
- `router/src/protocol/runtimeAssemblyRequestResponseFrame.ts`
- 上述类型新增variant所需同module机械type/export修改

测试：

- 新建`router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`
- `router/tests/runtime-endpoint-connection-send-trust.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`只做connect sibling回归
- 本leaf result

禁止修改Gateway、broker、server/snapshot、RuntimeEndpoint request callback/disconnect/isolation behavior、
Host/Rust、wire shape、其它task/result。不得派子Agent，不得启动server/network/live。

## Receipt、request与response

- receipt必须由`RuntimeDispatcher`自己的WeakMap恢复exact captured runtime connection；
-不得按service/current runtime registry重新选择sender；
- foreign/expired/closed receipt在发送前拒绝；
- method-bearing JSON-RPC request不得被connect-only type guard捕获；
- connect acquire predicate仍只接受`protocol=WebSocket && method=null`；
- JSON-RPC request必须method non-null、unary、payload present，并通过F440T strict gate；
- `RuntimeFrameSender`/Endpoint sender只widen到current executable union，不允许任意transport-only frame。

response只接受同captured socket、same request id的F440T JSON-RPC end：

- success必须payload present（`null`合法）；
- invalidParams/internalError/deadlineExceeded必须payload absent；
- `response.error`、connect/HTTP branch、wrong id/socket、malformed payload均不能完成pending；
- first terminal先detach/clear timer/abort listener，再resolve/reject；
- late/duplicate response不影响新request。

## Timeout/cancel

- timeout与AbortSignal都先detach pending，再best-effort发送现有`request.cancel`；
- signal.reason若是current canonical `RequestCancelReason`则原样发送，否则`caller_cancel`；
- 不新增cancel spelling或JSON-RPC response；
- abort已settled request为no-op；
- runtime response先赢时清除abort/timer，late abort不发cancel；
- pending/timer最终归零。

Dispatcher只拥有runtime leg；peer cancel/deadline的唯一peer terminal仍归后继broker。

## Test-first与验证

先增加sibling test，使旧dispatcher因API不存在或把method request误判connect而RED。至少覆盖：

- exact receipt happy path success/null；
-三种无payload outcome；
- foreign/closed receipt、wrong socket/id/branch/payload presence拒绝；
- concurrent/乱序request；
- timeout/abort detach-before-cancel；
- canonical/unknown abort reason；
- response-vs-abort race最多一个terminal；
- late/duplicate response不完成新pending；
- connect acquire/type guard行为不回归；
- Endpoint sender仍拒绝非executable frame。

必跑：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/router-websocket-trust-dispatch.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/router-websocket-trust-dispatch.test.ts
pnpm --dir router type-check
git diff --check
```

必须先列出非零tests并记录count，不使用pnpm wrapper的零输出冒充listing。

## 停止与交付

若dispatcher需要Gateway/broker/source-disconnect/isolation API，提交仍有效的pure sibling checkpoint并返回
`TASK_SCOPE_EXPANDED`；不得吞入F440Z。若request/response shape要求修改F440T wire，停止并回报冲突。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440y-router-rpc-dispatcher`
- branch：`codex/p5-f440y-router-rpc-dispatcher`
- result：`P5-F440Y-router-runtime-dispatcher-websocket-rpc-result.md`

Implementation与result分开提交；不merge/rebase/push。
