# P5-F427A Agine HTTP correlation owner audit result

状态：`PASS`（只读审计完成，repair 尚未执行）。没有发现需要新增业务 ID 或改变幂等语义的
事项，因此不是 `TASK_SCOPE_EXPANDED`。

## 1. 精确输入与边界

| 锚点 | commit | tree | 状态 |
| --- | --- | --- | --- |
| Internals integration 候选 | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | clean |
| F426C WIP，只读差分参考 | `62c3d6342ab81210e15c5ebb9e56cb17ae66a9f6` | `45e0c12486fd523773a6a0eaff1293044cb67182` | parent 正是精确候选 |
| Skiff result base | `d664bddf17adae74e15b74a2f03dc30102f1665b` | `08d95882714078c3a742280cde082871d15cb533` | clean |

候选的 `agine/service/service.yml` 声明 36 个 POST HTTP 路由：原 14 个与 F425D
新增 22 个。审计同时读取 HTTP protocol/service/client、legacy receive WebSocket、Host
file RPC、tests、mock 与 E2E helper，只为判断字段 owner。没有修改 Internals
production、test 或 fixture，也没有访问 stable/live。

本审计采用以下精确定义：

- HTTP correlation 字段是请求/响应中仅为把 unary HTTP request 与它自己的 response
  对回去的 `requestId`、`request_id`、`correlationId`、`correlation_id` 或同义字段；
- legacy WS `requestId` 是同一 socket 上多路 request/response 的匹配键，旧 receive 尚未删除时
  继续存在；
- `chatId`、`runId`、`toolCallId` 等是业务资源、执行或结算身份，不因为名字含 `Id` 而成为
  transport correlation。

## 2. 总结论

精确候选中：

- 28/36 个 HTTP request surface 接受 `requestId`：原 14 个中的 6 个，加上新增 22 个的全部；
- 34/36 个 HTTP response 使用旧 WS envelope；只有 package owner 直接处理的 `/session` 与
  `/track` 不经过该 envelope；
- scoped Agine HTTP production schema/adapter 中没有 `request_id`、`correlationId`、
  `correlation_id` 命中，唯一实际命中的 correlation 名称是 `requestId`；
- HTTP business owner 没有用 `requestId` 做查询、授权、幂等、去重、取消或状态推进。HTTP adapter
  读取它后只把它交给 `httpSuccess`/`httpError` 回显；
- browser 不用 HTTP response 的 `requestId` 或 `eventName` 做匹配。`fetch` Promise 已天然把
  response 归属到调用栈；当前 client 只读取旧 envelope 的 `ok`/`payload` 并做 flatten；
- 未提供 request ID 的 6 个普通 HTTP 路由仍返回 `requestId: null` 和 `eventName`，所以只删
  request payload 字段不足以完成 cutover；
- unknown route、unauthorized、caller-owned-field guard 和 decode error path 也调用
  `requestIdFromBody`，构成 route schema 之外的通用 correlation ingress/echo，必须一并删除。

`agine/service/api/agine.skiff` 还存在未被 `service.yml`、HTTP route resolver 或 HTTP adapter
引用的 `ThreadToolProvidersAddPayload`/`ThreadToolProvidersRemovePayload`，两者也带
`requestId?`。它们不是当前 HTTP wire surface；对应实际 consumer 是
`agine_ws_tool_providers` 的 legacy `*Input`。repair 应删除或重命名这两个误导性的 dead
`*Payload` 声明，但不能据此新增 HTTP route，也不能删除 WS `*Input.requestId`。

## 3. producer、consumer 与实际读取

### 3.1 当前 producer 分类

| 类别 | 当前 producer | 结论 |
| --- | --- | --- |
| H | `agine/client/src/lib/http.ts::chatHttpRequest` 为 `/chat/list`、`/chat/create`、`/chat/get`、`/chat/send` 自动生成 `nanoid()`，chat smoke/two-host/machine helpers 也显式生成 | 真正的 HTTP producer；必须删除 |
| M | `/chat/llm-call` 与 `/hosts/activation-token` 的 production `agineHttpPost` 不自动注入；service tests 或部分 E2E 可手工传入 | schema/adapter 仍接受并回显，仍必须删除 |
| W | 新增 22 项的当前 browser production 仍调用 legacy `socket.request`，其中 `requestId` 是 WS producer；对应 HTTP payload 与 HTTP tests 也接受该字段 | WS producer 保留在 WS DTO；不能让同一 DTO进入 HTTP |
| WIP | F426C WIP 新增 `ordinaryUserHttpPost`，会给 22 项 HTTP 自动注入 `nanoid()` | 与本 leaf 决定相反，不可原样复用 |
| R | `agine_transport.successEnvelope/errorEnvelope` 为 34 个普通 HTTP response 生产 `eventName`、`requestId`、`ok`、`payload` | HTTP 侧整组删除；WS sender 继续使用 |

