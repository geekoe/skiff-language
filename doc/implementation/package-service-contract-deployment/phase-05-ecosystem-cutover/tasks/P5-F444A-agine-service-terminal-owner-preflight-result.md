# P5-F444A Agine service terminal owner preflight result

状态：`TASK_SCOPE_EXPANDED`。

本节点是只读预检，不实现。F443B 把候选阻塞收敛为 IA1 manifest migration，但当前
Internals 输入证明这不是单纯的 36-entry 文件搬移：

- `@agine/protocol/http` 和 Agine client 已经拥有
  `/thread/host-files/list`、`/thread/host-files/search`，Agine service 尚无这两个 HTTP
  entry、route 或 handler；
- Host 仍通过 raw WebSocket event 上行 `host/hello`、`host/activation-ack`、
  `host/ping`、`host/tool-attempts` 和 Host `tool_call/result`，并等待 raw event response；
- 当前 Skiff `std.websocket` 不公开 raw receive。`websocket.yml.jsonRpc`只拥有
  **peer -> Skiff** inbound method；Agine 的三个 Host method是
  **Skiff -> Host** outbound `requestJsonToConnection`，不能用 `jsonRpc`伪造其声明；
- 当前 service 仍没有任何 `std.websocket.requestJsonToConnection` 调用，且仍拥有完整
  raw receive、event DTO、两层 transport correlation、current-directory
  `refreshRequested` 和 connection cache。

因此必须先闭合一个 `agine/protocol/** + agine/host/**` authenticated HTTP producer leaf，
再由唯一的 `agine/service/**` terminal leaf一次删除 raw receive并落地 direct Host peer RPC。
两个 production 写集互斥、依赖显式；没有把同一 service 文件交给并行 leaf。

## 1. 输入与审计边界

