# P5-F438B Agine / Host WebSocket request owner audit result

状态：`PASS / TASK_EXECUTABLE`。这是只读 consumer 闭合结果，不是 implementation。

## 1. 输入、边界与结论

审计输入：

| Repo | Commit | Tree | 审计状态 |
| --- | --- | --- | --- |
| Skiff design input | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` | 权威语义已由任务父节点冻结 |
| Internals | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` | production/test 只读 |

读取了任务指定的五份权威文档、直接父节点 `P5-F438`、两个 worktree 的适用
`AGENTS.md`，并对 `agine/service/**`、`agine/host/**`、`agine/client/**`、
`shared-client/**` 及直接相关 `agine/protocol/**` 做了 producer/consumer 反搜。没有运行 build、
type-check、浏览器、stable/live 或 canonical workflow。

冻结结论：

1. Host 是 external peer。Host 主动发起的 hello、activation ack、presence、attempt sync、tool result
   全部是业务上行，终态必须使用 authenticated HTTP；它们不能成为 client-initiated platform request。
2. `host.files.list`、`host.files.search`、`host.current-directory` 是 Skiff 向一个精确 Host
   connection 发起并等待结果的有限操作，终态使用
   `std.websocket.requestJsonToConnection`。它们不需要业务 `requestId`、DB relay、polling 或 `jobId`。
3. Host tool execution 是真正跨 request 的 durable business lifecycle。保留 `toolCallId`、
   `attemptId`、`runId`、deadline、Host ledger 和 service settlement journal；不能把它误改成一个最长
   595 秒的平台 transport request。Host poll/sync 与 result/receipt 改为 HTTP。
4. 浏览器普通 chat/provider/agent/toolprovider 请求已大部迁到 HTTP；旧 WebSocket receive 分支与其
   `eventName`/`requestId` DTO 是 compatibility graph，应删除。缺失的 thread mount add/remove 与 Host
   file browser HTTP route应补齐，而不是保留 raw receive。
5. chat stream、run/provider/transcript event 和 browser `tool_call/request` 是单向 server
   notification，继续使用 `std.websocket.send*`。它们的 `runId`、`messageSeq`、`toolCallId` 是业务
   identity，不是 transport correlation。
6. `service.yml.websocket` 终态只保留 `/ws` path + connect handler。`api.yml` 不再导出统一
   `websocket` callable；不存在 `receive`、message operation 或 request route。
7. 没有需要用户裁决的未知同步/持久生命周期；本任务不是 `TASK_NOT_EXECUTABLE`。

## 2. 当前 owner 图

### 2.1 统一 receive 与 correlation

- `agine/service/internal/agine_service.skiff:websocket` 接收 legacy
  `WebSocketIngressEvent`，把 `receive` 分支交给
  `agine_ws_dispatch.receive`。
- `agine/service/internal/agine_ws_dispatch.skiff:receive` 是唯一 raw dispatcher，先调用
  `agine_transport.decodeWebSocketRequest`，再依次调用 `agine_ws_chat.dispatch`、
  `agine_ws_host_tool_files.dispatch`、`agine_ws_agent_provider.dispatch`。
- `agine/service/internal/agine_transport.skiff:decodeWebSocketRequest`,
  `successEnvelope`, `errorEnvelope`, `sendResponse`, `sendError` 拥有旧
  `eventName + requestId + ok/error/payload` correlation。
- `agine/service/api/agine.skiff:ClientMessage` 汇总所有 legacy receive input；`ServerEnvelope`
  是旧 response DTO。
- `agine/client/src/lib/ws.ts:request` 委托共享
  `EnhancedWebSocket.request`；`agine/client/src/lib/hostFileApi.ts:listHostFiles/searchHostFiles`
  是当前唯一 Agine browser production caller。
- `shared-client/shared/lib/enhanced_websocket.ts:EnhancedWebSocket.request` 生成 request id 并按
  `(eventName-response, requestId)` 关联。它还被非 Agine、非 Skiff 的
  `/client/<PROJECT_NAME>` gateway consumers 使用。
- `agine/host/src/shared/enhanced_websocket.ts:EnhancedWebSocket.request` 没有 production caller，
  且只按 response event name 关联；它是 Host 内完全 dead helper。

### 2.2 Host identity 与 exact connection

- `agine/service/internal/agine_connect.skiff:acceptConnection` 从 upgrade headers 验证 session、
  Host id 或 activation token；`hostAccept` 返回 `businessIdentity` 和
  `maxConnections=1/close-oldest`。
- `agine/service/internal/host_toolprovider_connection.skiff:activateHostConnection` /
  `authenticateHostConnection` 和
  `agine/service/internal/host_toolprovider_registry.skiff:upsertActivatedHostToolProvider` /
  `refreshActiveHostToolProviderPresence` 持久化 `actorSubjectId`、`activeConnectionId`、
  `lastSeenAt` 与 Host capability metadata。
