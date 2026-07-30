# P5-F440Z2 Router runtime-source / socket writer lifecycle result

状态：`PASS / ENDPOINT_LOCAL_LIFECYCLE_COMPLETE`。

本节点只建立了 `RuntimeEndpoint` exact request-source disconnect/isolation，以及
`WebSocketConnectionLifecycle` captured observed peer writer。没有注册 broker callback、解析
JSON-RPC、接 Gateway request dispatch，或修改 Gateway/broker/server/dispatcher。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `85ff15138c947d1bc776a4d6c27076d7a783a1ac` | — |
| worktree 实际起点 | `07e0c25db4c80a79f73c0d3fd495884733865bf1` | — |
| implementation | `c8f6b08410616b8fb384daf04ed1c61f7af6f995` | `04e419db3be120fd7fafdda5f634fc738efde8d2` |
| result | 本文独立提交 | 由最终交付消息记录 |

`85ff1513..07e0c25d` 只新增 F440Z1/F440Z2 两个 task 文档，没有 production/test
变化。Implementation 与本文 result 分离提交。

## 2. Test-first RED

生产实现前，先在 `websocket-connection-lifecycle.test.ts` 增加真实 callback-failure
用例并执行：

```text
router/node_modules/.bin/vitest run --root router \
  tests/websocket-connection-lifecycle.test.ts
```

实际执行 `8` tests，结果 `1 failed / 7 passed`。新增用例在真实目标 API 处失败：

```text
TypeError: lifecycle.capturePeerWriter is not a function
```

因此 RED 证明缺少的是 broker 可捕获的 lifecycle writer，而不是零测试、依赖失败或
synthetic probe。

## 3. Runtime source lifecycle

`RuntimeEndpoint` 现在提供任务冻结的两个 API：

```text
onConnectionRequestSourceDisconnect(handler) -> unsubscribe
isolateConnectionRequestSource(source, reason) -> void
```

每条 runtime WebSocket session 继续由单调 token fence；第一次
`connection.request` / `connection.request.cancel` 时冻结 canonical
`{sender, sessionToken}` source，同 session 后续消息复用同一 source object。

close、error、Endpoint shutdown 与 isolation 进入同一个一次性 cleanup：

- exact source disconnect handlers 在 session token、capability/assembly registration 和 sender
  cleanup 前同步执行；
- sender `WeakSet` fence 使 error/close、isolate/close 及 shutdown/close 竞态最多通知一次；
- handler 使用冻结 snapshot 逐个调用；一个 handler 抛错会记录
  `runtime.connection_request_source_disconnect_handler_error`，不阻止其它 handler 或后续 cleanup；
- unsubscribe 幂等，移除后不接收后续 source disconnect；
- cleanup 后 `sendConnectionResponse` 同时受 disconnected sender、session token 与 OPEN socket
  fence，旧 source 不能完成重连 session；
- 同 runtime id 重连产生新 sender、新 token、新 canonical source，旧 source isolate/response 均不影响
  replacement。

Isolation 只接受当前 OPEN、capability-registered 且 sender/token 精确匹配的 source。stale、forged
或 cross-runtime tuple 都 fail-closed no-op。调用成功时先记录
`runtime.connection_request_source_isolated`，diagnostic reason 以 UTF-8 `512` bytes 为上限；runtime
socket 只收到固定 `1008 / runtime request source isolated`，diagnostic reason 不进入 peer/business
payload。

## 4. Observed peer writer

`WebSocketConnectionLifecycle.capturePeerWriter(connectionId)` 返回结构上可直接作为 broker
`CapturedPeerWriter` 注入的 captured writer：

```text
writeText(frame) -> Promise<void>
close(code, reason) -> void
```

writer 捕获 exact lifecycle connection object，不按 id 重新查找，因此 id 被 replacement 复用后，旧
writer 不能写入或关闭新 socket。

每次 text write 在调用 `ws.send` 前安装一次性 observed lease：

