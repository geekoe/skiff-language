# P5-F440Y Router RuntimeDispatcher WebSocket RPC sibling result

状态：`PASS / RECEIPT_PINNED_RUNTIME_LEG_COMPLETE`。

本节点只建立了 Router `RuntimeDispatcher` 到 captured runtime socket 的
`runtimeAssembly.websocketJsonRpc` sibling。Gateway、broker、snapshot、server、peer socket 与
source-disconnect/isolation hookup 均未接入；后继 F440Z 可以直接消费冻结 API。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明 implementation baseline | `484d55bea6b73c5dd776edb16a0b552a0f001448` | — |
| worktree 实际起点（只多本任务文档） | `039db56342744f559f6c46a8416ecab0666312e7` | — |
| implementation | `e23c4f245bdb64d154a66a173322955d580e696b` | `5d360452a270fcdb497d64ba390d69b5f071c3b4` |
| result | 本文独立提交 | 由最终交付消息记录 |

Worktree：
`/Users/geek/workspace/skiff-p5-f440y-router-rpc-dispatcher`

Branch：
`codex/p5-f440y-router-rpc-dispatcher`

## 2. Test-first RED

生产实现前先新增 direct sibling test，并使用真实目标 API 运行：

```text
router/node_modules/.bin/vitest run --root router \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
```

实际执行 `2` tests，`2 failed`：

1. `TypeError: dispatcher.dispatchAssemblyWebSocketJsonRpc is not a function`；
2. method-bearing request 被旧 connect-only guard 捕获，随后读取不存在的
   `request.websocketConnect.connectionId`。

这同时证明缺失的是实际 public sibling 与真实分类行为，不是零测试、synthetic probe 或依赖错误。

## 3. Public sibling 与 strict gates

`RuntimeDispatcher` 增加 current API：

```text
dispatchAssemblyWebSocketJsonRpc(
  { header, payloadBytes },
  timeoutMs,
  exact RuntimeDispatchConnectionReceipt,
  { signal }
) -> Promise<{ exact websocketJsonRpc response header, payloadBytes }>
```

协议层将 F440T method-bearing request 纳入 current executable
`RuntimeAssemblyRequestStartFrameWireHeader`，但 sender 仍只接受明确的 Router→Runtime executable
union。`RuntimeEndpoint` 通过现有 `RuntimeFrameSender` contract 机械获得这一精确 widening；direct
type gate 证明 inbound-only `connection.request` 仍不可作为 Endpoint outbound header。

request dispatch 在发送前复用 F440T header 与 payload strict gate：

- `protocol=webSocket`；
- method non-null；
- unary；
- `websocketJsonRpc` metadata/profile/identity exact；
- payload present 且在既有 1 MiB 上限内。

connect sibling 同时收紧为 `protocol=webSocket && method=null && websocketConnect present`，因此
method-bearing request 不再进入 acquire predicate。

## 4. Exact receipt 与 response correlation

receipt 仍由 `RuntimeDispatcher` 私有 brand 创建，并只通过 dispatcher-owned `WeakMap`恢复冻结的
`runtimeId/dispatchBuildId/socket` snapshot。JSON-RPC sibling：

- 不调用 runtime registry selection；
- foreign receipt 在发送前以 protocol boundary 拒绝；
- runtime disconnect 会使同 socket receipt records 过期；
- closed socket receipt 在发送前拒绝；
- request id 已 pending 时 fail closed，不覆盖现有 invocation。

pending 使用 exact captured socket、request id 与内部 execution token。只接受同 socket、同 request
id 的 strict F440T `response.end.websocketJsonRpc`：

- `success` 必须有 payload，JSON `null` 的四个 bytes保持合法；
- `invalidParams`、`internalError`、`deadlineExceeded` 必须没有 payload；
- `response.error`、connect/HTTP response branch及 payload mismatch只会拒绝当前 invocation，
  不会被投影为 typed JSON-RPC success；