- `ToolProvider.activeConnectionId` 是平台 RPC 所需的精确 connection 来源。调用前仍须验证 provider
  owner、active status、presence TTL、capability 与 thread mount；transport broker再验证该
  connection确实属于当前 service/deployment/generation。

### 2.3 Host file relay

当前完整链：

```text
agine/client hostFileApi.request
  -> agine_ws_host_tool_files.dispatchUserFileRequest
  -> host_toolprovider_runtime.dispatchHostFileBrowseRequest
  -> host_file_rpc.dispatchHostFileBrowseRequest
  -> DB HostFileBrowseRequest + sendJsonToConnection(Host)
  -> HostRuntime.attachHostFileHandlers
  -> Host eventName result
  -> agine_ws_host_tool_files.dispatchHostFileResult
  -> host_file_rpc.receiveHostFileBrowseResult
  -> delete DB relay + sendJsonToConnection(browser)
  -> EnhancedWebSocket.request resolves
```

`agine/service/internal/model.skiff:HostFileBrowseRequest` 只保存两层 transport correlation：
browser connection/request id 与 Host connection/relay id。它不是业务 job、run 或 durable resource。

### 2.4 Host tool durable lifecycle

- `agine/service/internal/host_tool_delivery.skiff:createOrReadAttempt` 创建 durable
  `HostToolAttempt`；`deterministicAttemptId` 由 `toolCallId + mountId` 形成业务 attempt identity。
- `agine/service/internal/host_tool_action_issue.skiff:requestEvent` 形成
  `tool_call/request`，`issueExecuteActionCas` 持久化 delivery CAS。
- `agine/service/internal/host_tool_reconciliation.skiff:syncHostToolAttempts` 返回
  execute/keep/resend/cancel/forget actions；当前由 Host 主动发送
  `host/tool-attempts` 后获得 response。
- `agine/host/src/HostToolAttemptRuntime.ts` 与 `ToolAttemptLedger.ts` 跨断线保留 executing/result_ready/lost
  state、幂等执行、结果重发与 tombstone；`attemptId` 是这里不可删除的业务 identity。
- `agine/host/src/protocol/toolCall.ts:buildToolCallResultEvent` 形成 Host result；
  `agine_ws_host_tool_files.dispatchToolResult` 当前接收并返回 receipt。

### 2.5 Producer / consumer evidence index

| Surface | Producer evidence | Consumer / terminal owner evidence |
| --- | --- | --- |
| browser Host file RPC | `agine/client/src/lib/hostFileApi.ts:listHostFiles`, `searchHostFiles` | `agine_ws_host_tool_files.dispatchUserFileRequest` -> `host_file_rpc.dispatchHostFileBrowseRequest` |
| browser ordinary chat | `agine/client/src/stores/appStore/chatActions.ts`, `chatHistory.ts`, `messageActions.ts` (`chatHttpRequest` / `ordinaryUserHttpPost`) | `agine_http_chat.dispatch` |
| agent/provider catalog and mutation | `agine/client/src/stores/appStore/configActions.ts` | `agine_http_agent_provider.dispatch` |
| toolprovider/current directory | `agine/client/src/lib/toolproviderApi.ts`, `threadHostBindings.ts` | `agine_http_tool_providers.dispatch`; cache miss continues at `host_toolprovider_connection.requestHostCurrentDirectoryRefresh` |
| browser client tool result | `agine/client/src/stores/appStore/messageActions.ts:respondToAskUser` | `agine_http_user_tools.toolCallResult` |
| Host hello/presence/sync | `agine/host/src/GatewayClient.ts:sendHello`, `sendPresence`, `sendToolAttemptState` | `agine_ws_host_tool_files.dispatchHostLifecycle` |
| Host activation ack | `GatewayClient.sendActivationAck` | `HostCoordinator.activationAck` -> `host_toolprovider_connection.acknowledgeHostActivation` |
| Host current directory response | `agine/host/src/HostRuntime.ts:hostCurrentDirectoryResponseEvent` | `HostCoordinator.currentDirectory` |
| Host file handlers | `HostRuntime.attachHostFileHandlers` | `agine_ws_host_tool_files.dispatchHostFileResult` -> `host_file_rpc.receiveHostFileBrowseResult` |
| Host tool result | `HostToolAttemptRuntime.sendDurableResult` -> `buildToolCallResultEvent` | `agine_ws_host_tool_files.dispatchToolResult` -> `HostCoordinator.onToolResult` -> `host_tool_reconciliation.onHostToolResult` |
| Host execute/cancel reconciliation | `host_tool_action_issue.executeAction`, `host_tool_reconciliation.snapshotAction` | `HostRuntime.attachToolHandlers` -> `HostToolAttemptRuntime.handleAction` |
| browser tool notification | service/Agent tool projection -> `chat_stream` / WebSocket send | `agine/client/src/stores/appStore/wsToolHandlers.ts:registerToolHandlers` |
| chat stream notifications | `agent_bridge_event_projection.skiff`, `conversation_title_tool.skiff`, `tool_result_adapter.skiff`, `host_provider.skiff` | `registerProviderHandlers`, `registerRunHandlers`, `registerTranscriptHandlers`, `registerToolHandlers` |

