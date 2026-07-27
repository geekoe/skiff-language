# P5-F440Z3B Router WebSocket RPC Gateway integration

状态：Ready。R0b2d；把已完成的纯 bridge 接到真实 Gateway、socket lifecycle 与 server
composition，完成 Router WebSocket RPC production path。

## 直接父节点

- `P5-F440Z3A-router-websocket-rpc-bridge-core-result.md`
- `P5-F440Z1-router-websocket-rpc-snapshot-result.md`
- `P5-F440Z2-router-runtime-source-lifecycle-result.md`
- `P5-F440Y-router-runtime-dispatcher-websocket-rpc-result.md`
- `P5-F440X-router-websocket-rpc-hookup-preflight-result.md`

父节点已经冻结：

- current physical WebSocket binding、closed `jsonrpc-2.0-text` profile 与 immutable method table；
- exact RuntimeEndpoint request source lifecycle、isolation 和 observed peer writer；
- receipt-pinned connect / JSON-RPC dispatcher；
- profile-neutral broker 与纯 `WebSocketRpcBridge`；
- handlerless method-bearing connection 也必须 eager pin old generation。

本 leaf 只负责真实 upgrade/attach/lifecycle/server 接线，不再改变 snapshot、dispatcher、broker、
Endpoint 或 wire 语义。实现基线为 `78c10985` 对应的 current integration tree。

## DAG 位置与完成检查点

```text
Z3A pure bridge
  -> Z3B Gateway/server production hookup
  -> F0 cross-system fixtures/tooling
  -> combined integration gate
```

当前是实现检查点，不是稳定候选。完成后关闭 Router R0b production owner，并解除 F0。

## 目标

真实外部 WebSocket 连接必须同时保留既有 connect/downlink 行为，并接入 Z3A bridge：

1. upgrade 时从同一个 selected snapshot/binding 捕获 profile、method table、deployment、
   assembly generation、host/path 与 timeout；
2. 有 connect handler **或** method table 非空时，都先 dispatch connect 并取得 exact runtime
   receipt/replica pin；无 handler但有method时使用 Host 已实现的 synthetic accept；
3. physical binding 无 handler且无method时不 acquire runtime，但仍可作为同 old
   service/generation 的 path-only JSON-RPC transport；
4. socket attach 后捕获 Z2 observed writer，建立 bridge generation，并把 peer text/binary/close
   交给该 handle；
5. inbound peer request 只使用 attach-time method table与receipt；current snapshot replacement
   不影响旧连接；
6. outbound runtime request/cancel 由同一个 production bridge 处理，response 回 exact
   RuntimeEndpoint source；
7. peer disconnect、runtime disconnect、writer failure与Gateway shutdown均先完成broker teardown，
   再最多一次release generation；
8. server只创建一个 bridge实例，关闭顺序为Gateway连接清理、bridge cleanup、RuntimeEndpoint关闭。

不新增public配置、manifest字段、wire spelling、错误码或语言语义。

## 写入范围

生产 owner：

- `router/src/gateway/webSocketGateway.ts`
- `router/src/gateway/webSocketConnectionLifecycle.ts`，仅在真实关闭回调无法承载异步
  bridge finalize时做最小生命周期扩展
- `router/src/gateway/webSocketRpcBridge.ts`，仅允许production attach所需的机械 adapter/API
  收口，不改变Z3A broker/dispatch语义
- 可新增一个 `router/src/gateway/` 下的 WebSocket RPC attach sibling，避免继续扩大已超过千行的
  Gateway主文件
- `router/src/router/server.ts`
- `router/src/index.ts`，仅必要export

测试 owner：

- 新建 `router/tests/websocket-jsonrpc-gateway.test.ts`
- `router/tests/websocket-gateway.test.ts`
- `router/tests/websocket-connection-lifecycle.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`
- `router/tests/runtime-assembly-request-wire.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`
- `router/tests/websocket-rpc-bridge.test.ts`
- `router/tests/websocket-generation-lifecycle-router.test.ts`
- 本 leaf result

若现有Gateway constructor fixture需要跟随新的必填bridge dependency，可在上述测试内机械更新。
不得把production bridge做成server可遗漏的optional路径。

禁止修改：

- current snapshot/reader、artifact producer/schema、deployment identity；
- `RuntimeDispatcher`、`RuntimeEndpoint`、broker/profile state machine与generation router语义；
- Host/runtime/Rust；
- cross-system fixtures、README、checker与其它task/result。

若真实接线证明必须改变任一禁止 owner 或公共契约，停止并返回 `TASK_SCOPE_EXPANDED`，不要吞并。

## Captured connection组装

Gateway必须从同一次prepare/attach路径形成完整且不可变的Z3A context：