所有 HTTP request 命中在 adapter 中都是“实际读取、但只为 envelope 回显”。传入
`thread_store`/`tool_result_adapter` 的旧 `*Input` 虽然带有 `requestId` 与 `eventName`，business
owner 只读取业务字段。HTTP response 的 `requestId`/`eventName` 没有 browser matching
consumer；真正依赖它们的是 legacy WS `ClientWebSocket.request`、CookieWebSocketRpc 及 Host
file 两跳 RPC。

### 3.2 原 14 个路由

下表的 `?` 表示现有业务 owner 定义的 optional/nullable 字段。成功 body 表示 correlation
repair 后的直接 HTTP body；所有失败统一见第 4 节。

| route | 当前 HTTP request correlation / producer | adapter consumer 与实际用途 | 删除后的精确 request body | 删除后的成功 body |
| --- | --- | --- | --- | --- |
| `/session` | 无 | package `httpSession` 直接 owner | package contract，不变 | package response/cookie，不变 |
| `/track` | 无 | package `track` 直接 owner | package contract，不变 | package 204 contract，不变 |
| `/chat/list` | `requestId?` / H | `requestIdFromBody`，只回显 | `{}` | `{chats}` |
| `/chat/create` | `requestId?` / H | decode 后塞入 legacy `ChatCreateInput`，owner 不读，只回显 | `{chatId?,title?,agentId?,reasoningLevel?,toolProviderId?,webSearchEnabled?}` | `{chat}` |
| `/chat/get` | `requestId?` / H | decode 后塞入 legacy `ChatGetInput`，owner 不读，只回显 | `{chatId,clientInstanceId?}` | `{chat,messages,runtime}` |
| `/chat/llm-call` | `requestId?` / M | decode 后只回显 | `{chatId,messageId,mode:"request"|"response"}` | `{payload:<selected model context>}` |
| `/chat/send` | `requestId?` / H | decode 后塞入 legacy `ChatSendInput`，business bridge 不读，只回显 | `{chatId,content,sessionContext?,generationConfig?,clientInstanceId?}` | `{accepted,chatId,messageSeq?,assistantMessageSeq?,runId?,userMessageId?,inboxSize,willStartProcessing}` |
| `/hosts/activation-token` | 未声明 typed payload，但 raw body 接受 `requestId` / M | `requestIdFromBody`，只回显 | `{}` | `{activationToken,expiresAt}` |
| `/provider/credential/save` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{providerId,apiKey}` | `{providerId,configured}` |
| `/provider/credential/delete` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{providerId}` | `{providerId,configured}` |
| `/provider/chatgpt-plan/oauth/start` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{}` | `{sessionId,status,verificationUrl?,userCode?,expiresAt?,error?}` |
| `/provider/chatgpt-plan/oauth/session` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{sessionId}` | `{sessionId,status,verificationUrl?,userCode?,expiresAt?,error?}` |
| `/provider/chatgpt-plan/oauth/cancel` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{sessionId}` | `{sessionId,status,verificationUrl?,userCode?,expiresAt?,error?}` |
| `/provider/chatgpt-plan/disconnect` | 无 request hit | response 仍由 R 产生 `requestId:null` | `{}` | `{configured,accountLabel?}` |

`/chat/llm-call` 成功 body 中的 `payload` 是该 endpoint 本身的业务字段，不是 transport
wrapper；应保留一次，不能被“删除 outer payload”规则误删。

### 3.3 F425D 新增 22 个路由

这 22 项当前都在 `agine/protocol/http.ts` 通过 `RequestIdPayload` 继承
`requestId?`，并在 `agine/service/api/agine.skiff` 的对应 `*Payload` 重复声明。HTTP adapter
均读取后只用于 R；当前 browser 的同名调用属于 W，而不是已完成的 HTTP producer。

| route | 当前 adapter consumer | 删除后的精确 request body | 删除后的成功 body |
| --- | --- | --- | --- |
| `/chat/update` | `ChatUpdatePayload -> ChatUpdateInput -> envelope` | `{chatId,title?,webSearchEnabled?}` | `{chat}` |
| `/chat/update_model` | `ChatUpdateModelPayload -> ChatUpdateModelInput -> envelope` | `{chatId,providerId,modelId,reasoningLevel?}` | `{chat}` |
| `/chat/pin` | `ChatPinPayload -> ChatPinInput -> envelope` | `{chatId,pinned}` | `{chat}` |
| `/chat/delete` | `ChatDeletePayload -> ChatDeleteInput -> envelope` | `{chatId}` | `{deleted,stoppedRunId?}` |
| `/chat/stop` | `ChatStopPayload -> envelope`；owner 当前不读 request `runId` | `{chatId,runId?}` | `{stopped,runId?}` |
| `/chat/regenerate` | `ChatRegeneratePayload -> envelope` | `{chatId}` | 无成功形态；当前只返回 `not_found`/`not_implemented` error |
| `/chat/usage` | `ChatUsagePayload -> envelope` | `{chatId}` | `{usage:{messageCount,totalTokens}}` |
| `/chat/move-tool-to-background` | `ChatMoveToolToBackgroundPayload -> envelope`；owner 读 `toolCallId`，当前不读 request `runId` | `{chatId,runId?,toolCallId}` | `{accepted,toolCallId,messageSeq,agentMessageId?,agentMessageSeq?,reasonCode?}` |
| `/agents/list` | `AgentListPayload -> envelope` | `{}` | `{agents,overrides}` |
| `/agents/hidden-list` | `AgentHiddenListPayload -> envelope` | `{}` | `{agents}` |
| `/agents/create` | `AgentCreatePayload -> envelope` | `{agent}` | `{success,agent}` |
| `/agents/update` | `AgentUpdatePayload -> envelope` | `{agent}` | `{success,agent}` |
| `/agents/delete` | `AgentDeletePayload -> envelope` | `{agentId}` | `{success}` |
| `/agents/reset` | `AgentResetPayload -> envelope` | `{agentId}` | `{success}` |
| `/agents/unhide` | `AgentUnhidePayload -> envelope` | `{agentId}` | `{success}` |
| `/provider/list` | `ProviderListPayload -> envelope` | `{}` | `{providers}` |
| `/toolproviders/list` | `ToolProvidersListPayload -> envelope` | `{}` | `{toolProviders}` |
| `/toolproviders/remove` | `ToolProvidersRemovePayload -> envelope` | `{toolProviderId}` | `{success}` |
| `/toolproviders/rename` | `ToolProvidersRenamePayload -> envelope` | `{toolProviderId,name}` | `{toolProvider}` |
| `/toolproviders/current-directory` | `ToolProvidersCurrentDirectoryPayload -> envelope` | `{toolProviderId}` | `{toolProviderId,currentDirectory,refreshRequested}` |
| `/thread/toolproviders/list` | `ThreadToolProvidersListPayload -> envelope` | `{chatId?,threadId?}` | `{toolProviders}` |
| `/tool_call/result` | `ClientToolCallResultPayload -> legacy ToolCallResultInput -> envelope` | `{toolCallId,chatId,attemptId?,runId?,messageSeq?,toolName,executor:"client",status,result?,error?}` | `{outcome,toolCallId,chatId?,threadId?,attemptId?,messageSeq?,pruneResultPayload,retryable,settlementKind?,resolvedAttemptId?,resolvedStatus?,resolvedError?,reasonCode?,continuationMessageSeq?}` |

## 4. HTTP response wrapper 删除闭集

当前 `successEnvelope` 固定产生：

```json
{
  "eventName": "<operation>-response",
  "requestId": "<echo-or-null>",
  "ok": true,
  "error": null,
  "payload": {"business": "result"},
  "business": "result copied again"
}
```

`errorEnvelope` 固定产生 `eventName:"error"`、`requestId`、`ok:false`、`error` 与
`payload:null`。`copyKnownPayloadFields` 又把已在 `payload` 中的若干业务字段复制到顶层。

HTTP repair 的删除闭集是：

1. HTTP request 的 `requestId`，以及 dispatcher 对 `requestId` 的 raw extraction/echo；
2. HTTP response 的 `eventName`、`requestId`、`ok`；
3. HTTP 上所有 `*-response` 与 `tool_call/receipt` event label；
4. transport outer `payload` wrapper 和 `copyKnownPayloadFields` 的重复投影；
5. client 的 `flattenLegacyEnvelope` 与对 `body.ok` 的 success 判定。

这不是删除同名业务字段：`/chat/llm-call` 的模型上下文 `payload`、各 route 的直接业务结果字段
必须保留。legacy WS 的 `successEnvelope/errorEnvelope/sendResponse/sendError` 也必须保留。

修复后的统一 HTTP wire 是：

```text
2xx: 直接 business result object；没有业务 payload 时为 {}
non-2xx: {"error":{"code":"<code>","message":"<message>"}}
405: 同一 error body，且保留 Allow: POST
```

HTTP status 决定 transport success/failure，body 提供业务结果或结构化错误。dispatcher 应对
`requestId`、`request_id`、`correlationId`、`correlation_id` fail closed，防止 Skiff decoder
忽略 unknown field 后形成“虽未声明但仍被接受”的假删除。

## 5. legacy WS 与 Host 边界

以下 request ID 不属于本 HTTP batch，必须保留：

- `agine_transport.WebSocketRequest`、`decodeWebSocketRequest`、`sendResponse`、`sendError`，
  `ClientMessage`/`ServerEnvelope` 与 `agine_ws_*`，用于旧 receive 多路 response matching；
- client shared/enhanced socket 的 pending-request map；
- `agine/client/e2e/support/cookie-websocket-rpc.mjs`、two-host `wsRequest`、
  `machineHarness.browserWsRequest` 的 WS matching；
- `/thread/host-files/list|search` 到 Host 的异步两跳 workflow。这里的 request ID 关联浏览器
  connection、Host request 和返回结果，是当前真实的 Host file RPC identity；
- Host CLI/`agine/host` 中相同 workflow 的 request ID。

旧 receive 尚存时，防止回流的必要结构边界是：

```text
HTTP JSON -> HTTP-only *Payload（仅业务字段） -> transport-neutral command -> owner
WS frame  -> legacy *Input（eventName + requestId + 业务字段） -> 同一 command -> owner
```

HTTP adapter 不得再通过构造 `requestId:null`、`eventName:"..."` 的 legacy `*Input` 来复用 WS
DTO。`agine_transport` 要把 WS envelope helper 与 HTTP direct-body helper 分开；HTTP files
不得调用 `successEnvelope`、`errorEnvelope` 或 `requestIdFromBody`。架构 checker 应把该边界
变成静态 gate。

## 6. 真实业务 ID 与幂等

| 字段 | 业务语义 / 实际 consumer | 决定 |
| --- | --- | --- |
| `chatId` / `threadId` | chat/thread resource selector、owner 与投影边界 | 保留 |
| `agentId` | Agent resource selector | 保留 |
| `toolProviderId` | Tool provider resource/mount selector | 保留 |
| `messageId` / `messageSeq` | canonical message identity/持久化序号；tool result 用序号核验归属 | 保留 |
| `runId` | Agent run identity；runtime projection、async event gating、terminal 去重均使用 | 保留；stop/background request 中暂未读取也不能由 correlation leaf 误删 |
| `toolCallId` | canonical tool execution selector | 保留 |
| `attemptId` | tool settlement attempt identity 与去重依据 | 保留 |
| OAuth `sessionId` | OAuth workflow resource handle | 保留 |
| `clientInstanceId` | 每 browser tab/session 的稳定来源 metadata，不是每请求 ID | 保留；`/chat/get` 当前未读可另开业务 cleanup |
| model/provider `responseId` | `/chat/llm-call` 投影内的 provider response identity | 保留 |

当前 HTTP `requestId` 从未驱动幂等，删除后不能用另一个模糊 correlation 字段替换。已有 canonical
幂等/去重 owner 包括：

- Agent thread user input 的 `dedupeKey`；
- tool settlement 的 `attemptId` / `settlementDedupeKey`；
- `/chat/create` 的 caller-supplied `chatId` 与 derived `createCommandId`。

如果产品要求 `/chat/send` 在网络 retry 下 exactly-once，需要另行决定显式
`idempotencyKey -> dedupeKey` contract；本 leaf 不自行新增。client
`ThreadRunRequestInfo.requestId` 目前只是无 producer/consumer 的 dead type，也不是已声明 HTTP
wire 字段，可在后续 dead-code cleanup 处理。

## 7. log、mock 与 E2E owner

### 7.1 Browser HTTP owner

`chatHttpRequest` 当前把生成的 ID 写入 request body 与 request log；F426C WIP 的
`ordinaryUserHttpPost` 延续了该行为。repair 后 HTTP-local state 应是 `fetch` Promise、
`Response` 对象和需要取消时的 `AbortController`。业务层已有的 in-flight 状态继续按
`chatId`/`toolCallId` 等业务资源索引，不建立 HTTP correlation map。log 可以记录 method/path、
status 与脱敏后的业务 body，但不生成 wire `requestId`。

### 7.2 Mock

`e2e/support/mockApp.ts` 目前只有 `request(payload)`，以 `payload.eventName` dispatch，并把
ordinary HTTP 与 legacy WS 一起记录到 `sentPayloads`。repair 要增加独立
`httpPost(path,payload)` mock surface；HTTP assertions 观察 `{path,payload}` 或显式
`transport:"http"` 记录，WS `request/send` 继续观察 `eventName/requestId`。不能为了复用
mock 再把 HTTP body 包成 WS message。

### 7.3 E2E helper

- `api.chat-smoke.mjs` 的同一个 `requestId()` 同时服务 HTTP 和 CookieWebSocketRpc；拆成
  WS-only generator，list/get/create/send HTTP body 删除 ID，`postJson` 删除 `body.ok` 和
  `flattenEnvelope`。agents create/delete cleanup 改走 ordinary HTTP 后直接读 business body；
- `system.two-hosts.e2e.ts` 的 preflight/list/activation/create/get/send HTTP ID 与 wrapper
  parsing 删除，文件内 WS helper 的 ID 保留；
- `machineHarness.ts::createChatWithDefaultAgent` 删除 HTTP ID/wrapper parsing，
  `browserWsRequest` 的 ID 保留；
- `frontend.chat.e2e.ts`/`mockApp.ts` assertions 改为 HTTP path/body 与 WS event 两套记录。

## 8. 最小 repair DAG

service/protocol checkpoint 与 browser caller 必须串行，不能并行落地：

```text
A. service/protocol direct-body checkpoint
   |
   v