## 3. 完整分类矩阵

分类：

- **HTTP-UP**：external peer 主动业务 request，迁到 HTTP。
- **PLATFORM-RPC**：Skiff 向精确 Host connection 发起并等待，迁到
  `requestJsonToConnection`。
- **NOTIFY**：Skiff 单向下行，继续 `send*`。
- **DURABLE-ID**：保留明确业务 identity，但移除 transport `requestId`。
- **DELETE**：完全 dead legacy graph。

### 3.1 Browser / ordinary request receive

| 旧 event / owner | 当前 consumer | 终态 | 精确处理 |
| --- | --- | --- | --- |
| `chat/list`, `chat/create`, `chat/get`, `chat/send`; `api/agine.skiff` input DTO | 已无 `agine_ws_*` dispatch；browser 已走 `chatHttpRequest` | HTTP-UP + DELETE | 保留对应 HTTP payload/handler；删除只服务旧 envelope 的 Input/eventName/requestId。`ChatCreateInput` 的内部/test caller改用已有 `ChatCreateCommand`，不能因本地复用而保留 transport DTO。 |
| `chat/update`, `chat/update_model`, `chat/pin`, `chat/delete`, `chat/stop`, `chat/regenerate`, `chat/usage`, `chat/move-tool-to-background`; `agine_ws_chat.dispatch` | browser 已走 `ordinaryUserHttpPost` | HTTP-UP + DELETE | HTTP route/command owner已存在；删除全部 WS cases、matching envelope tests与旧 Input DTO。`runId`/`toolCallId`按业务语义保留。 |
| `agents/list`, `agents/hidden-list`, `agents/create`, `agents/update`, `agents/delete`, `agents/reset`, `agents/unhide`; `agine_ws_agent_provider.dispatch` | browser 已走 HTTP | HTTP-UP + DELETE | 保留 `agine_http_agent_provider`; 删除 WS cases/Input DTO。 |
| `provider/list`; `agine_ws_agent_provider.dispatch` | browser 已走 HTTP | HTTP-UP + DELETE | 保留 `ProviderListPayload` 与 HTTP owner；删除 `ProviderListInput`。 |
| `tools/list`; `agine_ws_agent_provider.dispatch` | 无 Agine browser production request caller | DELETE | 无 HTTP successor consumer；删除 case、`ToolsListInput`及 envelope tests。若未来需要 catalog，单独声明 HTTP route，不能复活 receive。 |
| `/toolproviders/list`, `/remove`, `/rename`, `/current-directory`; `agine_ws_tool_providers.dispatch` | browser 已走 HTTP | HTTP-UP + DELETE | 保留现有 HTTP handlers；current-directory 的内部 Host refresh改为 PLATFORM-RPC，见 §4。 |
| `/thread/toolproviders/list`; `agine_ws_tool_providers.dispatch` | browser 已走 HTTP | HTTP-UP + DELETE | 保留现有 HTTP handler。 |
| `/thread/toolproviders/add`, `/thread/toolproviders/remove`; `agine_ws_tool_providers.dispatch` | current client没有 production WS caller，但 service command仍有意义 | HTTP-UP + DELETE | 若 UI/业务仍需 mutation，在同一 consumer leaf增加 `/thread/toolproviders/add|remove` HTTP paths/types；删除旧 cases/Input DTO。没有 consumer时可连 command adapter一起判 dead，但不得保留 WS fallback。 |
| `/thread/host-files/list`, `/thread/host-files/search`; `agine_ws_host_tool_files.dispatchUserFileRequest` | `hostFileApi.ts` | HTTP-UP at browser boundary + PLATFORM-RPC to Host | 新增同名 POST HTTP route；一个 HTTP request内验证并等待 Host。删除 browser WS requestId与receive cases。 |
| application `ping` / `pong`; `agine_ws_dispatch.receive` | Agine browser heartbeat已被 `AgentSocket.startHeartbeat` 禁用 | DELETE | WebSocket protocol ping/pong由栈拥有；删除 JSON ping/pong case。 |
| invalid/unsupported message response | raw receive fallback | DELETE | 终态 unsolicited data由 Router以1003关闭，不发业务 envelope。 |
| `server_error` / `originalMessage.requestId`; Agine GlobalErrorHandler | Agine service不生产该 shape | DELETE in Agine | 删除 `agine/client/src/components/GlobalErrorHandler.tsx` 中 correlation listener或改为HTTP/global error owner；不影响非 Agine shared gateway。 |