- 只有 send callback success 且 socket/connection 仍为 exact OPEN/admitted 才 resolve；
- callback error、同步 throw、发送前非 OPEN，以及 callback-success 与 CLOSING 的竞态均明确 reject；
- outstanding text bytes 与当前 `socket.bufferedAmount` 一起消费既有 slow-client budget；
- callback terminal 精确释放 write bytes/lease；
- peer close、transport error、policy close、runtime disconnect、explicit close 与 shutdown 统一
  reject outstanding writes；
- write entry 从 exact `Set` detach 后才 resolve/reject，late/duplicate callback 无法再次 terminal；
- callback/send failure 以既有 `1011 / websocket client send failed` 关闭该 exact connection；
- 既有 fire-and-forget downlink、binary、policy close 与 shutdown transport behavior 没有放宽或改写。

`observedWriteCount()` 提供 direct lifecycle accounting oracle；全部 success/failure/race 测试终态均为
零。

## 5. 规定验证

worktree 没有安装依赖。验证期间临时建立：

- `router/node_modules -> /Users/geek/workspace/skiff/router/node_modules`
- 根 `node_modules -> /Users/geek/workspace/skiff/telemetry/node_modules`

用于已有 Vitest/TypeScript/MongoDB declarations；验证后两个 symlink 均已删除，没有安装依赖。

规定 listing：

```text
router/node_modules/.bin/vitest list --root router \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/websocket-connection-lifecycle.test.ts
```

精确列出 `27` 个非零测试：

- runtime Endpoint connection send/source trust：`7`
- runtime Endpoint source lifecycle：`5`
- WebSocket connection lifecycle：`15`

规定 execution：

```text
router/node_modules/.bin/vitest run --root router \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/websocket-connection-lifecycle.test.ts
```

结果：`3 files passed`，`27 passed / 27 total`。

| Check | Result |
| --- | --- |
| direct Vitest listing | PASS，27 non-zero |
| direct Vitest execution | PASS，27/27 |
| `pnpm --dir router type-check` | PASS |
| `git diff --check` | PASS |
| `git diff HEAD^ HEAD --check`（implementation） | PASS |

## 6. 自验收与范围审计

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| exact disconnect once / cleanup 前通知 | unified sender WeakSet cleanup；handler 在 token/registry delete 前 | close 与 shutdown 观察 exact object、current token/registration；重复 shutdown 不重发 |
| reconnect/session fencing | per-session canonical source + sender/token/disconnected gates | same runtime id 新 source；旧 response/isolate、forged/cross-runtime token 均无权影响新 session |
| handlers/unsubscribe/throw isolation | handler snapshot、幂等 unsubscribe、per-handler catch diagnostic | throwing handler 后 survivor 仍执行；removed handler 不执行；socket 仍按 1008 关闭 |
| Endpoint-owned isolate | exact current capability/session admission；bounded diagnostic；fixed close payload | current 成功；stale/foreign no-op；512-byte diagnostic 与固定 close reason |
| observed callback terminal | Promise writer + callback/throw/OPEN gates | success、callback error、sync throw、non-open、CLOSING callback race |
| outstanding accounting / single terminal | exact write Set + bytes；detach-first settle；finish rejects all | slow budget、close-vs-send、late callback、各终态 count 0 |
| captured writer isolation | writer closes over lifecycle connection object | replacement id 不接收旧 writer write/close |
| existing behavior | legacy `send`、binary、close helpers保持原路径 | 原有 7 个 lifecycle tests 与 7 个 Endpoint trust tests全部通过 |

Implementation commit 只修改任务写集内 `4` 个文件：

- `router/src/router/runtimeEndpoint.ts`
- `router/src/gateway/webSocketConnectionLifecycle.ts`
- `router/tests/runtime-endpoint-source-lifecycle.test.ts`
- `router/tests/websocket-connection-lifecycle.test.ts`

没有修改 Gateway 主模块、broker、`RuntimeDispatcher`、server/snapshot/protocol wire、Rust、其它
task/result 或 public slow-client policy。没有启动 stable instance、watch、手工 server 或 live
selector；只执行规定 non-live Router tests/type-check。未派子 Agent；未 merge、rebase或 push。
