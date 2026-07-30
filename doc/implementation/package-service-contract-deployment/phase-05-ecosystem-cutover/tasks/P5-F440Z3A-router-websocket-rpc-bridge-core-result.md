# P5-F440Z3A Router WebSocket RPC bridge core result

状态：`PASS / PURE_BRIDGE_COMPLETE`。

本节点用纯 captured connection 接口连接了 RuntimeEndpoint、profile-neutral broker 与
receipt-pinned RuntimeDispatcher sibling。实现不读取 current snapshot，不接 raw WebSocket、
Gateway、server 或 upgrade，不修改 Dispatcher behavior；真实 upgrade/attach 仍由 Z3B 拥有。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `e67e1b9dd3921140c67d9fb77b66c5b2e3364a04` | `87f04076b2381bfe7d6180224fd0d6192cdc6b8c` |
| worktree 实际起点 | `3c5b358bbcb7666fad46c2c1fb04692720706c0d` | `697db04a680089b607f2138b800a141dfacae9f2` |
| implementation | `1ffd685239126d9953bd8435ed7a6bd43de6deea` | `e7a0b9569d2ab53d95260f8b9934f791c31c5f8e` |

`e67e1b9d..3c5b358b` 是当前 phase-05 integration 上已有的父节点结果与本任务调度文档。
Implementation 与本文 result 分离提交；result commit/tree 由最终交付消息记录。

## 2. Test-first red

先新增 `tests/websocket-rpc-bridge.test.ts`，其中已经使用 fake Endpoint、dispatcher、captured
connection/writer 覆盖 outbound、inbound、source trust 与 finalize。首次直接调用 Vitest 时本
worktree 没有依赖，随后只临时链接已有 `/Users/geek/workspace/skiff/router/node_modules` 并重跑：

```text
router/node_modules/.bin/vitest run --root router \
  tests/websocket-rpc-bridge.test.ts
```

结果为 `1 failed suite`，production owner 创建前无法解析
`../src/gateway/webSocketRpcBridge.js`。这是目标 module 不存在产生的真实 compile RED，不是零匹配
selector 或 synthetic failure。

## 3. Captured connection 与 bridge API

新增 `router/src/gateway/webSocketRpcBridge.ts`，公开最小的 `WebSocketRpcBridge` 与 captured
connection/handle types：

- constructor 注册 Endpoint 的 request/cancel handler 与 exact source-disconnect handler；
  `cleanup()` 注销两者；
- `attach()` 捕获 socket generation、external connection id、service/deployment、old assembly
  identity/generation、physical WebSocket entry、host/path、profile/adapter、business identity、
  observed writer、timeout、runtime receipt/replica owner、source owner resolver 与 release callback；
- attach 时复制并冻结 method binding/deployment，后续不观察传入 map 的 mutation 或 current
  replacement；
- method-bearing connection 必须同时有 captured receipt 与 replica owner；pure path-only
  connection 才能不带 receipt；
- handle 只暴露 peer text、peer binary、peer disconnect/finalize 与 direct-test debug；
  不暴露 broker maps，也不要求 Gateway 组装 runtime wire。

连接 id、physical entry id 与 business identity 的 canonical validators 从现有
`runtimeAssemblyRequestMetadata.ts` 导出并由原 metadata validator 复用，没有改变 wire 接受集合。

## 4. Outbound runtime leg

- Endpoint `connection.request` 先验证 strict runtime header，再按 exact external connection、
  service、physical entry、profile、old assembly owner/generation 做 join；
- method-bearing generation 还要求 source sender 匹配 captured dispatcher receipt，且 replica
  匹配 captured owner；pure path-only 只允许同 service、old assembly identity/generation；
- unknown connection、foreign service/entry 返回 `connectionUnavailable`；foreign/stale
  runtime owner、profile 或 forged receipt source 先回 `protocolError`，再请求 Endpoint isolate
  offending source；
-合法 request 交给 broker，peer write 只通过 captured observed writer；broker terminal 通过
  `sendConnectionResponse` 回原 Endpoint source；
- runtime cancel 走 broker global `(sender, sessionToken, requestId)` key，先 detach/tombstone，
  best-effort peer cancel，零普通 response；
- Endpoint source disconnect 只清理该 exact sender/session 的 outbound pending；
- peer success、remote error、deadline、binary protocol close、disconnect 与 writer rejection 都按
  broker owner 最多完成一次 terminal。

## 5. Inbound peer leg