### 3.2 Host activation, presence and reconciliation upcalls

| 旧 event / producer -> owner | 终态 | Authorization / lifecycle |
| --- | --- | --- |
| `host/hello`; `GatewayClient.sendHello` -> `dispatchHostLifecycle` -> `HostCoordinator.helloWithRuntime` | HTTP-UP `POST /host/hello` | Host id或activation header；必须与connect时持久化的 active Host/provider一致。返回首次activation的temporary Host id、provider/name/status；携带cwd + capabilities。 |
| `host/activation-ack`; `sendActivationAck` -> `HostCoordinator.activationAck` | HTTP-UP `POST /host/activation-ack` | Host id credential；幂等消费activation token，保留token状态机，不依赖WS request id。 |
| `host/ping`; `sendPresence` -> `pingWithRuntime` | HTTP-UP `POST /host/ping` | Host id credential；刷新lastSeen/currentDirectory/capabilities。HTTP response天然关联。 |
| reconnect hello | 当前 socket-open monitor再次发送 `host/hello` | HTTP-UP | WebSocket connect先更新 exact activeConnectionId/generation；随后HTTP hello刷新业务presence。旧generation上的pending PLATFORM-RPC失败，不迁移、不自动retry。 |
| `host/tool-attempts`; `sendToolAttemptState` -> `syncToolAttempts` | HTTP-UP `POST /host/tool-attempts` + DURABLE-ID | request为snapshot数组，response为actions；保留attempt/tool IDs、CAS、Host ledger。删除`toolAttemptSyncOutstanding`的WS correlation，HTTP in-flight promise就是并发owner。 |
| `tool_call/result` with `executor=host`; Host runtime -> `dispatchToolResult` | HTTP-UP `POST /host/tool-call/result` + DURABLE-ID | Host credential，校验provider/attempt/tool/thread exact match；response直接是receipt。不能复用现有 user `/tool_call/result`，后者明确拒绝Host credential/executor。 |
| `tool_call/receipt`; service response -> Host | HTTP response | 不再是notification；receipt中的attempt/tool identities保留，transport eventName删除。 |
| execute/keep/resend/cancel/forget actions | 当前 `host/tool-attempts-response` payload | HTTP response + DURABLE-ID | 继续由sync response携带。`cancel`先持久化Host ledger tombstone再abort active controller；不是platform cancel envelope。 |

Host HTTP route的authentication不能使用browser session。后继需从
`agine_connect.hostConnectHeaderAuth` 抽出共享的严格 header parser，再由HTTP handler查找
HostActivationToken/ToolProvider。请求不得接收 caller-supplied owner、actor subject、business identity或
connection id；exact connection只从DB中当前 `ToolProvider.activeConnectionId` 读取。

### 3.3 Host outbound request / response

| 当前 flow | 终态 | 业务ID / transport state |
| --- | --- | --- |
| `host/current-directory/request` notification -> Host `host/current-directory` upcall -> browser HTTP polling | PLATFORM-RPC `host.current-directory` | 无业务ID；删除随机 `host_cwd_*` requestId与`refreshRequested` polling。可更新ToolProvider metadata cache，但response在原HTTP request内返回。 |
| `host/files/list-request` -> `host/files/list-result` | PLATFORM-RPC `host.files.list` | 无job/request business ID；删除HostFileBrowseRequest relay。 |
| `host/files/search-request` -> `host/files/search-result` | PLATFORM-RPC `host.files.search` | 无job/request business ID；删除HostFileBrowseRequest relay。 |

这些操作是有限、无独立durable状态的读取。代码已给出15秒Host timeout，结果只被当前browser caller消费；
没有poll/status/result resource。因此旧实现的异步形态不能证明需要`jobId`。

### 3.4 Tool delivery and server notifications

| Producer / payload | 终态 | 原因 |
| --- | --- | --- |
| browser `tool_call/request` from chat/tool lifecycle | NOTIFY + DURABLE-ID | ask-user/client tool请求不等待socket response；结果已走user HTTP `/tool_call/result`。保留`toolCallId`和`runId`。 |
| Host execute action in attempt sync response | HTTP response + DURABLE-ID | Host执行可跨请求、断线和进程重启，保留attempt journal。不能改为platform pending request。 |
| chat `message-added`, `step-start/finish`, text/reasoning delta, run completed/stopped/failed, title/provider/tool events | NOTIFY | `chat_stream` -> `send*`；run/message/tool identities用于乱序与业务状态，不是transport request id。 |
| `ChatStreamConnection` DB table + `rememberConnection` | DELETE | Gateway已按businessIdentity fan-out；connect handler已返回session businessIdentity。改用`sendTextToBusinessIdentity`后无需DB保存browser socket。 |
| low-level exact connection send for Host platform RPC | PLATFORM-RPC | 调用点使用`activeConnectionId`；不使用business-identity fan-out，因为unary response必须属于一个socket。 |