| Repo | 任务输入 | 实际 HEAD | Tree / 状态 |
| --- | --- | --- | --- |
| Skiff integration | `eea50e12507e3a14fe1edfd0363bffd9a0938f79` | `17d7fe8e59728aaac22a21a82c0cd9c3291ff33a` | `c76bdd8d167e32f01f0b0f284150c087eb5e1efc` / clean |
| Internals | `232094902785c6e725adafa6f4dc42137a1647b4` | exact | `0178f3282eec1c07cdd031a365abd580fa0f204f` / clean |
| skiff-packages | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` | exact | `44081bd0498919086c13adea97c07722cb768352` / clean |

Skiff actual HEAD 相对任务输入只新增
`P5-F444A-agine-service-terminal-owner-preflight.md`；production 与 reference tree
相对 `eea50e12` 无变化。Internals 和 skiff-packages 精确匹配任务 commit。

读取限于五个直接父结果、必要的 F438B/F440A owner引用、任务列出的 Skiff reference/std
文件、`agine/service/**`，以及直接确认 producer/consumer 边界的
`agine/protocol/**`、`agine/host/**`、`agine/client/**` 符号。没有开放式审计其它仓库。

## 2. 最终 authoring 冻结

### 2.1 `service.yml`

终态精确为：

```yaml
id: agine.ai/api
```

Agine 没有 service-to-service callable root，故 `serviceCalls`省略。`http`、`websocket`和
`timeout`均非法；120 秒 deployment 上限已经由 `config.dev.yml:timeout`拥有。

### 2.2 `http.yml`

不能机械承接当前 36 条 entry。终态是 43 条顶层 direct mapping：

1. 原 36 条 entry原样保留 key、method、path、kind、handler和adapterArgs；
2. 增加 browser Host read：
   - `threadHostFilesListPost` -> `POST /thread/host-files/list`
   - `threadHostFilesSearchPost` -> `POST /thread/host-files/search`
3. 增加 Host authenticated business upcall：
   - `hostHelloPost` -> `POST /host/hello`
   - `hostActivationAckPost` -> `POST /host/activation-ack`
   - `hostPingPost` -> `POST /host/ping`
   - `hostToolAttemptsPost` -> `POST /host/tool-attempts`
   - `hostToolCallResultPost` -> `POST /host/tool-call/result`

每条仍是同一个 raw HTTP adapter：

```yaml
<entryKey>:
  method: POST
  path: <path>
  kind: rawHttp
  handler: internal.agine_service.handleAgineHttp
  adapterArgs:
    - param: request
      source: { kind: http.request }
```

前 38 条 ordinary/browser route和最后 5 条 Host route必须在
`agine_http_dispatch.handle`中先按 route 分类；Host route在 browser session guard之前进入严格
Host header authentication，不能接受 browser cookie authority、caller-supplied owner、
business identity或connection id。

当前证据：

- inline manifest恰有 36 条；
- `agine/protocol/http.ts:AGINE_HTTP_POST_PATHS`已包含两个 Host-file path；
- `agine/client/src/lib/hostFileApi.ts`已经调用这两个 path；
- `agine/service/internal/agine_http_routes.skiff`和service receipt仍明确排除它们；
- Host五项上行仍由 `GatewayClient.send*` / `HostRuntime` raw event producer拥有。

### 2.3 `websocket.yml`

终态精确为：

```yaml
path: /ws

connect:
  handler: internal.agine_connect.acceptConnection
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
```

`jsonRpc`应**省略**，不是声明三个 Host method，也不需要写空 mapping。

归属证明：

- `service-yml.md` §4：`jsonRpc`只声明 peer-initiated inbound method；
  `requestJsonToConnection`是 outbound host operation，不要求列入 `jsonRpc`；
- `api-yml.md` §8：outbound codec来自调用点 concrete
  `requestJsonToConnection<TRequest,TResponse>`，不生成 ServiceContract 或 inbound gateway entry；
- `runtime.md` §11：peer response只恢复原 request frame，不创建 ingress request；
  declared peer method才创建独立 gateway request frame。

Agine 只主动调用 Host 的
`host.files.list`、`host.files.search`、`host.current-directory`，Host没有主动调用
Agine JSON-RPC method，所以 declared inbound map为零。

### 2.4 connect callback

不需要从 `internal.agine_service.websocket`重新提取 connect逻辑：
`internal.agine_connect.acceptConnection`已经是独立函数，manifest可直接引用它。

但函数不能按当前源码原样通过新 std surface，必须在 terminal service leaf中做形态收敛：

- 返回 `std.websocket.WebSocketConnectResult`，不再使用旧
  `WebSocketConnectResult<ConnectionContext>`；
- accept result只返回 `tag`、`businessIdentity`、`connectionPolicy`，删除旧 `context`字段；
- 继续从 `WebSocketConnectRequest.connectionId`完成 Host active connection持久化和 user
  notification business identity绑定；
- `internal.agine_service.websocket`统一 ingress-event dispatcher删除。

## 3. 当前图与目标图

### 3.1 list/search 当前没有 HTTP 图

当前 HTTP authoring、route table和handler均没有 list/search。唯一存活图是旧 raw WebSocket：

```text
agine_service.websocket(receive)
  -> agine_ws_dispatch.receive
  -> agine_ws_host_tool_files.dispatch
  -> dispatchUserFileRequest
  -> host_toolprovider_runtime.dispatchHostFileBrowseRequest
  -> host_file_rpc.dispatchHostFileBrowseRequest
  -> resolveThreadHostBinding
  -> db insert HostFileBrowseRequest
  -> sendJsonToConnection(activeConnectionId, host/files/*-request)

Host raw result
  -> dispatchHostFileResult
  -> host_toolprovider_runtime.receiveHostFileBrowseResult
  -> host_file_rpc.receiveHostFileBrowseResult
  -> transaction claim/delete HostFileBrowseRequest
  -> sendJsonToConnection(browserConnectionId, */response)