B. browser ordinary caller + original caller checkpoint
   |
   v
C. combined static/type/test/E2E-helper gates
```

### A. service/protocol checkpoint

精确 write owner：

- `agine/protocol/http.ts`；
- `agine/service/api/agine.skiff`；
- `agine/service/internal/agine_transport.skiff`；
- `agine/service/internal/agine_http_dispatch.skiff`；
- `agine/service/internal/agine_http_chat.skiff`；
- `agine/service/internal/agine_http_provider.skiff`；
- `agine/service/internal/agine_http_agent_provider.skiff`；
- `agine/service/internal/agine_http_tool_providers.skiff`；
- `agine/service/internal/agine_http_user_tools.skiff`；
- 为拆分 transport-neutral command 所必需的
  `agine/service/internal/thread_store.skiff`、
  `agine/service/internal/tool_result_adapter.skiff`、
  `agine/service/internal/agine_ws_chat.skiff` 与
  `agine/service/internal/agine_ws_host_tool_files.skiff`；
- 对应 `agine_http_*.test.skiff`、service API receipt/architecture checker 与 Agine
  contract README。

该 checkpoint 一次性删除 28 个 input hit、通用 raw echo 与 34 个 response envelope，并证明
WS DTO/WS response matching 未变。A 完成前不能让 browser 依赖 direct response shape。

### B. browser caller checkpoint

在 A 之后：

- `agine/client/src/lib/http.ts` 删除 auto-ID 与 legacy flatten，提供 literal
  `post(path,businessPayload)`；
- 原 `/chat/list|create|get|send` caller、`ModelContextViewer` 与 activation/provider caller
  统一直接读取 business body；
- 22 个 ordinary caller 从 socket request 切到 HTTP path/payload；
- unit mock、frontend mock 与上述 E2E helpers 同步拆分 transport。

HTTP call 的局部 Promise/AbortController 取代 correlation key；legacy socket pending map 不变。

### C. combined gates

先跑 service checkpoint，再跑 client，最后做双向反搜与 write-scope 检查。任何 HTTP test
继续断言 `requestId`、`eventName`、`ok` 或 wrapper flatten 都应失败；同时 WS/Host tests 必须
继续证明 request ID matching 有效。

## 9. F426C WIP 可复用性

WIP 相对精确候选只修改 15 个 `agine/client` 文件，`442 insertions / 120 deletions`，没有
service/protocol change，因此不是可接受候选。

可在 A 完成后按概念或逐 hunk 重放：

- `ToolCallCard`、`threadHostBindings`、`toolproviderApi`、`chatActions`、`configActions`、
  `messageActions` 从 legacy request/send 改为 literal HTTP path/body，以及对应 mock 分离；
- `protocol.ts` ordinary payload builder 去掉 `eventName` wrapper；
- `socket.ts` application JSON heartbeat no-op 与 correlation cutover 独立，可单独复用。

必须丢弃或重写：

- `ordinaryUserHttpPost` 的 `nanoid()`/`requestId` 注入；
- `chatHttpRequest` 的既有 `nanoid()`/`requestId` 注入；
- `flattenLegacyEnvelope` 及所有 `ok/payload` wrapper parsing；
- 期待 HTTP `requestId`、`eventName:"*-response"`、flattened duplicate fields 的 tests。

WIP 还遗漏原 6 个 request hit、全部 service schema/adapter、34 个 response wrapper 与 E2E
helper，因此不能把 WIP commit 整体 cherry-pick 后视为 F427 完成。

## 10. repair 验证命令与反搜 gate

在 repair-owned Internals worktree 和明确指向 repair-owned Skiff worktree 的 `SKIFF_ROOT`
运行：

```bash
SKIFF_ROOT=<repair-owned-skiff-worktree> npm run type-check --workspace @agine/service
SKIFF_ROOT=<repair-owned-skiff-worktree> npm test --workspace @agine/service
npm run type-check --workspace @agine/client
npm run test:logic --workspace @agine/client
npm run test:frontend --workspace @agine/client
node --test agine/client/e2e/support/cookie-websocket-rpc.test.mjs
git diff --check
```

反搜必须同时做负向与正向 gate：

```bash
rg -n -i 'request[_-]?id|correlation[_-]?id' \
  agine/protocol/http.ts agine/service/internal/agine_http_*.skiff \
  agine/client/src/lib/http.ts

