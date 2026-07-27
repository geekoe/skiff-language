# P5-F440Z3E Router WebSocket RPC Gateway integration resume result

状态：`PASS / REAL_GATEWAY_BRIDGE_HOOKUP_COMPLETE`。

本 leaf 已把真实外部 WebSocket upgrade 接到 Z3A `WebSocketRpcBridge`，并在 current
GatewayEntry v2 wire 上完成 handlerless method-bearing eager pin、path-only zero pin、
attach-time immutable capture、observed writer、幂等 bridge teardown/release 与 Router server
shutdown 接线。没有新增 public 配置、manifest 字段、wire spelling、兼容分支或语言语义。

## 1. 基线、分支与提交

| 项目 | 值 |
| --- | --- |
| worktree | `/Users/geek/workspace/skiff-p5-f440z3e-router-rpc-gateway` |
| branch | `codex/p5-f440z3e-router-rpc-gateway` |
| current integration checkpoint | `3b888648` |
| task start HEAD | `14e892646b3096f48b2a908e482d481ae70fc34f` |
| implementation commit | `ae03b1e8ea8b510b6fc604b3cd4d6c010a906d0b` |
| result commit | 本文件独立提交；最终 commit 见交付消息 |

Implementation commit 只包含任务允许的 5 个 production 文件与 3 个测试文件；result 未混入
implementation commit。

## 2. 真实 test-first RED

先在真实 loopback `AssemblyWebSocketGateway` fixture 中加入 current v2、handlerless、带
`chat.send` method table 的 physical binding，并要求同一次 upgrade 执行一次 connect/acquire。

依赖就绪后的精确 listing 为非零 `1`：

```text
tests/websocket-gateway.test.ts >
current RuntimeAssembly WebSocket gateway >
eagerly pins a handlerless method-bearing WebSocket connection
```

production 修改前执行：

```bash
router/node_modules/.bin/vitest run --root router \
  tests/websocket-gateway.test.ts \
  -t 'eagerly pins a handlerless method-bearing WebSocket connection'
```

结果为 exit `1`，`1 failed / 8 skipped`：

```text
AssertionError: expected [] to have a length of 1 but got 0
tests/websocket-gateway.test.ts:64
```

失败来自 production 的 `binding.handler === undefined` 分支跳过 connect dispatch；不是零 selector、
synthetic throw 或 wire v1/v2 blocker。实现 `handler exists OR method table nonempty` 后，同一
targeted probe `1/1 PASS`。

本 worktree 初始没有依赖，第一次调用只得到 Vitest 路径不存在，未把它计作 RED。随后临时链接
主仓库已有 `router/node_modules` 与 root `node_modules`，完成上述真实 RED 与全部验证；两个链接
均已删除，没有安装依赖或修改 lockfile。

## 3. Implementation

### 3.1 Selected capture 与 eager pin

`webSocketGateway.ts` 显式按下式决定 pin：

```text
requiresRuntimePin =
  binding.handler !== undefined
  OR capturedMethodTable.size > 0
```

- true 时继续走既有 connect dispatch、expect/acquire、receipt 与 replica 验证；
- handlerless method-bearing 因而取得 Host synthetic accept 对应的 exact receipt/pin；
- false 时不 select runtime、不 expect/acquire，保持 path-only zero pin；
- connect 拒绝、admission 失败、upgrade 失败或 bridge attach 失败都从同一 lifecycle finalize
  清 reservation、bridge generation 与已有 pin；
- direct/business `connection.send` 仍走原 lifecycle downlink，不进入 request broker。

新增单一职责 sibling `webSocketRpcConnectionAttachment.ts`，从 selected snapshot/binding 一次
捕获并复制：

- service/deployment、old assembly identity/generation、physical entry、host/path；
- exact singleton `jsonrpc-2.0-text` profile；
- 独立 copied method table 与 deployment timeout；
- handler/method 推导出的 pin requirement。

socket attach 时 sibling 再生成独立 socket generation，捕获 connection id、business identity、
observed writer、Router timeout、exact receipt/replica（若 pin）、runtime owner resolver 与幂等
release callback。message text/binary 只进入对应 bridge handle；RPC peer write 只使用
`capturePeerWriter()`，不直接调用 `socket.send`。

### 3.2 Old snapshot/runtime owner 与 fail-closed

- bridge capture 不再读取 current snapshot，也不重新 select runtime；
- snapshot replacement 后旧连接继续使用旧 method binding、deployment、assembly generation、
  business identity、receipt 与 replica；
- server owner resolver从 Endpoint source sender反查已注册 assembly replica；service id来自
  immutable captured connection/header exact match，bridge再验证 old assembly/generation；
- pinned connection 额外要求 exact replica 与 dispatcher receipt sender；
- path-only connection允许同 captured service 与 old assembly/generation 的 registered replica，
  不要求 pin；
- peer frame无法覆盖 service、entry、assembly、generation、gateway identity、business identity
  或 receipt。

### 3.3 Lifecycle、writer 与 teardown

`WebSocketConnectionLifecycle` 的 finish callback 现在可返回 Promise；lifecycle 会跟踪所有异步
finalize，`shutdown()` 在清 transport 后等待它们，并聚合 finalize failure。Gateway connection
finalize固定为：

```text
bridge/broker finalize
-> remove attachment state
-> idempotent generation release (only when pinned)
```

同一 finalize 被 peer close/error、writer failure、generation lost、attach failure与Gateway
shutdown共享。Gateway close先停止 upgrade，再 shutdown/await bridge finalizers，再 flush
generation lifecycle；各阶段即使失败也继续清理并最终聚合错误。

