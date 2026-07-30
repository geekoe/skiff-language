# P5-F440Z3E Router WebSocket RPC Gateway integration resume

状态：Ready。Z3B恢复节点；wire v2 blocker已由Z3D解除，现在完成真实Gateway/server接线。

## 直接父节点

- `P5-F440Z3B-router-websocket-rpc-gateway-integration-result.md`
- `P5-F440Z3D-current-gateway-entry-wire-v2-hard-cut-result.md`
- `P5-F440Z3A-router-websocket-rpc-bridge-core-result.md`

Z3B已经记录真实 eager-pin RED，但因connect wire v1/v2断层按范围停止并清理全部WIP。Z3D已一次
hard cut HTTP/test/connect/JSON-RPC request到current GatewayEntry v2；Router 301 tests与相关Rust
transport均证明该blocker已解除。

本节点使用全新开发会话，从 `3b888648` 对应的current integration tree重新实现，不依赖Z3B会话
或已清理WIP。

## DAG与目标

```text
Z3D current wire checkpoint
  -> Z3E real Gateway/server hookup
  -> F0 remaining fixtures/tooling
  -> combined integration gate
```

真实外部WebSocket upgrade必须接到Z3A bridge，同时保持已有connect policy与
`connection.send` downlink行为：

1. 同一次selected snapshot/binding捕获profile、immutable method table、deployment、old
   assembly generation、host/path与timeout；
2. connect handler存在 **或** method table非空时，统一dispatch connect并取得exact
   receipt/replica pin；handlerless method-bearing使用Host synthetic accept；
3. 无handler且无method的path-only连接zero acquire，但仍可承载同old service/generation的
   outbound JSON-RPC request；
4. socket attach后捕获observed writer并建立bridge generation；peer text/binary/close进入opaque
   bridge handle；
5. snapshot替换不影响旧连接method table、receipt、generation与runtime owner；
6. disconnect、writer failure、generation lost与shutdown先broker teardown，再最多一次release；
7. server创建唯一bridge，Gateway close后cleanup bridge，再关闭RuntimeEndpoint。

不新增public配置、manifest字段、wire spelling或语言语义。

## Production写集

- `router/src/gateway/webSocketGateway.ts`
- 可新增 `router/src/gateway/webSocketRpcConnectionAttachment.ts` 或同职责单一的sibling，承载RPC
  attach/data转换，避免扩大已超过1000行的Gateway
- `router/src/gateway/webSocketConnectionLifecycle.ts`，仅真实关闭回调无法等待bridge finalize时做
  最小机械扩展
- `router/src/gateway/webSocketRpcBridge.ts`，仅production attach所需的机械profile adapter/API
  收口；不得改变Z3A broker/dispatch语义
- `router/src/router/server.ts`
- `router/src/index.ts`，仅必要export

禁止修改snapshot/reader、artifact producer/schema、RuntimeDispatcher、RuntimeEndpoint、
broker/profile state machine、generation router语义、Host/runtime/Rust、cross-system fixture、
README/checker或其它task/result。

若真实接线仍要求禁止owner或公共契约变化，停止并返回 `TASK_SCOPE_EXPANDED`。

## Gateway组装与pin

显式计算：

```text
requiresRuntimePin =
  connect handler exists
  OR binding.websocketMethods.size > 0
```

- true时走既有connect dispatch、expect/acquire/receipt验证；
- false时不选runtime、不acquire；
- upgrade、admission或attach任一步失败均清reservation、bridge generation和已取得的pin；
- connect拒绝、business identity与connection policy语义不变。

Bridge context必须来自attach-time capture：

- 独立socket generation token与connection id；
- selected binding的service/deployment、assembly identity/generation、entry id、host/path；
- closed singleton profile及对应adapter；
- `binding.websocketMethods.capture()` 独立copy；
- captured business identity；
- lifecycle `capturePeerWriter(connectionId)`；
- Router timeout与binding/deployment timeout；
- exact receipt与replica id（若pin）；
- runtime owner resolver由Endpoint source sender经registry验证同service、old
  assembly/generation/replica；
- 幂等release callback，仅已pin时调用generation lifecycle release。

message到达时不得重新读current snapshot、重新select runtime或接受peer字段覆盖captured
routing/business identity。

## Event与关闭规则

- peer text交 `handlePeerText`，binary交 `handlePeerBinary`；
- socket close/error、policy close、generation lost、attach error、Gateway shutdown进入同一幂等
  finalize；
-所有RPC peer write只经observed writer，不直接`socket.send`；
-既有 `connection.send` direct/business downlink保持原路径，不进入request broker；
- origin runtime source disconnect只清该source的outbound pending；
- pinned generation lost关闭peer `1011 / websocket runtime disconnected`；
- notification不dispatch runtime、不response；
- peer/runtime request与cancel、late completion保持Z3A最多一次terminal。

Gateway close顺序：

```text
stop accepting upgrades
-> lifecycle shutdown / await all bridge finalize
-> generation lifecycle flush
```

Server shutdown即使前一步失败也继续：

```text
AssemblyWebSocketGateway.close()
-> WebSocketRpcBridge.cleanup()
-> HTTP Gateway / RuntimeEndpoint / activation close
```

不得先关闭Endpoint。

## Test写集与test-first

- 新建 `router/tests/websocket-jsonrpc-gateway.test.ts`
- `router/tests/websocket-gateway.test.ts`
- `router/tests/websocket-connection-lifecycle.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`
- `router/tests/runtime-assembly-request-wire.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`
- `router/tests/websocket-rpc-bridge.test.ts`
- `router/tests/websocket-generation-lifecycle-router.test.ts`
- 本节点result

先重建并执行真实RED：

- current v2 handlerless method-bearing loopback upgrade要求一次connect/acquire；production当前得到0；
  或
- peer text当前仍被Gateway直接关闭、没有进入bridge。

至少覆盖：

- handlerless method-bearing eager pin与peer request；
- snapshot replacement后old method/receipt/generation；
- path-only zero acquire与合法old owner outbound往返；
- inbound/outbound success、remote error、cancel，notification零dispatch；
- binary/protocol error、writer rejection、peer disconnect最多一次terminal；
- source/replica/service/entry mismatch fail closed；
- origin source disconnect隔离；
- pinned runtime disconnect 1011；
- upgrade/attach failure与shutdown后connection/pin/bridge accounting归零；
- connect policy、direct/business `connection.send` 与HTTP gateway回归。

loopback/ephemeral server允许，但必须由测试回收；不得启动stable、watch、长期server、外部network或
live。

## 必跑non-live验证

```bash
router/node_modules/.bin/vitest list --root router \
  tests/websocket-jsonrpc-gateway.test.ts \
  tests/websocket-gateway.test.ts \
  tests/websocket-connection-lifecycle.test.ts \
  tests/router-websocket-trust-dispatch.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-generation-lifecycle-router.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/websocket-jsonrpc-gateway.test.ts \
  tests/websocket-gateway.test.ts \
  tests/websocket-connection-lifecycle.test.ts \
  tests/router-websocket-trust-dispatch.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-generation-lifecycle-router.test.ts
pnpm --dir router type-check
git diff --check
```

必须先记录non-zero listing/count。

## 提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f440z3e-router-rpc-gateway`
- branch：`codex/p5-f440z3e-router-rpc-gateway`
- result：`P5-F440Z3E-router-websocket-rpc-gateway-integration-resume-result.md`

Implementation与result分开提交。5分钟内开始真实test-first修改；不得派子Agent，不得
merge/rebase/push。