- peer text/binary/disconnect 直接交 broker；当前 text profile 对 binary 固定关闭 `1003`；
- inbound method 只查 attach-time copied method table，并用 old assembly identity/generation、
  captured deployment/method gateway identity、physical entry、host/path/method/profile、connection
  id 与 captured business identity 构造 F440Y request header；
- peer params 作为 opaque profile payload 转换，不能覆盖或注入 captured business identity；
- dispatcher 只消费 captured generation receipt，不 current-select runtime；
- success payload 使用 captured profile adapter
  `fromRuntimePayload(..., "inboundResult")`；`invalidParams`、`internalError`、
  `deadlineExceeded` 与 dispatcher unavailable/protocol rejection 映射到冻结 profile terminal；
- ordinary notification 不调用 dispatcher，也不建立 response terminal；
- peer cancel、inbound deadline、peer disconnect 与 protocol close 在 broker 中先 detach/tombstone，
  再以 canonical `caller_cancel`、`deadline_exceeded`、`client_disconnect` 或 `protocol_error`
  abort dispatcher leg；
- late Promise completion 因 execution token/entry 已失效而不能再次写 peer；同值 inbound/outbound
  peer id 使用独立方向索引。

## 6. Broker limits、timeout 与 teardown

`webSocketRequestBrokerTypes.ts` 现在拥有唯一冻结的
`DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS`，bridge 直接消费它，不复制 capacity、TTL 或 profile
limit magic constants。Broker generation 额外捕获 exact profile adapter 和
`inboundTimeoutMs`；bridge 将其计算为：

```text
min(routerRequestTimeoutMs, captured deploymentTimeoutMs)
```

deployment timeout 缺省时使用 Router timeout。两个输入都在 attach 时验证为 positive safe
integer。Broker 每个 generation 使用自己的 captured timeout，不读取后续配置。

Broker 还把 runtime protocol response 调整为先发送 terminal、再调用 isolation callback，并为
inbound cancel/deadline/disconnect/protocol/terminal-writer failure提供 canonical AbortSignal reason。
Generation teardown 会同步清 active indexes、timer、terminal lease 与该 generation tombstones。

`finalize()` 顺序固定为 broker peer-disconnect teardown、移除 bridge indexes、再调用 generation
release；重复 peer-disconnect/finalize/release 最多一次。`cleanup()` 对所有 generation 执行同一
顺序，并在 direct snapshot 中证明 active、timer、tombstone、terminal lease 与 attached count
全部归零。

## 7. Focused validation

最终使用任务指定的四个 non-live Router selector。实际 listing 非零：

| Test file | Count |
| --- | ---: |
| `tests/websocket-rpc-bridge.test.ts` | 32 |
| `tests/websocket-request-broker.test.ts` | 29 |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 7 |
| `tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts` | 19 |
| **Total** | **87** |

```text
router/node_modules/.bin/vitest list --root router \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
```

结果：listing exit `0`，`87` 个非零测试。

```text
router/node_modules/.bin/vitest run --root router \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
```

结果：`4 files passed`，`87 passed / 87 total`。

| Check | Result |
| --- | --- |
| direct Vitest listing | PASS，87 non-zero |
| direct Vitest execution | PASS，87/87 |
| `pnpm --dir router type-check` | PASS |
| `git diff --check` | PASS |
| implementation `git diff --cached --check` | PASS |

type-check 临时使用：

- `router/node_modules -> /Users/geek/workspace/skiff/router/node_modules`
- root `node_modules -> /Users/geek/workspace/skiff/telemetry/node_modules`

两个链接均已删除；没有安装依赖。未展开完整 Router suite。

## 8. Scope audit

Implementation 只新增/修改：

- `router/src/gateway/webSocketRpcBridge.ts`
- `router/src/index.ts`
- `router/src/protocol/runtimeAssemblyRequestMetadata.ts`
- `router/src/router/webSocketRequestBroker.ts`
- `router/src/router/webSocketRequestBrokerTypes.ts`
- `router/tests/websocket-request-broker.test.ts`
- `router/tests/websocket-rpc-bridge.test.ts`

没有修改 `webSocketGateway.ts`、`webSocketConnectionLifecycle.ts`、server、snapshot/reader、
`runtimeEndpoint.ts`、RuntimeDispatcher behavior、broker state/wire helper、Rust、其它 task/result。
没有 raw WebSocket、current snapshot 或 live owner 依赖；captured context 所需字段都能由父 checkpoint
唯一提供，因此没有触发 `TASK_SCOPE_EXPANDED`。

未启动 server、stable、instance、network 或 live 测试；未派子 Agent；未 merge、rebase 或 push。