## 4. 平台 Host peer RPC 冻结

### 4.1 Canonical method 与 concrete type

method常量与Host TypeScript request/response接口的唯一 owner应新增在
`agine/protocol/hostPeer.ts`。Skiff caller concrete types放在
`agine/service/api/agine.skiff` 或新的私有 `internal/host_peer_protocol.skiff`；它们不是
ServiceContract或gateway entry schema。跨语言 fixture test逐字段校验同一JSON shape。

| Method | Request | Success response | Skiff caller / Host handler |
| --- | --- | --- | --- |
| `host.files.list` | `{ path?: string }` | `HostBrowseDirectoryResult { root,cwd,parent,breadcrumbs,items,truncated }` | `host_file_rpc` replacement / `HostService.listDirectory` |
| `host.files.search` | `{ path?: string, query: string }` | `HostBrowseSearchResult { root,cwd,matches,truncated }` | `host_file_rpc` replacement / `HostService.searchBrowseFiles` |
| `host.current-directory` | `{}` | `{ currentDirectory: string }` | `host_toolprovider_connection.currentDirectoryForToolProvider` / `HostService.getCurrentDirectory` |

`toolProviderId`、owner、browser connection与Host connection不进入peer payload。Service在调用前从session、
thread mount和ToolProvider记录完成授权，并只向DB中解析出的 exact `activeConnectionId`发起请求。

### 4.2 Error projection

Host adapter只能返回固定platform error：

```text
{ code: string, message: string, detail?: Json }
```

- known workspace errors保留稳定code：
  `HOST_FILES_INVALID_PATH`、`HOST_FILES_PATH_OUTSIDE_ROOT`、
  `HOST_FILES_TIMEOUT`、`HOST_FILES_FAILED`；
- unknown method -> `HOST_PEER_UNKNOWN_METHOD`；
- malformed payload -> `HOST_PEER_INVALID_PAYLOAD`；
- handler throw/unknown local error -> `HOST_PEER_HANDLER_FAILED`，message脱敏；
- caller cancellation不发送业务error response；Host abort handler并让platform cancel完成竞态。

Skiff caller分别处理：

- peer `ok:false` / connection disconnect / broker protocol failure：
  `std.websocket.WebSocketRequestError`，投影为现有公开`ApiError`（offline为`host_offline`，
  file timeout为`host_files_timeout`，known peer file code规范化后透传）；
- request encode或typed success decode：
  `std.json.DecodeError`，投影为`host_files_protocol`，不能伪装成peer/transport error；
- caller/browser cancel保持cancel，不产生成功或可重试业务result；
- HTTP层使用现有fixed `{error:{code,message}}`，不返回WebSocket envelope。

### 4.3 Host adapter state machine

Host WebSocket production parser终态只接受：

```text
{type:"request", requestId, method, payload}
{type:"cancel", requestId}
```

并只发送：

```text
{type:"response", requestId, ok:true, payload}
{type:"response", requestId, ok:false, error:{code,message,detail?}}
```

要求：

1. request id非空且原样回显；Host不得生成、持久化或解释该transport id。
2. 固定method table dispatch，先做outer shape和method-specific payload validation。
3. 每个request最多settle一次；不同request并行，response可乱序。
4. adapter维护requestId到`AbortController`的有界in-flight map。cancel先移除/标记，再abort；response先赢时
   后到cancel忽略；cancel先赢时handler的晚到值/error不得发送。
5. duplicate active request id、unknown cancel、malformed outer envelope均fail closed；不能触发任意
   eventName listener。
6. handler throw映射固定error，不把stack/path/credential写入wire。
7. Host WebSocket不得发送`type:"request"`或任意application `eventName` frame。hello/ping/tool sync/result
   都从HTTP client发送。

`agine/host/src/shared/enhanced_websocket.ts:request` 应删除；新的peer responder是接收
platform request的adapter，不是另一个client-generated correlation helper。

## 5. Host file browser可在一个HTTP request内等待

结论：可以，且必须这样实现。

目标链：

```text
browser fetch POST /thread/host-files/{list|search}
  -> HTTP session + thread/mount/provider/capability authorization
  -> read ToolProvider.activeConnectionId
  -> timeout(15s) {
       requestJsonToConnection<HostFileRequest, HostFileResponse>(
         exactConnection, method, payload
       )
     }
  -> ordinary HTTP response
```

owner矩阵：