```

目标 list图：

```text
handleAgineHttp
  -> agine_http_dispatch.handle
  -> agine_http_routes.resolve("thread-host-files-list")
  -> agine_http_host_files.list
  -> resolveThreadHostBinding(owner, chatId, mountId, toolProviderId)
  -> canReadFiles + host.files.v1 + presence/online
  -> exact binding.toolProvider.activeConnectionId
  -> timeout(15000) {
       host_peer_rpc.listFiles(
         exactConnectionId,
         HostFilesListParams { path }
       )
     }
  -> std.websocket.requestJsonToConnection<
       HostFilesListParams,
       HostFilesListResult
     >(connectionId, "host.files.list", params)
  -> business result/error projection
  -> direct HTTP HostBrowseDirectoryResult
```

search图相同，route/tag/function/method/type分别为
`thread-host-files-search`、`agine_http_host_files.search`、
`host.files.search`、`HostFilesSearchParams`、`HostFilesSearchResult`；query长度上限仍在发出
platform request前验证。

### 3.2 current-directory 当前图

```text
handleAgineHttp
  -> agine_http_dispatch.handle
  -> agine_http_tool_providers.toolProvidersCurrentDirectory
  -> host_toolprovider_runtime.currentDirectoryForToolProvider
  -> host_toolprovider_connection.currentDirectoryForToolProvider
  -> ownedActiveHostToolProvider
  -> read ToolProvider metadata currentDirectory cache
  -> cache miss:
       requestHostCurrentDirectoryRefresh
       -> sendTextToBusinessIdentity(host/current-directory/request)
       -> return currentDirectory:"", refreshRequested:true
```

目标图：

```text
handleAgineHttp
  -> agine_http_dispatch.handle
  -> agine_http_tool_providers.toolProvidersCurrentDirectory
  -> host_toolprovider_runtime.currentDirectoryForToolProvider
  -> host_toolprovider_connection.currentDirectoryForToolProvider
  -> ownedActiveHostToolProvider(owner, toolProviderId)
  -> host.files.v1 + presence/online + exact activeConnectionId
  -> timeout(15000) {
       host_peer_rpc.currentDirectory(exactConnectionId, {})
     }
  -> std.websocket.requestJsonToConnection<
       HostCurrentDirectoryParams,
       HostCurrentDirectoryResult
     >(connectionId, "host.current-directory", {})
  -> optional synchronous metadata refresh
  -> { toolProviderId, currentDirectory }
```

每次 HTTP request等待 Host结果；不存在 `refreshRequested`、polling、detached refresh或
business request id。metadata可作为 presence/display cache更新，但不再是返回结果的短路 owner。

### 3.3 当前仍存活的 legacy owner

| 类别 | 当前 production owner | 终态 |
| --- | --- | --- |
| raw receive | `agine_service.websocket` -> `agine_ws_dispatch.receive` -> 五个 `agine_ws_*` dispatcher | 全部删除；其它当前业务没有合法 final raw receive owner |
| browser legacy request DTO | `api/agine.skiff`中的 `Chat*Input`、`Agent*Input`、`ToolsListInput`、`ToolProviders*Input`、`ThreadToolProviders*Input`、`ThreadHostFiles*Input` | HTTP payload/command已有owner者删除Input；无consumer的tools/mount cases直接删除 |
| Host raw event DTO | `ToolCallResultInput`、`HostHelloInput`、`HostActivationAckInput`、`HostPingInput`、`HostFilesResultInput`、`HostCurrentDirectoryInput`、`HostToolAttemptsInput` | Host HTTP payload或private peer record取代；eventName/requestId删除 |
| generic envelopes | `ClientMessage`、`ServerEnvelope`、`agine_transport.WebSocketRequest`及decode/success/error/send helpers | 删除；server notification business envelope不受影响 |
| file DB relay | `model.HostFileBrowseRequest`及3 indexes、`host_file_rpc`全部TTL/insert/claim/delete/browser relay | 删除，由同一 request frame中的broker pending取代 |
| current-directory polling | `requestHostCurrentDirectoryRefresh`、`refreshRequested`、Host `host/current-directory` raw result | 删除 |
| browser connection cache | `model.ChatStreamConnection`、`chat_stream.rememberConnection`、DB fan-out scan | 删除；notification直接 `sendTextToBusinessIdentity` |
| exact Host connection | `ToolProvider.activeConnectionId`，connect auth/activation写入 | 保留，是三个 outbound request的唯一connection owner |
| durable tool identity | `HostToolAttempt`、`toolCallId`、`attemptId`、`runId`、settlement/reconciliation | 保留；不得用transport id替代 |

`agine_ws_dispatch.receive`在当前候选仍有真实 Host producer，所以现在不能孤立删除；但按当前
reference它没有任何合法**终态** owner。browser request已迁 HTTP；server -> browser/Host chat/tool
notification是 outbound raw send，只证明 send owner，不证明 receive owner。

## 4. Host peer private protocol冻结

唯一跨语言 fixture继续是
`agine/protocol/fixtures/host-peer-jsonrpc-v1.json`。Skiff private声明放在
`agine/service/internal/host_peer_protocol.skiff`，不放入 `api.yml`或ServiceContract：

```text
HostFilesListParams { path: string? }
HostFilesSearchParams { path: string?, query: string }
HostCurrentDirectoryParams {}

