# P5-F436B Agine legacy requestId terminal owner audit result

状态：`TASK_SCOPE_EXPANDED`（只读审计完成，repair 未执行）。

非 Host 的 legacy WebSocket request/response matching 已具备终态删除条件；Host 的 HTTP
credential/current-process lease、activation hello/ack replay，以及 Host file 两跳结果方向和
job lifecycle 尚未冻结。现有代码不能回答这些产品/安全 contract 问题，本审计不代替 owner
作选择。

## 1. 精确输入、权威与边界

| 输入 | commit | tree | 审计前状态 |
| --- | --- | --- | --- |
| Internals candidate | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` | clean |
| Skiff result base | `fb06108be8ea9c370216f52891fddddb1ccca340` | `43ca843d6e19e73e37ff7f81c03191c40c1dc29f` | clean |

唯一 contract authority 是
`doc/architecture/package-service-contract-deployment.md` 第 3 节与第 6.4 节：一个 service
最多一个 WebSocket entry，只允许 `path` 与可选 `connect`；WebSocket 是 server-to-client
downlink，客户端 text/binary data frame 必须由 Router 以 `1003` 拒绝；ping/pong/close
属于协议栈；HTTP unary response 天然归属于其 request，不能保留业务 `requestId`
matching。

本审计读取了任务指定的 Internals candidate、直接关联的 tests/E2E/docs/package scripts，
并只读核对 Skiff Router、runtime wire 和 `std/websocket.skiff`。没有修改 Internals，没有
运行 build、dev、start、stable、live、固定端口或网络 workload。

以下分类贯穿全文：

| 标记 | 意义 | 终态 |
| --- | --- | --- |
| `DEL-WS` | Agine legacy WS req/res matching 或只为它存在的 dead projection | 删除，不留 shim |
| `KEEP-NEG` | 已完成 HTTP direct-body/fail-closed 断言 | 保留为显式负例 |
| `KEEP-BIZ` | 真实资源、run、tool、attempt identity | 保留，不能被字符串清扫误改 |
| `KEEP-3P` | 第三方/其他产品协议或记录字段 | 由其 owner 保留 |
| `DESIGN-JOB` | 确有异步 command/result 生命周期，但名字错误 | 决策后改为明确 `jobId` 等业务 handle |

## 2. 全量命中与 owner/path 矩阵

精确、大小写敏感扫描
`requestId|request_id|correlationId|correlation_id` 的主候选结果是 54 个文件、382 个命中：

| scope | 文件 | 命中 |
| --- | ---: | ---: |
| `agine/service/**` | 33 | 288 |
| `agine/protocol/**` | 0 | 0 |
| `agine/client/**` | 11 | 54 |
| `agine/host/**` | 6 | 28 |
| `shared-client/**` | 4 | 12 |

按 token 为：`requestId` 346、`request_id` 12、`correlationId` 12、
`correlation_id` 12。后三种 token 的 36 个命中全部属于 HTTP negative/checker；
没有正向 producer、decoder 或业务 owner。另有 3 个直接相关旧文档、8 个命中，因此完整
审计集合是 57 个文件、390 个命中。相关 package scripts 本身为零命中。

### 2.1 Service

| path | 数量 | 分类 | owner 与终态 |
| --- | ---: | --- | --- |
| `agine/service/README.md` | 5 | `KEEP-NEG` + `DEL-WS` | 保留四 alias fail-closed 说明；删除 receive/RPC、旧 manifest/API 说明 |
| `agine/service/api/agine.skiff` | 38 | `DEL-WS` + `DESIGN-JOB` | 删除 legacy `*Input.requestId`、`ServerEnvelope`；Host file relay 字段须在决策后成为 `jobId` |
| `agine/service/internal/agent_bridge.chat_config.test.skiff` | 7 | `DEL-WS` | `requestId:null` 只是 legacy input fixture；业务测试改用 command |
| `agine/service/internal/agent_bridge.host_tools.test.skiff` | 1 | `DEL-WS` | 同上；保留真实 tool identity |
| `agine/service/internal/agent_bridge.lifecycle.test.skiff` | 3 | `DEL-WS` | 同上；保留 chat/run lifecycle |
| `agine/service/internal/agent_bridge.synthetic_projection.test.skiff` | 1 | `DEL-WS` | 同上 |
| `agine/service/internal/agent_bridge.user_projection.test.skiff` | 1 | `DEL-WS` | 同上 |
| `agine/service/internal/agent_bridge_host_wake.test.skiff` | 2 | `DEL-WS` | 只删 transport fixture field；`attemptId`/wake determinism 保留 |
| `agine/service/internal/agine_http_chat.test.skiff` | 1 | `DEL-WS` | HTTP test 直接构造 legacy `ChatCreateInput`；改用 transport-neutral command |
| `agine/service/internal/agine_service.delete_contract.test.skiff` | 2 | `KEEP-NEG` + `DEL-WS` | direct HTTP body 无 `requestId` 的断言保留；legacy success envelope test 删除 |
| `agine/service/internal/agine_service.llm_call.test.skiff` | 1 | `KEEP-NEG` | direct body 不含 `requestId` 的负例保留 |
| `agine/service/internal/agine_service_architecture.test.mjs` | 22 | `KEEP-NEG` + `DEL-WS` | 保留 HTTP alias/DTO/direct-body gate；反转 legacy facade、manifest、WS owner、envelope gate |
| `agine/service/internal/agine_service_dispatch.test.skiff` | 12 | `KEEP-NEG` + `DEL-WS` | 四 alias 400/no-echo 与 direct body 负例保留；decoder/envelope/unknown WS event tests 删除 |
| `agine/service/internal/agine_transport.skiff` | 33 | `KEEP-NEG` + `DEL-WS` | 唯一保留 `hasForbiddenHttpCorrelationField`；删除 WS decoder、success/error envelope 和 response sender |
| `agine/service/internal/agine_ws_access.skiff` | 8 | `DEL-WS` | req/res error correlation helper 整体删除 |
| `agine/service/internal/agine_ws_agent_provider.skiff` | 19 | `DEL-WS` | 普通业务已有 HTTP owner；整个 legacy WS adapter 删除 |
| `agine/service/internal/agine_ws_chat.skiff` | 19 | `DEL-WS` | 同上 |
| `agine/service/internal/agine_ws_dispatch.skiff` | 5 | `DEL-WS` | receive dispatcher 与业务 JSON `ping`/`pong` 删除 |
| `agine/service/internal/agine_ws_host_tool_files.skiff` | 42 | `DEL-WS` + `DESIGN-JOB` | Host uplink 改 HTTP 后删除旧 adapter；file relay 先冻结 job/result contract |
| `agine/service/internal/agine_ws_tool_providers.skiff` | 15 | `DEL-WS` | 普通业务已有 HTTP owner；整个 legacy WS adapter 删除 |
| `agine/service/internal/conversation_title_tool.test.skiff` | 2 | `DEL-WS` | legacy chat input fixture field 删除；title downlink 测试保留 |
| `agine/service/internal/host_file_browse.test.skiff` | 11 | `DEL-WS` + `DESIGN-JOB` | browser transport ID 删除；relay claim/expiry/owner tests 改为 frozen `jobId` lifecycle |
| `agine/service/internal/host_file_rpc.skiff` | 16 | `DEL-WS` + `DESIGN-JOB` | `HostFileBrowseRequest.id` 已是 relay 主键；若保留 async workflow，由它成为 `jobId` |
| `agine/service/internal/host_tool_settlement.test.skiff` | 7 | `DEL-WS` | legacy result/chat fixtures 删除 field；`attemptId`/`toolCallId` settlement 继续是 owner |
| `agine/service/internal/host_toolprovider_connection.skiff` | 1 | `DEL-WS` | current-directory refresh 随机 ID 没有 service consumer，删除；downlink trigger 保留 |
| `agine/service/internal/host_toolprovider_current_directory.test.skiff` | 1 | `DEL-WS` | legacy host input fixture 删除 |
| `agine/service/internal/host_toolprovider_rename.test.skiff` | 1 | `DEL-WS` | legacy toolprovider input fixture 删除 |
| `agine/service/internal/host_toolprovider_runtime.skiff` | 2 | `DESIGN-JOB` | 当前只透传 browser request ID；改为 frozen job API |
| `agine/service/internal/model.skiff` | 1 | `DEL-WS` + `DESIGN-JOB` | 删除 `HostFileBrowseRequest.requestId`；已有 `id` 是唯一可复用 stable relay identity |
| `agine/service/internal/thread_store.create_thread_id.test.skiff` | 1 | `DEL-WS` | legacy input fixture 删除；caller-supplied `chatId` 保留 |
| `agine/service/internal/thread_store.list.test.skiff` | 2 | `DEL-WS` | legacy input fixture 删除 |
| `agine/service/internal/tool_result_adapter_background.test.skiff` | 2 | `KEEP-NEG` + `DEL-WS` | direct HTTP no-ID 断言保留；legacy Host result fixture field 删除 |
| `agine/service/service-api-receipt.test.mjs` | 4 | `KEEP-NEG` | 四 alias 不得进入 36 个现有 HTTP contract 的 gate 保留并扩展到新 Host entries |

### 2.2 Browser client

| path | 数量 | 分类 | owner 与终态 |
| --- | ---: | --- | --- |
| `agine/client/e2e/api.chat-smoke.mjs` | 1 | `DEL-WS` | Cookie helper 的 ID factory 删除；纯 downlink observer 保留 |
| `agine/client/e2e/frontend.chat.e2e.ts` | 5 | `KEEP-NEG` + `DESIGN-JOB` | 四 HTTP no-alias 断言保留；Host-file mock WS ID 改为 frozen HTTP/job contract |
| `agine/client/e2e/support/cookie-websocket-rpc.mjs` | 8 | `DEL-WS` | request producer、`pending` Map、matching 删除；可降为 cookie downlink observer |
| `agine/client/e2e/support/cookie-websocket-rpc.test.mjs` | 5 | `DEL-WS` | pending/abort/timeout/close RPC tests 删除，替换为 connect/downlink/1003 tests |
| `agine/client/e2e/support/machineHarness.ts` | 5 | `DEL-WS` | `browserWsRequest` 及普通 RPC callers 改 HTTP |
| `agine/client/e2e/support/mockApp.ts` | 4 | `DEL-WS` | mock request ID/envelope 删除；HTTP caller 与 downlink emitter 分离 |
| `agine/client/e2e/system.two-hosts.e2e.ts` | 10 | `DEL-WS` | `/toolproviders`/agents WS helpers 改 HTTP；另测 uplink `1003` |
| `agine/client/src/architecture.client-boundaries.test.ts` | 4 | `KEEP-NEG` | HTTP helper 不含四 alias 的 gate 保留 |
| `agine/client/src/components/GlobalErrorHandler.tsx` | 3 | `DEL-WS` | 删除 `originalMessage.requestId` 特例；无 Agine producer 的 `server_error` listener 也应清理 |
| `agine/client/src/lib/http.test.ts` | 8 | `KEEP-NEG` | 两组四 alias negative 保留 |
| `agine/client/src/lib/types.ts` | 1 | `DEL-WS` | `ThreadRunRequestInfo.requestId` 无任何 producer/consumer；删除 dead field/type，不凭名字创造新 owner |

### 2.3 Host

| path | 数量 | 分类 | owner 与终态 |
| --- | ---: | --- | --- |
| `agine/host/src/HostRuntime.test.ts` | 6 | `DEL-WS` + `DESIGN-JOB` | current-directory echo ID 删除；file cases 改 frozen `jobId`/HTTP result |
| `agine/host/src/HostRuntime.ts` | 15 | `DEL-WS` + `DESIGN-JOB` | current-directory ID 删除；file relay ID 在决策后改 `jobId`；所有 Host uplink 改 HTTP |
| `agine/host/src/cli.test.ts` | 2 | `DEL-WS` | current-directory echo matching test 删除 |
| `agine/host/src/gateway/types.ts` | 2 | `DEL-WS` | 无使用者的 generic `HttpEnvelope`/`HttpResponseEnvelope` 删除 |
| `agine/host/src/shared/enhanced_websocket.ts` | 2 | `DEL-WS` | 零调用的 copied `request()`/nested error matching 删除；Node `ws.ping()` control frame 保留 |
| `agine/host/src/shared/event_manager.ts` | 1 | `DEL-WS` | request-specific nested-path comment/能力随 copied RPC cleanup 收窄；普通 event listener 保留 |

### 2.4 Shared client 与第三方 owner

| path | 数量 | 分类 | owner 与终态 |
| --- | ---: | --- | --- |
| `shared-client/shared/components/admin/AdminPurchaseList.tsx` | 1 | `KEEP-3P` | App Store notification ingestion record 的字段；本 source 不能证明它是 Apple wire 还是该 subsystem 的 audit ID，均归 purchase notification schema，不是 Agine transport |
| `shared-client/shared/lib/enhanced_websocket.ts` | 6 | 非 Agine legacy owner | 其他产品仍有大量 `socket.request()` caller，不能由 Agine leaf 全局删除；Agine 改用无 `request/send` 的 downlink-only class/interface |
| `shared-client/shared/lib/event_manager.ts` | 1 | 非 Agine legacy owner | nested condition 仍服务 shared legacy callers；Agine downlink surface 不依赖它做 correlation |
| `shared-client/shared_capacitor/components/GlobalErrorHandler.tsx` | 4 | 非 Agine legacy owner | 其他 shared legacy request caller 的 handler；不随 Agine terminal cutover 全局删除 |

shared-client 的 active request callers 包括 upload、chatty、notification、App Store、admin 等
多个非 Agine product surface。terminal gate 应证明 Agine 不再实例化该能力，而不是在本 batch
破坏这些 owner。

### 2.5 直接相关 docs 与零命中 companion

| path | 数量 | 终态 |
| --- | ---: | --- |
| `agine/docs/cookie-session-http-ws-migration.md` | 1 | 删除“HTTP 生成 requestId”的旧指导 |
| `agine/docs/multi-host-file-picker-design.md` | 1 | 将旧 transport ID 设计替换为 frozen job/result contract |
| `agine/docs/skiff-mechanisms-and-packages.md` | 6 | 删除 receive/RPC compatibility 示例 |

`agine/service/service.yml`、`api.yml`、`internal/agine_service.skiff`、
`internal/host_runtime_architecture.test.mjs`、`agine/host/scripts/host-architecture.test.mjs` 和三个
package.json 虽无或不以本 regex 命中，却是直接 contract/checker owner，必须进入 repair。
package scripts 继续作为 gate 入口，不应为了本 cutover改名或绕过。

### 2.6 必须保留的业务 identity 与 semantic control

在 scoped exact bare-name 命中中，没有一个 live Agine `requestId` 被证明是业务资源 identity。
真正 owner 都已有明确名称：

| identity | owner/evidence | 结论 |
| --- | --- | --- |
| `chatId`/`threadId`, `messageId`/`messageSeq`, `runId` | thread、canonical message、agent run | `KEEP-BIZ` |
| `toolProviderId`/`mountId` | Host mount/provider selection 与授权 | `KEEP-BIZ` |
| `toolCallId`/`attemptId` | tool execution、durable Host ledger、settlement idempotency | `KEEP-BIZ` |
| `clientInstanceId` | browser tab/session source metadata，不是每请求 correlation | `KEEP-BIZ` |
| `HostFileBrowseRequest.id` | 当前 `relayId` primary key，claim、TTL、host result lookup 都使用 | async 方案若成立，重命名/暴露为 `jobId`，不再另造 ID |
| shared `llmRequestId` | shared chat 的模型 invocation identity；case-insensitive control scan 可见 | `KEEP-BIZ`，不属于 exact bare token |
| package `AgentChildAgentRequest.id` | `packages/agent/thread_subagent_queue.skiff` 的持久化 child request resource | scope 外 `KEEP-BIZ`；不能被 repo-wide zero-hit gate 误删 |

`ThreadRunRequestInfo.requestId` 是唯一看似模型 request identity 的 in-scope bare field，但全仓只有
声明，无 producer/consumer；当前证据支持删除 dead projection，而不是保留或擅自重命名。若未来
重新引入，应由模型 invocation owner 定义 `modelRequestId`/`runId` contract。

## 3. 真实 frame 路径与 dead source graph

当前可达路径没有任何一个 hop 能把客户端 text/binary data frame 送到 Agine receive：

```text
browser ClientWebSocket.send / Host ws.send
  -> browser/Node WebSocket data frame
  -> Skiff Router webSocketGateway.attachSocket socket.on("message")
  -> close(1003, "client data frames are not supported")
  -> STOP；没有 runtime dispatch、service receive 或业务 side effect
```

`router/src/gateway/webSocketGateway.ts:449-454` 对所有 `message` 事件执行上述 close，text 与
binary 没有分支差异。upgrade/connect 仍会构造一次 Router→Runtime
`request.start.requestId`；这是 Skiff 内部 invocation correlation，不是 Agine wire。
`router/src/protocol/envelope.ts` 的 `connection.send` 只有 service/entry、business identity
或 connection ID 与 payload，没有 `requestId`。

当前 Internals 仍 author 了一个不可达且已不符合当前 std ABI 的静态图：

```text
service.yml websocket.routes[].operation
  -> api.yml websocket export
  -> internal.agine_service.websocket(WebSocketIngressEvent<ConnectionContext>)
  -> receive branch
  -> agine_ws_dispatch.receive
  -> agine_ws_chat / agine_ws_host_tool_files /
     agine_ws_tool_providers / agine_ws_agent_provider
  -> agine_transport response envelope
```

当前 `std/websocket.skiff` 只有非泛型 `WebSocketConnectRequest`、
`WebSocketConnectResult` 与 downlink send native；没有 service receive/context ABI。
因此这不是可保留的 compatibility path，而是 dead source。

终态删除闭集：

- production：`agine_service.websocket` 的 receive facade、`agine_ws_dispatch.skiff`、
  `agine_ws_access.skiff`、四个 `agine_ws_*` RPC adapter、`agine_transport` 的 WS decoder 和
  response/error envelope/sender；Host/browser 的 uplink `send/request` producers；
- API：`api.yml::websocket` 与只为旧 generic facade 导出的 `ConnectionContext`，以及
  `api/agine.skiff` 的 legacy input/envelope types；
- manifest：`websocket.routes`、`operation`；
- tests/fixtures：`expectedWsOwners` 35-event inventory、facade receive assertions、
  decoder/envelope tests、所有仅为构造 legacy input 的 `requestId:null`；
- checkers：把“必须存在 legacy receive/requestId”反转为“不得存在”，同时保留 HTTP negative；
- E2E：Cookie RPC pending map、machine/two-host WS request helpers、mock WS response envelope；
- docs：第 2.5 节三个旧文档与 README legacy 描述。

普通业务已迁到 36 个 HTTP route；Host branch 必须先完成第 6 节决策并新增 HTTP entry，才可删除
其 dead adapter。删除必须是 source deletion/transport-neutral command 调用，不得留
`receiveLegacy`、no-op handler、兼容 envelope 或把 HTTP body重新包成 WS DTO。

## 4. WebSocket downlink 保留矩阵

以下事件当前同时有 production producer 与 consumer，且仍是业务 downlink：

| event | service producer | consumer | matching owner |
| --- | --- | --- | --- |
| `chat/title-updated` | `conversation_title_tool.skiff` | `wsTranscriptHandlers.ts` | `chatId` |
| `chat/message-added` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers.ts` | `chatId`、`runId`、`messageSeq` |
| `chat/step-start` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers.ts` | `chatId`、`runId`、`stepId`、`messageSeq` |
| `chat/step-finish` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers.ts` | 同上 |
| `chat/text-delta` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers.ts` | 同上 |
| `chat/reasoning-delta` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers.ts` | 同上 |
| `chat/run-completed` | `agent_bridge_event_projection.skiff` | `wsRunHandlers.ts` | `chatId`、`runId`、terminal seq |
| `chat/run-failed` | `agent_bridge_event_projection.skiff` | `wsRunHandlers.ts` | 同上 |
| `chat/run-stopped` | `agent_bridge_event_projection.skiff` | `wsRunHandlers.ts` | 同上 |
| `host/current-directory/request` | `host_toolprovider_connection.skiff` | `HostRuntime.ts` | one-way target `toolProviderId`；删随机 request ID |
| `host/files/list-request` | `host_file_rpc.skiff` | `HostRuntime.ts` | frozen async 方案成立时为 `jobId` |
| `host/files/search-request` | `host_file_rpc.skiff` | `HostRuntime.ts` | 同上 |

`agine_connect.skiff` 在 connect 时调用 `chat_stream.rememberConnection`；Host connect 又建立
`activeConnectionId`。以上 producer 通过 connection ID/business identity 选目标，payload
用业务 ID 归并状态。它们不依赖客户端先发 receive frame，也不依赖 response matching。

当前还存在需要在 repair 中显式关闭的孤儿：

- `chat/tool-execute-start` 与 `chat/tool-execute-finish` 有 service producer，但 browser
  production 没有 handler，现有 browser test 甚至断言 handler 不存在；选择删除 producer
  或另开产品事件 owner，不能假装是 request/response；
- `chat/provider-switched`、`chat/provider-retry-waiting`、`machine/online_change` 有 client
  listener，但未发现 Agine production producer；
- browser `tool_call/request` listener 没有直接 downlink producer；Host tool request 当前是
  `host/tool-attempts` response action 内的业务对象；
- Agine `server_error` listener 在 legacy error sender 删除后没有 producer。

shared browser 的 application JSON `ping` 已被 `AgentSocket.startHeartbeat()` override 禁用；
Host 使用 Node `ws.ping()`/`pong` control frame，应保留。`agine_ws_dispatch` 中业务 JSON
`ping -> pong` 必须删除。当前名为 `host/ping` 的业务事件是 presence update，转 HTTP 时应以
presence 语义建模，不能当作协议 ping RPC。

## 5. Host flow、认证、生命周期与 HTTP 分类

| flow | 当前方向与 producer/consumer | 当前认证 | retry/timeout/result owner | HTTP 终态分类 |
| --- | --- | --- | --- | --- |
| activation-token issuance | browser POST `/hosts/activation-token` → service | user HTTP session | token TTL 300000ms；`HostActivationToken` 状态 `pending/issued/consumed/...` | 已有 endpoint，保持 |
| activation hello | Host WS `host/hello` → service；`host/hello-response` → HostRuntime | connect header `X-Agine-Host-Activation: agha_*` 产生 context/active connection | initial timeout 10s；`issued` 可复用 temporary host ID；Host 先持久化 ID | 需要新的 Host HTTP entry；auth/replay 未冻结 |
| activation ack | Host WS `host/activation-ack` → service | 仍依赖最初 activation WS context 中 token hash + current connection | timeout 5s；把 token `issued -> consumed`；ack 近似幂等 | 需要新的 Host HTTP entry；token/host credential 切换未冻结 |
| run hello | 每次 WS open/reconnect 由 `GatewayClient.sendHello` 发出 | `Authorization: AgineHost agh_*` connect auth + current `activeConnectionId` | 10s；刷新 name/CWD/capabilities/presence；response 启动 loops | 需要新 Host HTTP entry |
| presence | hello 后立即、以后每 15s Host→service | current Host context/connection | 只有 error response；CWD/capabilities piggyback | 需要新 Host HTTP presence entry |
| tool-attempt sync | Host durable ledger snapshots 每 1s → service；actions response → Host runtime | current Host context/connection | 同时最多一个 outstanding；disconnect 清空 outstanding；`attemptId`/`toolCallId` owner | 需要新 Host HTTP entry |
| tool result | Host ledger result → service settlement；receipt → Host ledger | current Host context/connection | 网络/对账会重发；settlement 由 `attemptId`/`toolCallId` 去重 | 现有 `/tool_call/result` 只允许 user session + `executor=client`；Host route/auth 需决策 |
| current directory | CWD piggyback hello/presence/attempt；service 也 one-way downlink refresh request，Host 回传 | current Host context/connection | refresh `requestId` 在 service 完全不读；无需 async handle | 用户 `/toolproviders/current-directory` 只是读缓存；Host write 需新 HTTP entry，downlink trigger 无 ID |
| file list/search | browser WS request → service DB relay → Host downlink → Host WS result → exact browser connection response | browser session + current Host context/connection 双 hop | Host local 15s；service DB TTL 25s；一次 claim，错误 owner/kind/connection 不消费 | browser 与 Host result 都需新 HTTP entries；result channel/job lifecycle 未冻结 |

对应的实际 source owner 是：

- token issuance：
  `agine_http_provider.hostActivationToken -> host_toolprovider_runtime.createHostActivationToken`；
- activation/run hello、ack、presence、attempt：
  `GatewayClient`/`HostRuntime -> agine_ws_host_tool_files.dispatchHostLifecycle ->
  HostCoordinator.helloWithRuntime/activationAck/pingWithRuntime/syncToolAttempts -> HostRuntime`
  response listeners；
- tool result：
  `HostToolAttemptRuntime -> GatewayClient.send -> dispatchToolResult ->
  HostCoordinator.onToolResult -> HostToolAttemptRuntime.handleReceipt`；
- current directory：
  `host_toolprovider_connection.requestHostCurrentDirectoryRefresh -> HostRuntime ->
  HostCoordinator.currentDirectory`，而正常 refresh 也通过 hello/presence/attempt payload；
- file list/search：
  `client hostFileApi -> dispatchUserFileRequest -> host_file_rpc.dispatchHostFileBrowseRequest ->
  HostRuntime file handler -> host_file_rpc.receiveHostFileBrowseResult -> browser pending caller`。

### 5.1 已有 endpoint 不能被误当成 Host uplink

- `/hosts/activation-token` 是用户签发 activation token，已经是 HTTP；它不等于 Host 使用 token
  的 hello/ack；
- `/toolproviders/current-directory` 是用户读取缓存并可触发 refresh，不接受 Host 写入；
- `/tool_call/result` 明确拒绝 Host auth header，且只接受 `executor=client`；
- `/thread/host-files/list|search` 当前不在 `service.yml`/HTTP route table，architecture checker
  明确断言它们不存在；不能只改 browser caller 而不增加 service contract。

因此除了用户签发 token 之外，没有一条 Host uplink 可“直接切到已有 endpoint”。

### 5.2 Host file 两跳的 stable identity

`HostFileBrowseRequest.id` 当前由 `relayId = newId("host_files")` 生成，是 DB primary key，也是
Host downlink、result claim、owner/kind/connection 验证和 TTL cleanup 的 lookup key。若冻结为
异步 workflow，它就是唯一合理的 `jobId` owner；`requestId` 字段只是原 browser socket
matching key，应删除，不能再并行新增第二个模糊 correlation ID。

但现有 model 只有 pending record，并在 claim 时立即删除；没有 completed/error 状态、result
retention、HTTP poll visibility、browser cancellation 或 retry/replay contract。更关键的是，
当前 response 用 `browserConnectionId` 精确回到发起 tab，而 session-cookie HTTP 不能天然恢复
这个 target。故本审计只能识别 stable identity，不能自行决定它要被同步 HTTP、polling 或
job-keyed downlink怎样暴露。

## 6. `TASK_SCOPE_EXPANDED` 的最小决策问题

1. **Host HTTP credential 与 current-process lease**：Host POST 是否直接使用
   `Authorization: AgineHost <agh_*>`？当前授权还要求 `activeConnectionId` 与
   `ConnectionContext` 完全匹配；HTTP 没有该 context。是由 credential 单独授权，还是由
   connect/hello 签发短期 lease？cookie、Host header、activation header 同时出现时谁优先？
2. **activation replay/ack**：activation token 是否直接授权 HTTP hello 并在 `issued` 状态重复
   返回同一 temporary Host ID？Host 已持久化 `agh_*` 后，ack 用 activation token、Host
   credential 还是 lease？timeout/retry、token expiry 与重复 ack 的精确结果是什么？
3. **Host tool result route**：复用 `/tool_call/result` 的 dual-auth contract，还是新增
   Host-only path？错误/同时存在的 user session 与 Host header 如何 fail closed？是否仍要求
   current-process lease？receipt 是否只按 `attemptId`/`toolCallId` 幂等？
4. **Host file result与 exact-tab**：选择长连接 HTTP、`POST -> {jobId}` 加 poll，还是
   `jobId`-keyed WS downlink？谁可读取结果、结果保留多久、何时 terminal、如何取消/重试，
   以及多个同 session tab 中谁收到结果？若选 async，确认复用
   `HostFileBrowseRequest.id -> jobId`。

这些问题分别由现有 `agine_connect.skiff` header/context、activation token state machine、
`agine_http_user_tools.skiff` 的 Host deny、以及 `HostFileBrowseRequest` 的
browser/host connection fields直接证明。冻结前不能开始 Host repair leaf。

## 7. Browser、Host、shared-client 删除闭集

| surface | 当前状态 | 终态 |
| --- | --- | --- |
| browser `hostFileApi -> ws.request` | Agine production 唯一 `request()` caller | 改 frozen HTTP/job API；删除 `ws.request` export 与 mock request |
| shared `EnhancedWebSocket.request()` | nanoid + two `once` listeners + timeout；listener 集合就是 pending state | Agine 不再实例化；其他产品 legacy owner 保留 |
| Cookie E2E helper | 显式 `pending` Map，以 `requestId`+response event match | 删除 RPC 能力；保留 downlink observation |
| browser `socketBridge.send` | production 无 caller，仍暴露 uplink | 删除；保留 connect/on/off 与 event normalization |
| browser GlobalErrorHandler | 跳过 `originalMessage.requestId` | 删除特例；无 producer 时删除整个 Agine listener |
| Host copied `request()` | 没有任何 call site，但保留 response/error listener | 删除 method、request-only nanoid/import/comment |
| Host `GatewayClient.send` | hello/presence/attempt/result/file-result 全经 WS uplink | 改 Host HTTP client；WS object只接收 downlink |
| service downlink send | chat/Host producer 直接按 connection/business identity 发送 | 保留 |
| browser/Host listener | 消费第 4 节真实 downlink | 保留；Host response listeners随 HTTP迁移删除 |

为使类型边界也 fail closed，推荐由独立 shared owner 新增 downlink-only class/interface，
只暴露 connect/close/on/off，不暴露 request/send；现有 shared legacy class 不做破坏性全局
删除。Browser 与 Host 不能仅靠“当前没人调用”继续继承可发 data frame 的 surface。

## 8. `service.yml` 与 public API 终态

当前：

```yaml
websocket:
  routes:
    - path: /ws
      operation: websocket
```

终态最多一个 singleton entry：

```yaml
websocket:
  path: /ws
  connect:
    handler: internal.agine_service.websocketConnect
    adapterArgs:
      - param: request
        source: { kind: websocket.connectRequest }
```

Agine 需要 connect 做 user session/Host auth，因此可以保留可选 connect handler。必须删除
`routes`、`operation`、receive branch、generic `WebSocketIngressEvent` facade，以及
`api.yml` 的 `websocket` callable export。connect-only internal handler直接使用当前
`std.websocket.WebSocketConnectRequest/Result`；不能继续 author generic context result。
downlink native send 不需要 API-exported callable。

## 9. 可执行 repair DAG

`D0` 是冻结第 6 节四项决定；未完成时 Host leaves 必须保持 blocked。

```text
D0 Host auth/activation/tool-result/file-job decisions
  |
  v
C1 shared contract checkpoint
  |----------------------|----------------------|
  v                      v                      v
X2 shared downlink       S2 Host service/HTTP   O2 ordinary/dead WS source
specialization           implementation          deletion (after S2 contract)
  |                      |                      |
  |                      v                      |
  |                  H3 Host CLI HTTP            |
  |                      |                      |
  +-----------> B3 browser HTTP/job <------------+
                         |
                         v
                    V4 combined owner
```

### 9.1 不重叠 owner

| leaf | production 写入 owner | test/doc owner | 依赖 |
| --- | --- | --- | --- |
| `C1-contract` | `service.yml`, `api.yml`, `api/agine.skiff`, `agine_service.skiff`, connect-only facade/manifest；HTTP route/schema checkpoint | `service-api-receipt.test.mjs`, 整个 `agine_service_architecture.test.mjs`, README 与三份相关 docs | `D0` |
| `X2-shared` | 新的 shared downlink-only socket source；现有 generic legacy class只做必要抽取，不删其 request caller contract | 新的 shared socket boundary tests | `C1` |
| `S2-host-service` | 新 Host HTTP auth/handlers，`host_toolprovider_*`, `host_file_rpc.skiff`, `model.skiff`; 若决定复用 user route，则独占 `agine_http_user_tools.skiff` | `host_file_browse`, Host auth/current-dir/tool settlement tests，整个 `host_runtime_architecture.test.mjs` | `D0`,`C1` |
| `O2-old-ws` | `agine_transport` WS 部分与 `agine_ws_*.skiff`；只删除 `agine_ws_host_tool_files`，不写 C1 的 API/facade 或 S2 files | dispatch/delete contract 与所有非 Host legacy `requestId:null` service fixtures | `C1`,`S2` |
| `H3-host-cli` | 整个 `agine/host/**` production transport cutover | Host unit/architecture/package-boundary tests | `C1`,`S2` |
| `B3-browser` | 整个 `agine/client/**` production cutover | client logic/frontend/E2E helpers和 mock；不写 `shared-client/**` | `C1`,`X2`,`S2` |
| `V4-combined` | 无 production/test 写入；只整合、运行 gate、记录 evidence | combined receipt/evidence owner | 所有 leaves |

同一个 checker 文件只归一个 leaf；不能让“service cleanup”和“Host job”两个 agent同时改
architecture inventory。`O2-old-ws` 必须在 S2 Host contract落地后删除旧 Host adapter，避免用
compatibility shim 掩盖缺口。

### 9.2 最小正/负探针

1. manifest/API：恰好一个 connect-only WebSocket entry；无 `routes`、`operation`、receive、
   API websocket callable；
2. Router：browser 与 Host text、binary data frame 各自得到 `1003`，service side effect 为零；
   protocol ping/pong control frame仍可用，JSON `ping` 同样 `1003`；
3. downlink：一次 browser session connect 后收到 chat event；一次 Host connect 后收到
   current-directory/file job event，payload 无 transport correlation；
4. HTTP：所有 existing/new Agine route 对四 alias 400 fail closed且不 echo；2xx direct body
   没有 envelope；
5. Host auth：正确 credential/lease 的 hello、presence、attempt、result 正向；缺失、重复、
   冲突 header、旧 lease、foreign host、wrong attempt 全部负向；
6. activation：按 D0 冻结的首次、retry、expiry、重复 ack、持久化后重启序列；
7. Host file：正确 owner/kind/Host/job terminal；foreign user/tab/host、wrong kind、expired、
   duplicate result、poll after completion/cancel 的正负结果；
8. canonical gates：service workflow/architecture/isolated tests、client type/logic/frontend、
   Host type/unit/architecture/package-boundary，最后 chat smoke 与 combined Host-file smoke。
   浏览器 workload 使用 worktree dynamic launcher，不占固定端口。

### 9.3 反搜 gate

不能使用 repo-wide “`requestId` 必须为零”的粗暴 gate。combined owner 应输出分类清单：

- `agine/**` production 正向 bare correlation 必须为零；
- `agine_transport.hasForbiddenHttpCorrelationField` 与 explicit HTTP negative tests/checkers/docs
  可保留四 alias；
- `shared-client/.../AdminPurchaseList.tsx` 是 `KEEP-3P`；
- shared generic request/error machinery由非 Agine active callers allowlist，另有 gate 证明 Agine
  未 import/instantiate其 request/send surface；
- Skiff Router↔Runtime/control/actor `request.start.requestId` 是内部 wire，保留；
- `packages/agent` 的 `AgentChildAgentRequest.id` local variable 命名属于业务 resource owner，
  scope 外保留；
- `chatId`、`runId`、`toolCallId`、`attemptId`、`jobId` 等明确业务 identity 不进入删除 regex。

## 10. 审计命令与状态

执行的 workload 只有只读文件/源码检查与本 result 的文档提交。关键命令：

```bash
git rev-parse HEAD
git rev-parse HEAD^{tree}
git status --short --branch

rg --count-matches 'requestId|request_id|correlationId|correlation_id' \
  agine/service agine/protocol agine/client agine/host shared-client
rg -n 'request_id|correlationId|correlation_id' \
  agine/service agine/protocol agine/client agine/host shared-client
rg -n 'requestId|request_id|correlationId|correlation_id' agine/docs
rg -n 'client data frames are not supported|connection\.send|request\.start' router/src std
rg -n 'host/(hello|activation-ack|ping|tool-attempts|current-directory|files)' \
  agine/service agine/client agine/host
rg -n '\.(request|send)\(' shared-client agine/client agine/host
```

没有运行 test/type-check：本 leaf 是静态 owner audit且没有 production change；contract
明确禁止 build/dev/start/stable/live/fixed-port workload。提交前只运行
`git diff --cached --check` 与两个 worktree 的 status 检查。

最终预期/已核对状态：

```text
Internals: ## codex/p5-f436b-agine-request-id-audit
Skiff:     ## codex/p5-f436b-agine-request-id-audit
```

即 Internals 零修改；Skiff 只包含本 result commit，提交后 worktree clean。没有 merge、
rebase 或 push，也没有承接 repair。