| Concern | Owner | Required semantics |
| --- | --- | --- |
| platform HTTP maximum / deployment override | Router + Agine deployment | 当前deployment 120s只是外层上限；Host file operation必须再收紧到15s。 |
| Host file operation deadline | Agine Skiff HTTP handler | `timeout(15000)`包住platform request；不得启动detached work。 |
| Host local I/O timeout | Host peer handler | 同一request的AbortController；15s/收到platform cancel时abort FileWorkspace/Ripgrep。 |
| browser explicit supersession | `useHostFileBrowser` / `hostFileApi` | 使用AbortController；新list/search、关闭pane或effect cleanup主动abort旧fetch，而不只用sequence忽略结果。 |
| browser socket/tab disconnect | browser fetch + Router HTTP gateway | HTTP disconnect传播runtime cancel，再传播platform cancel。 |
| Host disconnect/reconnect | broker + Skiff caller | old connection/generation的request立即失败；不转移到新socket、不自动retry。下一次browser request重新解析activeConnectionId。 |
| late Host response | Router broker | 只命中bounded settled tombstone并丢弃。 |

可删除：

- `model.skiff:HostFileBrowseRequest` DB object及所有index；
- `host_file_rpc:HostFileBrowseResultClaim`,
  `hostFileBrowseTtlMs`, `hostFileBrowseExpiresAt`,
  `cleanupExpiredHostFileBrowseRequests`, DB insert/claim/delete与browser relay send；
- API `HostFilesResultInput`、`ThreadHostFiles*Input.requestId`；
- Host `attachHostFileHandlers`中的eventName/requestId result send，改成method handlers；
- browser `EnhancedWebSocket.request` path与cookie websocket RPC tests；
- `refreshRequested` + fixed retry delay current-directory polling。

## 6. EnhancedWebSocket 与其他 correlation owner

| Owner | 当前合法 consumer | 结论 |
| --- | --- | --- |
| `agine/host/src/shared/enhanced_websocket.ts:EnhancedWebSocket.request` | 无 | DELETE；Host adapter取代它。 |
| `agine/client/src/lib/ws.ts:request` | 仅Host file browser | DELETE after file HTTP migration。 |
| `agine/client/e2e/support/cookie-websocket-rpc.mjs` | Agine chat smoke legacy WS RPC | DELETE；smoke改用cookie HTTP helper + notification-only WS observer。 |
| `agine/client/e2e/support/machineHarness.ts:browserWsRequest` | no direct production owner; legacy system helper | DELETE or migrate caller toHTTP；不得作为connect-only例外。 |
| `shared-client/shared/lib/enhanced_websocket.ts:EnhancedWebSocket.request` | 多个非Agine products通过`shared/base/socket.ts`连接legacy `/client/<PROJECT_NAME>`，包括upload、notification、chatty和admin APIs | RETAIN outside Agine scope；这是合法的非-Skiff WebSocket RPC consumer。不能因Agine cutover全局删除。 |
| `shared-client/shared/base/socket.ts:ClientSocket` | non-Agine shared apps | RETAIN。Agine的`AgentSocket`只继承connection/event能力，不得再调用或re-export request。 |
| service `agine_transport` requestId envelope | 仅legacy receive/response | DELETE。 |
| platform Host peer request id | Router + Host peer adapter outer envelope | RETAIN as transport-only opaque id；不得进入business DTO、DB或logs payload。 |

## 7. Connect-only Agine收敛 owner

### 7.1 精确 production owner

| Owner | 终态 |
| --- | --- |
| `agine/service/service.yml` | 所有HTTP entries显式保留/新增；`websocket`改为单一`path: /ws` + `connect: internal.agine_connect.acceptConnection`目标语义，删除`routes[].operation`. |
| `agine/service/api.yml` | 删除`websocket: internal.agine_service.websocket`；保留HTTP handler export（直到相应authoring允许直接external selector）与确有package consumer的types。 |
| `agine/service/internal/agine_service.skiff` | 删除`websocket`统一connect/receive facade；只保留HTTP facade，或由service.yml直接引用具体connect。 |
| `agine/service/internal/agine_connect.skiff` | 保留connect authentication、businessIdentity、connection policy；connect只完成admission/active connection persistence，不处理business messages。 |
| `host_toolprovider_connection.skiff`, `host_toolprovider_registry.skiff`, `model.skiff:ToolProvider` | 保留并强化activeConnectionId/actorSubjectId/generation-facing update；HTTP Host auth从credential/token解析owner，PLATFORM-RPC caller从这里读exact connection。 |
| `agine_ws_dispatch.skiff`, `agine_ws_chat.skiff`, `agine_ws_agent_provider.skiff`, `agine_ws_tool_providers.skiff`, `agine_ws_access.skiff` | 全部DELETE。 |
| `agine_ws_host_tool_files.skiff` | Host上行迁HTTP、file/current-dir迁platform RPC后全部DELETE。 |
| `agine_transport.skiff` | 删除WebSocket request decoder和success/error send envelope；保留HTTP response helper。 |
| `api/agine.skiff` | 删除ClientMessage/ServerEnvelope及所有只为receive存在的Input/requestId；保留HTTP payload/command、chat notification、tool durable与Host business types。 |
| `model.skiff:ChatStreamConnection`, `chat_stream.rememberConnection` | DELETE；notification按businessIdentity发送。 |
| `model.skiff:HostFileBrowseRequest`, `host_file_rpc` relay | DELETE并改为直接platform RPC wrapper。 |
| `agine/protocol/http.ts` | 增加Host authenticated HTTP paths与browser Host file/mount paths；继续禁止correlation字段。 |
| `agine/protocol/toolCall.ts` | 保留attempt/tool business contracts，移除eventName-only HTTP wrappers；Host sync/result改HTTP request/response types。 |
| `agine/protocol/hostPeer.ts` (new) | 唯一Host platform method常量、TS request/response/error类型。 |
| `agine/host/src/GatewayClient.ts` | 只管理WebSocket connect/reconnect和platform responder；移除sendHello/presence/sync raw application frames。 |
| `agine/host/src/HostRuntime.ts` | Host业务上行使用HTTP client；安装peer method handlers；保留presence/poll timers但timer body是HTTP。 |
| `agine/host/src/shared/enhanced_websocket.ts` | 删除request、eventName dispatcher和application message queue；增加固定platform request/cancel decoder或由新专用adapter拥有。 |
| `agine/client/src/lib/hostFileApi.ts` | 改HTTP并接受AbortSignal。 |
| `agine/client/src/lib/ws.ts` | 删除request export，只保留notification observer。 |
| `shared-client/**` | 不改generic legacy RPC；Agine不得再依赖其request surface。 |