rg -n 'requestIdFromBody|flattenLegacyEnvelope|-[Rr]esponse|tool_call/receipt' \
  agine/service/internal/agine_http_*.skiff agine/client/src/lib/http.ts

rg -n 'requestId|request_id|correlationId|correlation_id' \
  agine/client/e2e/api.chat-smoke.mjs \
  agine/client/e2e/system.two-hosts.e2e.ts \
  agine/client/e2e/support/machineHarness.ts

rg -n 'requestId' \
  agine/service/internal/agine_ws_*.skiff \
  agine/client/e2e/support/cookie-websocket-rpc.mjs \
  agine/host
```

前三个命令不能简单要求整文件零命中：E2E 文件同时含应保留的 WS helper。checker/test 必须按
HTTP route/body 与 WS function 分区，证明 HTTP JSON 不含四个禁用字段；最后一个正向 gate
则必须仍命中并由 WS/Host tests 覆盖。另需对所有 34 个普通 HTTP 路由断言 success/error
body 不含 `eventName`、`requestId`、`ok` 或 transport outer `payload`，其中
`/chat/llm-call` 明确允许自己的业务 `payload`。

## 11. 本 leaf clean 状态

本 leaf 只执行 `git`/`rg`/`sed` 等只读检查，没有运行会触达 stable/live 的 smoke，也没有
改动或测试尚未实现的 repair。交付写入仅为本文档。提交后 Skiff task worktree 应为 clean；
result-only commit/tree 与最终 clean status 由交付消息记录。没有 merge、rebase、push、
instance/watch/reload 或派生 Agent。