- 独立socket generation token与external connection id；
- selected physical binding的service/deployment、assembly identity/generation、entry id、host/path；
- physical closed profile及对应profile adapter；
- `binding.websocketMethods.capture()` 的独立copy；
- connect返回的captured business identity；
- lifecycle `capturePeerWriter(connectionId)`；
- Router request timeout与captured deployment/binding timeout；
- exact connect dispatcher receipt与replica id（若发生pin）；
- runtime owner resolver必须由Endpoint source sender经Router registry验证为同service、old
  assembly/generation/replica；
- release callback只对已pin generation调用
  `generationLifecycle.releaseConnection(connectionId)`，并保持幂等。

不能在message到达时重新读取current snapshot、current method table或重新选择runtime。
peer params、request id和method均不能覆盖captured business/routing identity。

## Upgrade与pin规则

把当前 `binding.handler !== undefined` 的分支改成显式：

```text
requiresRuntimePin =
  connect handler exists
  OR captured method table is non-empty
```

- `requiresRuntimePin=true` 时统一执行现有 connect dispatch、expect/acquire/receipt验证；
- handlerless method-bearing connection必须由真实测试证明发生一次 acquire；
- pure path-only connection保持 zero acquire；
- upgrade/admission/bridge attach中任一步失败，都必须释放reservation和已取得的generation，不泄漏
  lifecycle connection、broker generation或runtime pin；
- connect拒绝与既有connectionPolicy行为保持不变。

## Peer与runtime事件

- text frame转换为profile text后交 `handlePeerText`；
- binary frame交 `handlePeerBinary`，由profile固定terminal，不再用Gateway自己的“所有data都不支持”
  分支；
- socket close/error、policy close、generation lost与shutdown最终都调用同一幂等finalize；
-所有peer write只经captured observed writer；不得旁路回`socket.send`；
-既有 `connection.send` direct/business downlink保持原行为，不进入RPC request broker；
- runtime origin source disconnect只清该source的outbound pending，不误关其它source/connection；
- pinned runtime generation丢失仍关闭peer `1011`，随后按统一顺序teardown/release；
- notification不dispatch runtime、不产生response；peer request/cancel与runtime request/cancel均保持
  Z3A最多一次terminal。

若Gateway需要保存handle，只保存opaque handle，不暴露或复制broker maps。

## Server composition与关闭

`server.ts`在RuntimeEndpoint和Dispatcher可用后创建唯一 `WebSocketRpcBridge`，把它作为Gateway
必填dependency。shutdown即使前一步失败也继续尝试：

```text
AssemblyWebSocketGateway.close()
WebSocketRpcBridge.cleanup()
HTTP Gateway / RuntimeEndpoint / activation close
```

Gateway close必须先停止upgrade，再shutdown连接并等待所有bridge finalize/release，之后
generation lifecycle flush。不得关闭Endpoint后再清bridge。

## 结构要求

`webSocketGateway.ts`已经超过1000行。本任务不得把bridge协议处理、context validation或状态机复制
进该文件。Gateway只保留selection、upgrade、connect/admission与事件编排；超过简单组装的
RPC attach/data转换应放进新的gateway sibling。不得复制broker limits、timeout常量或JSON-RPC parser。

## Test-first与完成标准

先新增真实RED，至少证明当前production路径存在以下一个缺口，而不是synthetic failure：

- handlerless method-bearing upgrade没有eager pin；或
- peer text仍被Gateway直接关闭且没有进入bridge。

新增真实Gateway级测试至少覆盖：

- handlerless method-bearing连接：一次connect/acquire，peer request dispatch到captured old handler；
- snapshot替换后旧连接仍使用old method table、old receipt/generation；
- pure path-only连接zero acquire，合法old service/generation outbound request可往返；
- outbound success/remote error/cancel、inbound success/cancel、notification零dispatch；
- binary/protocol错误、observed writer rejection与peer disconnect最多一次terminal；
- runtime source/replica/service/entry不匹配fail closed；
- origin runtime disconnect只清其pending；
- pinned runtime disconnect关闭peer `1011`；
- attach失败、upgrade失败与shutdown后connection/pin/bridge accounting归零；
-既有 connect policy、direct/business `connection.send` 与HTTP gateway回归不变。

必跑non-live验证：

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

必须先用 `vitest list`记录non-zero count。可使用测试自己创建并回收的loopback/ephemeral server；
不得启动stable instance、watch、长期server、外部network或live测试。

## 提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f440z3b-router-rpc-gateway`
- branch：`codex/p5-f440z3b-router-rpc-gateway`
- result：
  `P5-F440Z3B-router-websocket-rpc-gateway-integration-result.md`

Implementation与result分开提交。5分钟内必须开始test-first实际修改；否则返回
`TASK_NOT_EXECUTABLE`与精确缺口。不得派子Agent，不得merge/rebase/push。