`config.dev.yml` 的deployment timeout仍可作为120s HTTP外层上限；Host file 15s必须由operation代码收紧。
不新增WebSocket receive/message config。Host CLI的gateway URL继续同时派生HTTP与WebSocket scheme；
Host credential只放header，不能回到query。

### 7.2 测试 owner

- Service删除/替换：
  `agine_service_dispatch.test.skiff`,
  `agine_service.delete_contract.test.skiff`,
  `host_file_browse.test.skiff`,
  `host_toolprovider_current_directory.test.skiff`,
  `agine_service_architecture.test.mjs`,
  `host_runtime_architecture.test.mjs`,
  `service-api-receipt*.mjs` 中legacy receive/manifest断言。
- Service保留并重定向：
  Host settlement/reconciliation/lifecycle tests继续证明attempt ID、delivery CAS、late result、
  cancellation和Host restart；新增Host HTTP auth、result receipt、sync幂等及platform RPC error mapping。
- Host删除/替换：
  `GatewayClient.test.ts`, `HostRuntime.test.ts`, `cli.test.ts` 的eventName flows；
  新增fixed request/cancel envelope、unknown method、malformed payload、handler throw、cancel race、
  out-of-order、disconnect/reconnect generation probes。
  `HostToolAttemptRuntime.test.ts`、`ToolAttemptLedger.test.ts`继续保留durable semantics。
- Client：
  `hostFileApi.test.ts`, `HostFileBrowserPane.test.ts`, relevant hook tests改为HTTP/AbortSignal；
  `ws.test.ts`与`architecture.client-boundaries.test.ts`证明Agine无WS request export；
  `cookie-websocket-rpc*.mjs`删除；
  chat smoke改成HTTP caller + WebSocket notification observer。
- 跨语言：
  protocol fixture逐字段验证三种Host peer method；不能只测试TypeScript structural assignability。

### 7.3 checkpoint前后

Skiff platform checkpoint前可完成：

1. protocol HTTP/Host peer type checkpoint；
2. Host hello/ack/ping/tool-attempt sync/tool result的authenticated HTTP routes和Host caller迁移；
3. 已有browser ordinary HTTP paths清理及missing thread mount HTTP paths；
4. ChatStreamConnection改businessIdentity send；
5. Host/client dead `EnhancedWebSocket.request` consumer清理（Host peer responder可先以fixture测试）。

必须等待Skiff `requestJsonToConnection` combined checkpoint：

1. file list/search/current-directory direct platform RPC；
2. Host fixed request/cancel responder与真实broker integration；
3. 删除HostFileBrowseRequest relay和Host file result receive；
4. 删除最后raw receive branch并把service.yml/api.yml改为connect-only；
5. Internals combined protocol/runtime/router risk probe。

## 8. 互斥后继 DAG 与精确写集

```text
P5-F438A shared Skiff checkpoint + combined Skiff probe
                     |
Internals protocol checkpoint
  agine/protocol/http.ts
  agine/protocol/toolCall.ts
  agine/protocol/hostPeer.ts (new)
  agine/protocol/package.json
                     |
        +------------+-------------+
        |                          |
Host peer/HTTP leaf          Service connect-only leaf
agine/host/** only           agine/service/** only
        |                          |
        +------------+-------------+
                     |
Agine browser leaf
agine/client/** only
                     |
Internals focused combined (read-only gate)
```