- wrong id/socket与 detached invocation 的 late/duplicate response静默无效；
- concurrent requests可乱序完成。

每个 terminal 都先清除 pending、timer 与 abort listener，再 resolve/reject；response 赢后 late abort
不会发送 cancel。

## 5. Timeout 与 AbortSignal

timeout 与 abort 都先 detach，再 best-effort发送既有 `request.cancel`：

- timeout reason保持 `timeout`；
- `AbortSignal.reason` 是 current canonical `RequestCancelReason` 时原样发送；
- unknown/non-string reason降级为 `caller_cancel`；
- duplicate abort、settled 后 abort及 late response均为 no-op；
- cancel sender同步失败不重开已 detached pending。

direct fake-timer assertions在 timeout、canonical/unknown abort及 response-vs-abort race后同时观察到
pending `0`、timer `0`；cancel sender回调内已经观察到 pending `0`。

## 6. 规定 GREEN

worktree没有安装 dependency tree。验证时临时建立：

- `router/node_modules -> /Users/geek/workspace/skiff/router/node_modules`
- `node_modules -> /Users/geek/workspace/skiff/telemetry/node_modules`

用于现有 Vitest/TypeScript/MongoDB declarations；命令结束后两个链接均已删除。

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| direct Vitest listing（任务四文件） | `34` | PASS，非零 listing |
| direct Vitest run（任务四文件） | `34` | PASS，4 files，34/34 |
| `pnpm --dir router type-check` | — | PASS |
| `git diff --check HEAD^ HEAD` | — | PASS |

四文件分别为：

- `runtime-assembly-websocket-jsonrpc-dispatch.test.ts`；
- `runtime-assembly-websocket-jsonrpc-protocol.test.ts`；
- `runtime-endpoint-connection-send-trust.test.ts`；
- `router-websocket-trust-dispatch.test.ts`。

没有使用会产生零输出的 pnpm Vitest wrapper冒充 listing。

## 7. 自验收矩阵

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| exact receipt、不重选 registry | receipt record WeakMap + captured frozen connection | exact socket happy path；registry picker零调用；foreign/expired/closed拒绝 |
| request strict、connect分流 | shared request frame gate；method-null connect predicate | API/payload gate；method-bearing connect classification；protocol corpus |
| exact typed response | shared response header+payload gate | success/null；三 failure；wrong id/socket/branch/payload；response.error |
| concurrency与 late fencing | request-id pending map + per-invocation execution token | concurrent乱序；late/duplicate不完成新 pending |
| timeout/abort cleanup | detach-first terminal path；canonical reason guard | timeout、canonical/unknown abort、timer/pending归零 |
| single terminal race | abort listener cleanup + pending token checks | response-wins/abort-wins各最多一个 terminal/cancel |
| Endpoint executable type gate | current executable wire union through `RuntimeFrameSender` | JSON-RPC outbound encode；inbound-only frame conditional type拒绝 |
| connect sibling不回归 | exact method-null type guard | existing connect acquire/trust tests全部通过 |

## 8. 范围与反向审计

Implementation提交只修改任务写集内 `7` 个文件：

- protocol request/type/frame/response gate `3` 个；
- `runtimeDispatcher.ts`；
- direct dispatch、protocol与 Endpoint sender tests `3` 个。

`runtimeEndpoint.ts` 无需 production edit：其 existing sender 参数引用的 current executable union已经
精确 widened，direct test负责守住该 gate。

反向审计确认没有修改或新增：

- Gateway、broker、snapshot、server；
- RuntimeEndpoint callback/disconnect/isolation behavior；
- Host/Rust 或 F440T wire shape；
- peer terminal/cancel/deadline owner；
- 其它 task/result。

未派子 Agent；未手工启动 server、stable instance、watch或 live selector。只运行任务规定的 non-live
Router tests/type-check；未 merge、rebase或 push。