真实 loopback outbound 测试还暴露了 `ws` 成功 send callback 传 `null`、而 lifecycle 仅把
`undefined` 当成功的既有缺口。现已把 `null | undefined` 都作为成功 sentinel，并新增直接回归；
否则合法 outbound request会被误判为 writer failure、返回 `transportUnavailable` 并关闭 1011。

### 3.4 Bridge profile 与 server

`WebSocketRpcBridge` 保存 constructor 使用的唯一 profile adapter map，并通过
`captureProfileAdapter()` 返回同一 closed singleton adapter；重复 profile fail closed。

`server.ts` 创建唯一 bridge：

```text
RuntimeEndpoint + RuntimeDispatcher
-> WebSocketRpcBridge
-> AssemblyWebSocketGateway
```

shutdown 即使前项失败也继续，顺序为：

```text
AssemblyWebSocketGateway.close()
-> WebSocketRpcBridge.cleanup()
-> HTTP Gateway
-> RuntimeEndpoint
-> activation client
```

因此 Endpoint 不会在 bridge/Gateway teardown 前关闭。

## 4. Test 覆盖

新增真实 loopback `websocket-jsonrpc-gateway.test.ts`，直接组合 production Gateway、
production bridge、ephemeral HTTP/WebSocket server、fake Endpoint/dispatcher/generation owner：

1. handlerless method-bearing upgrade eager pin，并在 snapshot replacement 后用旧
   method/deployment/generation/business identity/receipt完成 inbound peer request；
2. path-only zero acquire，在 replacement 后由同一旧 owner完成 outbound request/peer response；
3. pinned generation lost关闭 peer `1011 / websocket runtime disconnected`，bridge/pin归零；
4. Gateway shutdown等待 bridge teardown，connection/pin/bridge accounting归零。

`websocket-gateway.test.ts` 另覆盖 connect拒绝、bridge attach failure、path-only frame attach、
binary protocol close，以及 existing connect policy与 direct/business downlink回归。
`websocket-connection-lifecycle.test.ts` 覆盖 async finalize wait 与 real `ws` null callback。

Z3A pure bridge tests继续覆盖：

- outbound success、remote error、runtime cancel与origin source disconnect隔离；
- inbound success/error/cancel/deadline、notification零dispatch、late completion fencing；
- source/service/entry/replica/receipt mismatch fail closed；
- binary/protocol error、writer rejection与peer disconnect最多一次terminal；
- broker generation cleanup、timer/tombstone/terminal lease accounting。

## 5. 规定 non-live 验证

最终代码状态按任务原样执行九文件 listing，exit `0`。精确非零计数：

| Test file | Count |
| --- | ---: |
| `websocket-jsonrpc-gateway.test.ts` | 4 |
| `websocket-gateway.test.ts` | 10 |
| `websocket-connection-lifecycle.test.ts` | 17 |
| `router-websocket-trust-dispatch.test.ts` | 3 |
| `runtime-assembly-request-wire.test.ts` | 58 |
| `runtime-assembly-websocket-jsonrpc-protocol.test.ts` | 5 |
| `runtime-assembly-websocket-jsonrpc-dispatch.test.ts` | 19 |
| `websocket-rpc-bridge.test.ts` | 32 |
| `websocket-generation-lifecycle-router.test.ts` | 6 |
| **Total** | **154** |

规定 run 结果：

```text
Test Files  9 passed (9)
Tests       154 passed (154)
```

其余规定检查：

| 命令 | 结果 |
| --- | --- |
| `pnpm --dir router type-check` | PASS |
| `git diff --check` | PASS |
| implementation `git diff --cached --check` | PASS |

## 6. 完成矩阵

| 任务条款 | 代码/测试证据 | 结论 |
| --- | --- | --- |
| handler或method决定 eager pin | Gateway `requiresRuntimePin`；真实 RED→GREEN | PASS |
| path-only zero acquire且old owner outbound合法 | production bridge loopback path-only test | PASS |
| attach-time old snapshot/method/receipt capture | snapshot replacement inbound test | PASS |
| peer text/binary进入opaque bridge | attachment sibling + Gateway frame tests | PASS |
| observed writer唯一RPC写路径 | lifecycle writer + real outbound/null sentinel test | PASS |
| mismatch、cancel、notification、late terminal fail closed | 32 bridge tests + 19 dispatcher tests | PASS |
| generation lost/attach failure/shutdown最多一次release | Gateway/JSON-RPC integration tests | PASS |
| Gateway/bridge/Endpoint shutdown顺序 | Gateway async finalize + server ordered cleanup | PASS |
| connect policy与`connection.send`不回归 | Gateway direct/business/policy tests | PASS |
| current v2 wire不回归 | wire/protocol/dispatch 82 tests | PASS |

## 7. Scope 与操作约束

- production只修改任务允许的 Gateway、attachment sibling、lifecycle、bridge与server；
- 未修改 snapshot/reader、artifact producer/schema、RuntimeDispatcher、RuntimeEndpoint、
  broker/profile state machine、generation router、Host/runtime/Rust、cross-system fixture、
  README/checker或 public config；
- 未启动 stable instance、watch、长期 server、external network或 live selector；
- 所有 loopback server与客户端由测试回收，临时 dependency symlink已删除；
- 未派子 Agent，未 merge、rebase或push；
- 无 `TASK_SCOPE_EXPANDED` blocker。