后继leaf写集严格互斥：

1. **Protocol checkpoint**：只写 `agine/protocol/**`。
2. **Host peer/HTTP leaf**：只写 `agine/host/**`；消费protocol checkpoint，不能改shared-client。
3. **Service connect-only leaf**：只写 `agine/service/**`；同时消费protocol与Skiff checkpoint，拥有全部
   manifest/API/DB relay/receive删除，避免两个leaf共同改`service.yml`或`api/agine.skiff`。
4. **Browser leaf**：只写 `agine/client/**`；迁Host file HTTP、AbortSignal、smoke/helper与WS request
   surface。`shared-client/**`明确不在写集。
5. **Combined owner**：无production写集；只在上述三个Internals leaf合流后运行聚焦验证。

若implementation中发现必须修改`shared-client` generic request，先返回scope expansion；本审计已证明
non-Agine consumers仍合法，不能把它列入Agine leaf。

## 9. 聚焦验证与 combined 风险探针

便宜leaf验证：

- protocol：TypeScript compile-level fixture + exact JSON fixture；HTTP path registry/API/service manifest
  三方一致性静态检查。
- Host：
  `GatewayClient.test.ts`, `HostRuntime.test.ts`, `HostToolAttemptRuntime.test.ts`,
  `ToolAttemptLedger.test.ts`, `hostDeadline.test.ts`, `HostService.test.ts`,
  `host-architecture.test.mjs` 的聚焦子集。
- Service：
  source architecture checks；Host HTTP auth/sync/result tests；file list/search/current-directory typed
  peer fake；Host attempt settlement/lifecycle suites。
- Client：
  `hostFileApi.test.ts`, file browser/component tests, `ws.test.ts`,
  `architecture.client-boundaries.test.ts`, HTTP mock smoke。

combined风险探针必须一次覆盖：

1. 同一Host connection上并发list/search，response逆序返回且各自回到正确browser HTTP response。
2. wrong connection/generation/id、malformed/unknown response不能恢复调用。
3. browser abort、HTTP disconnect、15s deadline、Host disconnect、runtime cancel均清pending并发送
   best-effort platform cancel；late response只命中tombstone。
4. Host responder unknown method、malformed payload、handler throw、cancel-before/after-settle。
5. Host reconnect更新activeConnectionId；old in-flight失败，new browser request只打到new generation。
6. Host tool执行跨disconnect/restart仍由attempt ledger去重；result HTTP receipt、resend、cancel/forget
   不因平台transport变化丢失。
7. Browser普通业务没有client-initiated data frame；chat/tool notification仍可下发。
8. service artifact只有connect，任意unsolicited browser/Host eventName data frame按平台1003关闭。

本任务本身不运行这些implementation gates。

## 10. 反向搜索 allowlist

终态必须执行：

```bash
rg -n 'WebSocketIngressEvent|receiveEvent|function receive\\(' agine/service
rg -n 'HostFileBrowseRequest|host/files/(list|search)-(request|result)' agine
rg -n 'eventName.*requestId|requestId.*eventName|originalMessage\\.requestId' \
  agine/service agine/host agine/client
rg -n 'EnhancedWebSocket\\.request|socket\\.request|export async function request' \
  agine/host agine/client
rg -n 'host/(hello|activation-ack|ping|tool-attempts)|tool_call/result' \
  agine/host/src agine/service/internal
rg -n 'ChatStreamConnection|rememberConnection' agine/service
```

允许项：

- `agine/protocol/hostPeer.ts` 与Host responder中固定platform outer envelope的opaque `requestId`；
- `toolCallId`、`attemptId`、`runId`、`messageSeq`、resource ids；
- chat stream与`tool_call/request` notification的`eventName`；
- HTTP path registry/types中的业务operation名字（无requestId）；
- `shared-client/shared/lib/enhanced_websocket.ts` 及其非Agine legacy
  `/client/<PROJECT_NAME>` consumers；
- 测试fixture中专门断言legacy字段被拒绝或platform request id原样回显的匹配。

不允许项：

- `agine/service` production raw receive/message dispatcher；
- Agine browser/Host application `eventName + requestId` correlation；
- Host file DB pending/cleanup loop；
- Host主动发送platform request或任意application frame；
- HTTP payload/response/stream item中的requestId/correlationId；
- business-identity fan-out的平台unary request。

## 11. 未决问题

无blocking设计问题。

Implementation可自行选择文件拆分，但不能改变以下已冻结语义：三个Host peer method、Host上行HTTP、
15秒file operation deadline、exact active connection、tool attempt durable identity、connect-only
service authoring、shared-client非AgineRPC保留。若需要新增第四个Host peer method、改变tool attempt为同步
platform RPC、在HTTP schema重新引入correlation，必须先回到设计/用户决策，不能由后继leaf扩张。