HostBrowseBreadcrumb { name: string, path: string }
HostFileEntry {
  name: string,
  type: "directory" | "file",
  size: number?,
  path: string,
  relativePath: string?
}
HostBrowseDirectoryResult {
  root: string,
  cwd: string,
  parent: string?,
  breadcrumbs: Array<HostBrowseBreadcrumb>,
  items: Array<HostFileEntry>,
  truncated: bool
}
HostBrowseSearchResult {
  root: string,
  cwd: string,
  matches: Array<HostFileEntry>,
  truncated: bool
}

HostFilesListResult discriminator "kind" =
  { kind: "ok", value: HostBrowseDirectoryResult }
  | { kind: "invalidPath" }
  | { kind: "outsideWorkspace" }

HostFilesSearchResult discriminator "kind" =
  { kind: "ok", value: HostBrowseSearchResult }
  | { kind: "invalidPath" }
  | { kind: "outsideWorkspace" }

HostCurrentDirectoryResult { currentDirectory: string }
```

这些business params/result/nested records不得出现 `id`、`requestId`、connection id、
toolProviderId、owner或browser identity。Opaque JSON-RPC `id`只由Router broker与Host adapter outer
wire拥有。

### 4.1 business与platform error投影

| Caller结果 | Public HTTP projection |
| --- | --- |
| list/search `kind: ok` | 直接返回 `value` |
| `invalidPath` | `host_files_invalid_path`, HTTP 400 |
| `outsideWorkspace` | `host_files_path_outside_root`, HTTP 400 |
| `connectionUnavailable` / `transportUnavailable` | `host_offline`, HTTP 503 |
| `protocolError` | `host_files_protocol`, HTTP 502 |
| `resourceLimit` | `host_files_failed`, HTTP 502 |
| remote `-32700/-32600/-32601/-32602` | `host_files_protocol`, HTTP 502 |
| remote `-32001` | `host_files_timeout`, HTTP 504 |
| remote `-32603/-32000/-32800`或未知integer | `host_files_failed`, HTTP 502 |
| caller `TimeoutError` | `host_files_timeout`, HTTP 504 |
| request encode / success decode `std.json.DecodeError` | `host_files_protocol`, HTTP 502 |
| ancestor cancellation | 不可捕获；直接终止HTTP lane并best-effort cancel peer |

Remote `message/data`是不可信值，不进入public message或普通日志。`agine_transport.apiErrorHttpStatus`
必须显式拥有上述 400/502/503/504 映射；不能落入当前默认 400。

## 5. 精确 production 与 test/receipt owner

### 5.1 Terminal service leaf：删除

- production files：
  - `internal/agine_ws_dispatch.skiff`
  - `internal/agine_ws_chat.skiff`
  - `internal/agine_ws_agent_provider.skiff`
  - `internal/agine_ws_tool_providers.skiff`
  - `internal/agine_ws_access.skiff`
  - `internal/agine_ws_host_tool_files.skiff`
  - `internal/host_file_rpc.skiff`
- `internal/model.skiff`：
  `HostFileBrowseRequest`、`ChatStreamConnection`及其全部indexes；
- `internal/agine_service.skiff:websocket`；
- `internal/agine_transport.skiff`：
  `WebSocketRequest`、`decodeWebSocketRequest`、`successEnvelope`、
  `errorEnvelope`、`sendResponse`、`sendError`、`sendErrorToConnection`及只为它们存在的copy helpers；
- `internal/host_toolprovider_connection.skiff:requestHostCurrentDirectoryRefresh`；
- `internal/host_toolprovider_runtime.skiff`：
  `dispatchHostFileBrowseRequest`、`receiveHostFileBrowseResult`；
- `api/agine.skiff`：
  所有只由上述 raw receive消费的 `*Input`，以及 `ClientMessage`、`ServerEnvelope`。

### 5.2 Terminal service leaf：新增/替换

- authoring：
  `service.yml`、新`http.yml`、新`websocket.yml`、`api.yml`；
- new private RPC：
  `internal/host_peer_protocol.skiff`、`internal/host_peer_rpc.skiff`；
- HTTP：
  `internal/agine_http_routes.skiff`、`internal/agine_http_dispatch.skiff`、
  `internal/agine_http_tool_providers.skiff`、新`internal/agine_http_host_files.skiff`、
  新`internal/agine_http_host.skiff`；
- auth/connect：
  `internal/agine_connect.skiff`和新共享严格header parser
  `internal/host_transport_auth.skiff`；
- owner updates：
  `internal/host_toolprovider_connection.skiff`,
  `internal/host_toolprovider_runtime.skiff`,
  `internal/host_toolprovider_registry.skiff`,
  `internal/agine_transport.skiff`,
  `internal/chat_stream.skiff`,
  `internal/model.skiff`,
  `api/agine.skiff`。

`api.yml`终态为显式空 mapping `{}`：

- external HTTP/connect handler不要求进入Package API；
- private peer record也不进入Package API；
- `ConnectionContext`等内部source type可继续留在`api/agine.skiff`，但没有外部package consumer。

`api/agine.skiff`会因删除legacy event DTO和增加Host HTTP neutral payload而变化；不是因为external
schema必须公开，也不能把private peer records放进去伪造public owner。

### 5.3 test / receipt

删除或重写：

- `internal/agine_service_dispatch.test.skiff`
- `internal/agine_service.delete_contract.test.skiff`
- `internal/host_file_browse.test.skiff`
- `internal/host_toolprovider_current_directory.test.skiff`
- `internal/agine_service_architecture.test.mjs`
- `internal/host_runtime_architecture.test.mjs`
- `service-api-receipt.mjs`
- `service-api-receipt.test.mjs`

并更新直接受 `ChatStreamConnection`、Host lifecycle/raw DTO删除影响的聚焦测试。新receipt必须证明：

- `service.yml`只有`id`；
- `http.yml`精确43条、selector/key唯一；
- `websocket.yml`只有path/connect，`jsonRpc`、`routes`、`operation`、receive/message均不存在；
- Package API paths为空，ServiceContract仍为零operation；
- gateway closure为43 HTTP + 1 connect + 0 inbound JSON-RPC method；
- fixture三项business params/result无 `id|requestId`。

## 6. 范围扩张与最小可执行 DAG

### H1 — Host authenticated HTTP producer cutover

依赖：当前 F440D protocol checkpoint、F440F Host private peer adapter。

Production写集严格限制为：

- `agine/protocol/{http.ts,toolCall.ts}`及其package-local tests；
- `agine/host/src/**`，重点为
  `GatewayClient.ts`、`HostRuntime.ts`、`HostToolAttemptRuntime.ts`、
  `protocol/toolCall.ts`和新Host HTTP client。

职责：

- 冻结五条Host HTTP path/payload/response；
- hello/activation-ack/ping/tool-attempt sync/Host tool result改authenticated HTTP；
- tool-attempt action和tool-result receipt从同一HTTP response恢复；
- WebSocket只保留connect/reconnect、Host peer responder和接收server business notification；
- 不修改 `agine/service/**`。

RED：

```bash
rg -n 'eventName: "host/(hello|activation-ack|ping|tool-attempts)"|eventName: "tool_call/result"' \
  agine/host/src --glob '!*.test.ts'
```

当前必有 production命中。聚焦验证owner：
`GatewayClient.test.ts`、`HostRuntime.test.ts`、`HostToolAttemptRuntime.test.ts`、
`protocol/toolCall.test.ts`、新Host HTTP client test、Host architecture gate。

### S1 — Agine service terminal connect-only cutover

依赖：H1完成；当前 Skiff shared JSON-RPC/request/cancel与external-manifest checkpoint。

Production写集：只允许上文 §5.1/§5.2 的 `agine/service/**`；同一leaf独占所有service
manifest/source/model/API文件。

职责：43 HTTP、connect-only WebSocket、三个direct peer call、error投影、legacy raw/DB/poll/cache
删除。

首次 RED应先改Node receipt/architecture assertions，使当前候选同时失败于：

- 仍inline manifest且HTTP count 36；
- list/search route缺失；
- `requestJsonToConnection`零调用；
- raw receive/DB relay/refreshRequested仍存在；
- `api.yml`仍导出legacy websocket。

聚焦验证：

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs \
  agine/service/internal/host_runtime_architecture.test.mjs
```

随后运行 isolated service的Host peer/HTTP `.skiff` tests，覆盖fixture三success、两个business
union、五个 `WebSocketRequestError` branch、remote platform integer、TimeoutError、DecodeError、
exact connection、owner/mount/capability、ancestor cancel不投影，以及connect两类auth。

终态反向搜索：

```bash
rg -n 'WebSocketIngressEvent|agine_ws_dispatch\\.receive|HostFileBrowseRequest|ChatStreamConnection|refreshRequested|requestHostCurrentDirectoryRefresh' \
  agine/service
rg -n 'host/(files/(list|search)-(request|result)|current-directory(/request)?)' \
  agine/service
rg -n 'requestJsonToConnection|WebSocketRequestError|TimeoutError|std\\.json\\.DecodeError' \
  agine/service/internal
rg -n '\\b(id|requestId)\\b' agine/service/internal/host_peer_protocol.skiff
```

前两组 production应为零，第三组应命中完整caller/projector，第四组应为零。

### C1 — read-only combined gate

依赖：H1 + S1。无production写集。运行protocol/Host/service/client focused tests，然后恢复F443B
Gate C。

## 7. F443B Gate C恢复命令

原F443B专用Skiff worktree已经不存在；其production当时与integration bit-identical。实现合入候选后，
使用当前integration root作为显式 `SKIFF_ROOT`：

在 `/Users/geek/workspace/internals-phase-05-integration/aihub/service`：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  npm run type-check
```

在 `/Users/geek/workspace/internals-phase-05-integration/agine/service`执行同一命令。

两个canonical workflow必须继续使用隔离临时 ecosystem/artifact root，不得接触stable。

当前其它production blocker：

- **存在**：H1 Host authenticated HTTP producer尚未实现；这是本预检返回
  `TASK_SCOPE_EXPANDED`的精确范围。
- client Host read HTTP consumer和Host private JSON-RPC responder已经完成；未发现第四个Host peer
  method、shared-client修改或新的业务语义。
- H1 + S1完成后，在任务允许的producer/consumer范围内没有看到其它production owner；最终结论仍必须
  由两条Gate C command实际通过证明，不能用本只读审计代替。

## 8. 本次操作边界

只运行了 `git show/status/diff/log`、`rg/find/awk/sed/cat`等只读检查。没有运行build、type-check、
test workload、stable、live、network；没有修改Internals、skiff-packages或production/test/fixture；
没有merge、rebase、push，也没有派子Agent。本节点只新增本文。
