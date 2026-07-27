# P5-F424B Agine HTTP uplink owner audit result

状态：`TASK_SCOPE_EXPANDED`（只读审计完成；实现被三个协议/安全决策阻塞）。

## 审计基线与边界

| Repo / worktree | Branch | Exact commit | Exact tree | 状态 |
| --- | --- | --- | --- | --- |
| Skiff任务worktree `/Users/geek/workspace/skiff-p5-f424b-agine-audit` | `codex/p5-f424b-agine-audit` | `4b77ed1f6bfeb3deb7f3364981cf06de6e47a522` | `ec8cd55a377e01b37a107301b3707322c64b7921` | 审计开始时clean；该commit是下述冻结输入的后代，只增加F424任务文档 |
| Skiff冻结设计/工具链输入 | — | `ba74febaca5dbe8f2b55d6db04e0544a6758bf4b` | `7ac91495f85bbf997fe4f57ddfbec76b82cc753c` | `git merge-base --is-ancestor`确认是任务worktree祖先 |
| Internals `/Users/geek/workspace/internals-phase-05-integration` | `codex/package-service-phase-05` | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` | 审计开始时clean，审计后仍clean |

本任务只读检查了：

- `agine/service/**`
- `agine/client/**`
- `agine/host/**`
- Agine相关package manifests、scripts、tests和receipts
- Skiff冻结设计、当前`std.websocket`形状和test CLI入口

唯一写入是本文档。未修改Internals production、test或fixture；未运行stable/live/instance、浏览器、
本地服务或完整测试；未merge、rebase或push。

## 结论

`agine_ws_dispatch`及三个下游dispatcher一共接受**35个**不同的业务`eventName`。现有14个HTTP
entry与这35项的交集为**0**：已有HTTP chat create/get/send等操作不能替代本清单中的
update/pin/delete/stop等操作。

35项可互斥分为：

- 19项已有内部业务函数、只缺HTTP route/adapter；
- 2项只有WebSocket层编排，需要先提取共享业务handler；
- 9项整体被认证、连接绑定或响应目标决策阻塞；
- `tool_call/result`一项同时含可直接迁移的browser分支和被阻塞的Host分支；
- 4项应删除而不是迁移：浏览器应用层`ping`、无生产caller的`tools/list`、
  `/thread/toolproviders/add`、`/thread/toolproviders/remove`。

合计10个eventName含受阻语义：`tool_call/result`、两项browser host-file请求、两项Host
file-result和5项Host lifecycle。它们归结为三个不能由本审计自行决定的问题。

不能直接开始“删receive、把caller改HTTP”，原因如下：

1. **Host current-connection授权证明缺失。** 当前Host请求不只验证长期`agh_*`credential，还要求
   WebSocket context中的`hostConnectionId`等于`ToolProvider.activeConnectionId`。改成只验证长期
   credential会让已被替换的旧Host进程继续发heartbeat、tool result、file result和attempt snapshot，
   改变现有安全边界。HTTP没有可证明“我是当前WebSocket连接”的值。
2. **activation的两阶段证明依赖WebSocket context。** activation connect把临时签发的Host ID和
   activation token hash绑定进connection context；`host/hello`取回临时Host ID，Host落盘并切换
   header后，`host/activation-ack`仍依赖原连接的token hash消费token。HTTP必须重新定义hello和ack
   分别提交什么证明、何时消费token、怎样防重放。
3. **browser host-file响应的精确tab目标缺失。** 当前请求把发起browser的exact
   `connectionId`写入relay记录，Host结果只回到该连接。HTTP cookie只能恢复user/session
   `businessIdentity`，无法恢复发起tab的WebSocket connection。广播到同用户所有tab、增加
   client-instance到connection映射、或把file RPC改成同步/轮询HTTP，行为和隐私语义不同。

因此本任务返回`TASK_SCOPE_EXPANDED`。非Host、非host-file的机械HTTP迁移可以在决策前单独开发，
但Agine整体cutover和删除receive必须等三个决策冻结。

## 1. 完整accepted-event矩阵

分类：

- `R`：已有内部业务函数可复用，只缺HTTP route/adapter。
- `X`：业务编排只存在于WebSocket handler，需要先提取共享handler。
- `B`：业务函数存在，但认证、连接绑定或响应目标语义被上述决策阻塞。
- `D`：无应保留的业务HTTP操作，删除发送点/handler。
- `T`表示存在直接走WebSocket RPC的E2E/helper；普通unit fixture不算独立产品producer。

所有行的“已有HTTP覆盖”均为“无”。响应event名称仅是当前WebSocket RPC envelope；迁移后普通
request/response应由HTTP response承载，不应继续作为WebSocket下行。

### 1.1 Dispatcher与chat

| # | Accepted `eventName` | Production / test producer | 实际业务效果 | 分类与最小迁移 |
| ---: | --- | --- | --- | --- |
| 1 | `ping` | Browser：`shared-client/shared/lib/enhanced_websocket.ts`每5秒发送JSON data frame；Host没有发送此event，`agine/host/src/shared/enhanced_websocket.ts`使用真正的WebSocket control-frame `.ping()` | 返回`pong`，没有业务状态 | `D`。Agine browser必须禁用应用层heartbeat并依赖协议栈ping/pong；冻结设计下继续发送会被gateway以1003关闭 |
| 2 | `chat/update` | Browser：`chatActions.updateChatTitle`，payload由`lib/protocol.ts`构造 | `thread_store.updateThread(userId, input)` | `R`，新增POST `/chat/update` |
| 3 | `chat/update_model` | Browser：`messageActions.updateChatModel` | `thread_store.updateThreadModel` | `R`，新增POST `/chat/update_model` |
| 4 | `chat/pin` | Browser：`chatActions.pinChat` | `thread_store.pinThread` | `R`，新增POST `/chat/pin` |
| 5 | `chat/delete` | Browser：`chatActions.deleteChat`；`T`：chat smoke/two-host cleanup | owner校验后`thread_store.deleteThread` | `R`，新增POST `/chat/delete`；E2E cleanup一并改HTTP |
| 6 | `chat/stop` | Browser：`messageActions.stopGeneration` | `agent_bridge.stopThread(chatId, userId)` | `R`，新增POST `/chat/stop` |
| 7 | `chat/regenerate` | Browser：`messageActions.regenerate` | 先验证thread owner；存在时固定返回`not_implemented`，不存在返回`not_found` | `X`。最小等价实现是HTTP保留相同错误语义；删除UI/operation是另一个产品决定 |
| 8 | `chat/usage` | Browser：`messageActions.fetchChatUsage` | `thread_store.getUsage(userId, chatId)` | `R`，新增POST `/chat/usage` |
| 9 | `chat/move-tool-to-background` | Browser：`ToolCallCard.moveToolCallToBackground` | `tool_result_adapter.moveToolToBackground` | `R`，新增POST `/chat/move-tool-to-background` |

### 1.2 Tool、Host file、Host lifecycle和ToolProvider

| # | Accepted `eventName` | Production / test producer | 实际业务效果 | 分类与最小迁移 |
| ---: | --- | --- | --- | --- |
| 10 | `tool_call/result` | Browser：`messageActions.respondToAskUser`经`socket.send`；Host：`HostToolAttemptRuntime`经`buildToolCallResultEvent`和`GatewayClient.send` | `executor=client`调用`tool_result_adapter.onClientToolResult`；`executor=host`先要求current Host，再调用`HostCoordinator.onToolResult`结算attempt | Browser分支`R`；Host分支`B`。不能再仅按body里的`executor`选择认证。建议user和Host使用不同path，但最终拆分是待决项 |
| 11 | `/thread/host-files/list` | Browser：`hostFileApi.listHostFiles` | 校验thread mount/owner/read权限/Host在线和capability；记录exact browser/Host connection relay；下发`host/files/list-request` | `B`，新增HTTP request不能凭session恢复exact browser connection |
| 12 | `/thread/host-files/search` | Browser：`hostFileApi.searchHostFiles` | 同上，并校验query长度；下发`host/files/search-request` | `B`，与上一行共用响应目标决策 |
| 13 | `host/files/list-result` | Host：`HostRuntime`处理list request后的success/error `client.send` | 只接受current Host且relay Host connection/owner/provider/event/TTL全匹配；删除relay并只回原browser connection | `B`，Host HTTP current-connection证明缺失 |
| 14 | `host/files/search-result` | Host：`HostRuntime`处理search request后的success/error `client.send` | 与上一行相同 | `B` |
| 15 | `host/hello` | Host：WebSocket open/reconnect时`GatewayClient.sendHello` | current Host校验；刷新presence、cwd、capabilities；activation连接还返回临时Host ID | `B`，普通run与activation hello需要明确HTTP认证方案 |
| 16 | `host/activation-ack` | Host：先把hello返回的Host ID落盘、`setHostId`，再发送ack | 在**原WebSocket context**中用activation token hash与Host hash消费token并清除临时Host ID | `B`，两阶段activation协议必须重定 |
| 17 | `host/ping` | Host：激活后立即一次，随后每15秒 | current Host校验并刷新presence、cwd、capabilities；不是control-frame ping | `B`，应迁POST但必须保留current-process授权 |
| 18 | `host/current-directory` | Host：收到`host/current-directory/request`后由`HostRuntime`发送 | current Host校验并刷新当前目录 | `B` |
| 19 | `host/tool-attempts` | Host：open时一次，run模式每1秒且最多一个outstanding | current Host校验；刷新presence/cwd；接收attempt snapshot并返回execute/prune等actions | `B`。actions应成为HTTP response；其中Host `tool_call/request`是嵌套action，不是独立WS下行 |
| 20 | `/toolproviders/list` | Browser：machine settings/agent options；`T`：machine harness、two-host E2E | `host_toolprovider_runtime.listToolProviders(userId)` | `R`，新增POST同名path |
| 21 | `/toolproviders/remove` | Browser：machine settings；`T`：two-host cleanup | `host_toolprovider_commands.removeToolProvider`并刷新bridge | `R`，新增POST同名path |
| 22 | `/toolproviders/rename` | Browser：machine settings | `host_toolprovider_runtime.renameToolProvider` | `R`，新增POST同名path |
| 23 | `/toolproviders/current-directory` | Browser：machine settings，带250/750/1500ms重试 | 读cached cwd；缺失时WebSocket下发`host/current-directory/request`并返回`refreshRequested` | `R`，HTTP adapter可复用；Host的response上行仍被Host auth决策阻塞 |
| 24 | `/thread/toolproviders/list` | Browser：`threadHostBindings.listThreadHostBindings` | `host_toolprovider_runtime.listThreadToolProviders` | `R`，新增POST同名path |
| 25 | `/thread/toolproviders/add` | 审计tree内没有browser/Host/E2E production transport caller；仅server实现和tests | `host_toolprovider_commands.addThreadToolProvider` | `D`，不新增HTTP；删除前以反向搜索gate证明无外部caller |
| 26 | `/thread/toolproviders/remove` | 审计tree内没有browser/Host/E2E production transport caller；仅server实现和tests | `host_toolprovider_commands.removeThreadToolProvider` | `D`，同上 |

### 1.3 Agent与provider

| # | Accepted `eventName` | Production / test producer | 实际业务效果 | 分类与最小迁移 |
| ---: | --- | --- | --- | --- |
| 27 | `agents/list` | Browser bootstrap/settings；`T`：two-host E2E | `agent_runtime.listAgents(userId)` | `R`，新增POST `/agents/list` |
| 28 | `agents/hidden-list` | Browser bootstrap/settings | `agent_runtime.listHiddenAgents` | `R`，新增POST `/agents/hidden-list` |
| 29 | `agents/create` | Browser agent editor；`T`：chat smoke、two-host E2E | `agent_runtime.saveAgent(userId, agent)` | `R`，新增POST `/agents/create` |
| 30 | `agents/update` | Browser agent editor | 同样调用`saveAgent` | `R`，新增POST `/agents/update` |
| 31 | `agents/delete` | Browser agent editor；`T`：chat smoke/two-host cleanup | 顺序执行`resolveDeletableOwnedAgentId`、`deleteProjectedThreadsForAgent`、`deleteResolvedAgent` | `X`，先把三步编排提成唯一共享业务函数，再加POST `/agents/delete` |
| 32 | `agents/reset` | Browser agent editor | `agent_runtime.resetAgent` | `R`，新增POST `/agents/reset` |
| 33 | `agents/unhide` | Browser agent editor | `agent_runtime.unhideAgent` | `R`，新增POST `/agents/unhide` |
| 34 | `provider/list` | Browser config/bootstrap | `provider_runtime.listAvailableProviders(userId)` | `R`，新增POST `/provider/list` |
| 35 | `tools/list` | 审计tree内没有browser/Host/E2E production transport caller；仅server实现和architecture inventory | `agent_runtime.listTools()` | `D`，不新增HTTP |

## 2. Browser、Host和test-only WebSocket发送点

### 2.1 Browser production

| Source | 实际发送 |
| --- | --- |
| `shared-client/shared/lib/enhanced_websocket.ts` | `startHeartbeat()`发送应用层`ping`；Agine必须覆盖/禁用该行为，不能让connect-only socket继续发data frame |
| `agine/client/src/lib/ws.ts` | `request(eventName, data)`是所有browser WebSocket RPC的统一入口；`socketBridge.send`是无等待发送入口 |
| `agine/client/src/lib/protocol.ts` | 构造chat update/update_model/pin/delete/stop/regenerate/usage请求；调用点在store actions |
| `agine/client/src/stores/appStore/configActions.ts` | `provider/list`和7项agent操作 |
| `agine/client/src/stores/appStore/chatActions.ts` | `chat/pin`、`chat/update`、`chat/delete` |
| `agine/client/src/stores/appStore/messageActions.ts` | `chat/regenerate`、`chat/stop`、`chat/update_model`、`chat/usage`；client `tool_call/result`用`socket.send` |
| `agine/client/src/components/ToolCallCard.tsx` | `chat/move-tool-to-background` |
| `agine/client/src/lib/toolproviderApi.ts` | `/toolproviders/list/remove/rename/current-directory` |
| `agine/client/src/lib/threadHostBindings.ts` | `/thread/toolproviders/list` |
| `agine/client/src/lib/hostFileApi.ts` | `/thread/host-files/list/search` |

未找到production browser发送`tools/list`或`/thread/toolproviders/add/remove`。`MockApp`中的case和emit
是前端mock协议，不是production WebSocket producer。

### 2.2 Host production

| Source | 实际发送与触发 |
| --- | --- |
| `agine/host/src/GatewayClient.ts` | open/reconnect发送`host/hello`；activation发送`host/activation-ack`；presence发送`host/ping`；attempt poll发送`host/tool-attempts`；generic `send`负责其余payload |
| `agine/host/src/HostRuntime.ts` | hello成功后启动15秒presence与1秒attempt poll；响应`host/current-directory/request`发送`host/current-directory`；响应两项file request发送对应result；把tool attempt执行结果送给client |
| `agine/host/src/HostToolAttemptRuntime.ts`、`protocol/toolCall.ts` | 构造并发送`executor=host`的`tool_call/result` |
| `agine/host/src/shared/enhanced_websocket.ts` | 使用Node `ws.ping()` control frame；这是冻结设计允许的协议栈心跳，不映射到server `eventName=ping` |

所有Host data-frame send都必须改HTTP。Host WebSocket最终只保留接收
`host/current-directory/request`和两项host-file request，以及协议control frames。

### 2.3 Test-only direct transports

| Source | 当前直接WebSocket RPC | 迁移要求 |
| --- | --- | --- |
| `agine/client/e2e/api.chat-smoke.mjs`、`support/chat-smoke-cleanup.mjs` | `agents/create`、`chat/delete`、`agents/delete` | 改用同一HTTP helper；WebSocket只用于观察chat stream |
| `agine/client/e2e/support/cookie-websocket-rpc.mjs` | 泛型cookie WebSocket RPC及其timeout/close tests | 删除request半边或改造成纯downlink observer；HTTP retry另设helper |
| `agine/client/e2e/support/machineHarness.ts` | `/toolproviders/list` | 改HTTP |
| `agine/client/e2e/system.two-hosts.e2e.ts` | `/toolproviders/list`、`agents/list/create`以及`chat/delete`、`/toolproviders/remove`、`agents/delete` cleanup | 改HTTP；不能用测试helper延长legacy receive寿命 |
| `agine/client/e2e/support/mockApp.ts` | 浏览器内mock request/emit | 可保留业务mock，但应改为mock HTTP uplink与独立downlink emitter |

## 3. 现有HTTP覆盖与最小新增route

### 3.1 14个existing entries

现有`service.yml`有且只有下列14个named `rawHttp` POST entry，全部指向
`internal.agine_service.handleAgineHttp`：

```text
/session
/track
/chat/list
/chat/create
/chat/get
/chat/llm-call
/chat/send
/hosts/activation-token
/provider/credential/save
/provider/credential/delete
/provider/chatgpt-plan/oauth/start
/provider/chatgpt-plan/oauth/session
/provider/chatgpt-plan/oauth/cancel
/provider/chatgpt-plan/disconnect
```

`agine_http_dispatch`先按literal path解析，强制POST，执行`httpSession.guard/read`，构造
`subjectKind=user`且`connectionId=null`的`ConnectionContext`，再分发给chat/provider handler。
这些entry没有一个执行上述35项WS业务；`/hosts/activation-token`只由已登录user创建token，并不处理
Host activation hello/ack。

### 3.2 最小机械route组

沿用当前“literal POST path、一个facade、typed payload、统一HTTP envelope”的约定，并保持原
`eventName`拼写以减少映射，非阻塞部分至少需要：

| Route组 | 新增POST paths | 建议source owner |
| --- | --- | --- |
| Chat（8） | `/chat/update`、`/chat/update_model`、`/chat/pin`、`/chat/delete`、`/chat/stop`、`/chat/regenerate`、`/chat/usage`、`/chat/move-tool-to-background` | 扩展`internal/agine_http_chat.skiff`；regenerate和agent delete式编排先提共享业务函数 |
| Agent/provider catalog（8） | `/agents/list`、`/agents/hidden-list`、`/agents/create`、`/agents/update`、`/agents/delete`、`/agents/reset`、`/agents/unhide`、`/provider/list` | 新建`internal/agine_http_agent_provider.skiff`，避免继续增大现有provider credential owner |
| ToolProvider user API（5） | `/toolproviders/list`、`/toolproviders/remove`、`/toolproviders/rename`、`/toolproviders/current-directory`、`/thread/toolproviders/list` | 新建`internal/agine_http_tool_providers.skiff` |
| User tool/file API（3） | `/tool_call/result`、`/thread/host-files/list`、`/thread/host-files/search` | 新建`internal/agine_http_user_tools.skiff`；后两项在响应目标决策前不可完成 |
| Host lifecycle/file API（7） | `/host/hello`、`/host/activation-ack`、`/host/ping`、`/host/current-directory`、`/host/tool-attempts`、`/host/files/list-result`、`/host/files/search-result` | 新建`internal/agine_http_host.skiff`；把connect/HTTP共用的header parser提到`internal/agine_host_auth.skiff`，不能复制两套；整个组被Host auth/activation决策阻塞 |
| Host tool result（条件新增1） | 建议候选`/host/tool_call/result` | 若决定将user/Host认证面物理分开，由`agine_http_host.skiff`拥有；若决定单route双认证，则仍是`/tool_call/result`，但必须冻结冲突header和auth precedence |

这意味着：

- 删除4项dead行为后有31个live operation；
- 单一双认证tool-result path时需要31个新entry；
- 将Host tool result与user path分开时需要32个新entry，这是更清晰的候选，但本审计不能替安全设计作决定；
- 不应新增一个接受任意`eventName`的generic HTTP endpoint，否则只是把legacy envelope换了transport，
  会绕过当前literal route、typed payload和静态entry owner约定。

新HTTP payload应仿照已有`ChatCreatePayload`/`ChatSendPayload`：body不要求`eventName`，adapter显式转换为
内部command/input；可继续携带可选`requestId`用于diagnostic/response correlation。不能让HTTP handler
直接decode旧WebSocket envelope来“复用协议”。

`service.yml`、`agine_http_routes.skiff`、`api/agine.skiff`和architecture receipt中的entry集合必须由
一个串行checkpoint同步更新，避免出现manifest、route table、payload schema三套不同清单。

## 4. Host认证、identity与activation事实

### 4.1 当前connect建立的事实

`agine_connect.skiff`在upgrade时：

- 从singular `Authorization: AgineHost <agh_*>`或
  `X-Agine-Host-Activation: <agha_*>`解析Host认证；
- 拒绝duplicate、错误scheme/prefix、逗号和空白；
- 两种header同时存在时当前明确选择Host ID；
- Host ID分支调用`authenticateHostConnection(hostId, request.connectionId)`；
- activation分支调用`activateHostConnection(token, request.connectionId, query.hostName)`；
- 返回`businessIdentity = "host:" + sha256(hostId)`；
- 返回`maxConnections=1`、`overflow=close-oldest`、close code `4009`、
  reason `host connection replaced`；
- browser分支把upgrade转换为GET `/ws`交给`httpSession.guard/read`，以`session.id`作为
  `businessIdentity`，无connection policy，并记录`session.id -> connectionId`供chat stream使用。

冻结设计的目标connect result只有accept/reject、可选`businessIdentity`和`connectionPolicy`，**没有
user-defined connection context**。connect request仍有`connectionId`、query、headers、cookies和entry
identity。因此connect阶段的cookie/Host header校验、DB side effect、browser connection记录以及opaque
business identity/policy都能保留；`ConnectionContext.hostConnectionId`、
`hostActivationTokenHash`等事实不能再由后续业务消息隐式继承。

### 4.2 current Host gate不是普通credential check

`host_toolprovider_connection.currentHostToolProvider`同时要求：

```text
context.subjectKind == host
context.businessIdentity == actorSubjectId
context.hostIdHash matches credentialRef / metadata
context.hostConnectionId is non-null
ToolProvider is active, present and online
ToolProvider.actorSubjectId == actorSubjectId
ToolProvider.activeConnectionId == context.hostConnectionId
```

Host lifecycle、attempt sync、Host tool result和Host file result都直接或间接依赖这组条件。connect
replacement会更新`activeConnectionId`，旧socket因此不再是current actor。仅把`agh_*`放进每个HTTP
request只能证明“持有长期Host secret”，不能证明“请求来自当前获胜连接/进程”。

### 4.3 activation的现有两阶段状态机

1. 已登录browser通过`/hosts/activation-token`创建5分钟`agha_*`token。
2. Host用activation header upgrade。connect transaction将token从`pending`推进到`issued`，生成
   临时明文`agh_*`、保存hash和当前connection ID，并创建/刷新ToolProvider。
3. 同一connection上的`host/hello`通过context中的token hash和Host hash读取临时明文Host ID。
4. Host把ID落盘，调用`GatewayClient.setHostId`只更新后续连接header，然后仍在当前connection发送
   `host/activation-ack`。
5. ack用当前context中的activation token hash消费token，并清除临时明文。

HTTP版不能假定第4步的`Authorization`自动带有旧connection的activation token。必须明确ack携带
activation token、Host credential、一次性receipt或其它证明中的哪一种，以及失败/超时/重试/重放行为。

## 5. 必须由上层决定的精确问题

### D1. Host HTTP current-process认证

必须从以下协议族中选定一种，而不是由实现leaf猜测：

- 长期`agh_*`credential即为全部HTTP授权；这会明确放弃current-connection隔离并改变安全边界；
- connect签发短期、connection-generation绑定的HTTP bearer/nonce，Host每次POST携带，replacement时
  轮换/撤销；
- HTTP请求显式携带可验证的connection binding，由平台/gateway或Agine app owner签发；
- 不再把Host业务授权与WebSocket liveness绑定，改用另一套session/lease模型。

决策还必须冻结credential与activation header同时出现时的precedence、token TTL/rotation、重放、
旧进程、断线期间HTTP请求和错误码。

### D2. activation hello/ack

必须决定：

- activation hello是用`agha_*`直接POST，还是connect先返回/签发一次性receipt再POST；
- 临时Host ID在hello response中何时可读、何时清除；
- ack提交activation token、Host ID、两者还是connect receipt；
- hello/ack重试、token过期、Host落盘失败、ack丢失和重复ack如何收敛；
- activation期间的WebSocket是否已经使用最终`businessIdentity`并受max-one policy管理。

### D3. browser host-file response target

必须选择并冻结：

- 保持exact-tab语义：在WebSocket connect query中提交browser `clientInstanceId`，由server建立
  `(session, clientInstanceId) -> connectionId`映射，HTTP file request携带同一ID；
- 改为对user `businessIdentity`广播，接受多tab可见和重复消费；
- file RPC改为同步长HTTP、server stream、job+poll或其它HTTP结果通道，不再异步回WebSocket。

当前client已有sessionStorage级`getClientInstanceId()`用于chat HTTP dedupe，但connect URL没有携带它，
service也没有这张映射；不能把它当成已经存在的事实。

### D4. `tool_call/result`认证面

这是D1的route级子决策：user和Host应使用两个entry，还是一个entry按认证header选择actor。不得只相信
payload `executor`，也必须定义cookie与Host header同时出现时的fail-closed行为。

## 6. 下行、producer与真实consumer

### 6.1 移除receive后仍应保留的真实异步WebSocket下行

| Downlink `eventName` | Service producer | Production consumer | 结论 |
| --- | --- | --- | --- |
| `chat/title-updated` | `conversation_title_tool.skiff` | `wsTranscriptHandlers`更新chat标题 | 保留 |
| `chat/message-added` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers`合并user/assistant消息 | 保留 |
| `chat/step-start` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers`建立run/draft状态 | 保留 |
| `chat/step-finish` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers`更新model/usage | 保留 |
| `chat/text-delta` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers`追加文本 | 保留 |
| `chat/reasoning-delta` | `agent_bridge_event_projection.skiff` | `wsTranscriptHandlers`追加reasoning | 保留 |
| `chat/run-completed` | `agent_bridge_event_projection.skiff` | `wsRunHandlers`收敛成功终态 | 保留 |
| `chat/run-failed` | `agent_bridge_event_projection.skiff` | `wsRunHandlers`收敛错误终态 | 保留 |
| `chat/run-stopped` | `agent_bridge_event_projection.skiff` | `wsRunHandlers`收敛停止终态 | 保留 |
| `host/current-directory/request` | `host_toolprovider_connection.skiff` | `HostRuntime`读取cwd，再以HTTP上行结果 | 保留Host下行 |
| `host/files/list-request` | `host_file_rpc.skiff` | `HostRuntime`/`HostService`执行目录读取 | 保留Host下行 |
| `host/files/search-request` | `host_file_rpc.skiff` | `HostRuntime`/`HostService`执行搜索 | 保留Host下行 |

chat stream在browser connect时调用`rememberConnection(session.id, connectionId)`，并按thread owner找到最多
8个记录逐连接发送。这个记录动作已经在connect路径，不依赖receive；目标connect-only handler仍可执行
它。Host三项下行分别按business identity或保存的current Host connection发送，也不要求当前执行起源是
WebSocket；冻结的`std.websocket`允许HTTP、actor或service activation主动下发。

### 6.2 被D3阻塞的异步browser下行

`/thread/host-files/list-response`和`/thread/host-files/search-response`当前由
`host_file_rpc.skiff`发往relay记录里的exact browser connection，consumer是
`EnhancedWebSocket.request()`的requestId correlation。若D3选择保持异步WS响应，它们仍是合法真实
downlink；若改同步/轮询HTTP，则应删除这两个WS event。审计不能预选。

### 6.3 应改成HTTP response、不能继续占用WebSocket的消息

| 当前WS response event(s) | Producer | 当前真实consumer | Cutover |
| --- | --- | --- | --- |
| `pong` | `agine_ws_dispatch` | shared browser heartbeat的`pong`listener | 连同应用层ping删除 |
| `chat/update-response`、`chat/update_model-response`、`chat/pin-response`、`chat/delete-response`、`chat/stop-response`、`chat/regenerate-response`、`chat/usage-response`、`chat/move-tool-to-background-response` | `agine_ws_chat` | browser `EnhancedWebSocket.request()`按eventName+requestId correlation | HTTP response |
| `agents/list-response`、`agents/hidden-list-response`、`agents/create-response`、`agents/update-response`、`agents/delete-response`、`agents/reset-response`、`agents/unhide-response`、`provider/list-response`、`tools/list-response` | `agine_ws_agent_provider` | browser generic request correlation；`tools/list`无production request | live项改HTTP；dead项删除 |
| `/toolproviders/list-response`、`/toolproviders/remove-response`、`/toolproviders/rename-response`、`/toolproviders/current-directory-response`、`/thread/toolproviders/list-response`、`/thread/toolproviders/add-response`、`/thread/toolproviders/remove-response` | `agine_ws_tool_providers` | browser generic request correlation；thread add/remove无production request | live项改HTTP；dead项删除 |
| `tool_call/receipt`（browser identity） | `agine_ws_host_tool_files` | browser `respondToAskUser`使用无等待`socket.send`，production没有receipt listener | 改为user HTTP response；调用方至少处理HTTP失败 |
| `host/hello-response`、`host/activation-ack-response`、`host/ping-response`、`host/tool-attempts-response` | `agine_ws_host_tool_files` | `HostRuntime`显式listener；attempt response含actions | Host HTTP response |
| `host/current-directory-response` | `agine_ws_host_tool_files` | Host发送后没有production listener | HTTP可返回ack/status，但不得保留WS response |
| `tool_call/receipt`（Host identity） | `agine_ws_host_tool_files` | `HostRuntime`交给`HostToolAttemptRuntime.handleReceipt` | Host HTTP response |
| `invalid_message_response`、`unsupported_event_response` | `agine_ws_dispatch` | 未找到production listener；不符合普通eventName+`-response` correlation | 随receive删除 |

两项`host/files/*-result`当前没有给Host发送直接ack；它们成功claim relay后产生6.2中的browser
file response。Host attempt action中的`tool_call/request`由`host_tool_action_issue.skiff`构造并嵌在
`host/tool-attempts-response.actions[].request`，不是独立WebSocket send；迁移后它继续作为
`/host/tool-attempts` HTTP response payload的一部分。

### 6.4 producer/consumer不闭合的残留

| Event | 审计事实 | 后续gate |
| --- | --- | --- |
| `chat/tool-execute-start` | `host_provider.skiff`生产；production app store没有注册listener，只有mock/E2E观察 | 在下行cleanup leaf证明是否只靠`chat/message-added`/history恢复状态；若是则删producer |
| `chat/tool-execute-finish` | `tool_result_adapter.skiff`生产；production app store明确不注册listener | 同上 |
| direct browser `tool_call/request` | client `wsToolHandlers`有listener；审计tree内未找到production service直接send，唯一生产构造是Host attempt嵌套action；`mockApp`会emit client ask_user | 反向搜索完整dependency tree；若无隐藏producer则删listener/type，ask_user继续由`chat/get` runtime projection恢复 |
| `chat/provider-switched` | client有listener；Agine production service无producer | 证明dead后删除listener |
| `chat/provider-retry-waiting` | client有listener；Agine production service无producer | 同上 |
| `machine/online_change` | machine settings有listener；Agine production service无producer | 同上；当前页面也会主动list/retry |
| `sys_socket_open` | client WebSocket library本地synthetic lifecycle event，不是server downlink | 保留本地连接生命周期即可 |
| `server_error` | shared/client generic listener；Agine service无独立producer，当前RPC error走response envelope | 随RPC删除核对是否仍有平台级producer |

## 7. 建议DAG与互斥写入范围

```text
G0  冻结D1-D4安全/协议决定
 |
 +-- C1  shared HTTP path/payload/auth contract checkpoint
 |      service.yml + route table + service API + TS protocol + architecture receipts
 |
 +-- L2a Agine user service handlers --------+
 +-- L2b browser普通HTTP callers ------------+  可并行
 +-- L2c Host service auth/handlers ----------+  G0/C1后
 +-- L2d Host client HTTP transport ----------+  G0/C1后，与L2c按冻结接口并行
 +-- L2e host-file relay/browser targeting ---+  决策D3后
 |
 +-- C3  legacy receive/dispatcher/schema/test cleanup
 |
 +-- V4  combined package tests + chat/Host/file E2E proof
```

建议后继leaf的精确写入范围：

| Leaf | 唯一写入范围 | 明确排除/串行边界 |
| --- | --- | --- |
| Contract checkpoint | `agine/service/service.yml`、`agine/service/api.yml`、`agine/service/api/agine.skiff`、`agine/service/internal/agine_http_routes.skiff`、`agine/service/internal/agine_http_dispatch.skiff`、`agine/service/internal/agine_service.skiff`、`agine/protocol/**`、`agine/service/service-api-receipt*`、`agine/service/internal/agine_service_architecture.test.mjs` | 单一owner；其它leaf不得同时改这些清单/类型 |
| User service handlers | `agine/service/internal/agine_http_chat.skiff`、新`agine_http_agent_provider.skiff`、新`agine_http_tool_providers.skiff`、新`agine_http_user_tools.skiff`及对应新/现有`.test.skiff`；新`agine_agent_commands.skiff`唯一拥有agent-delete三步编排 | 不改manifest/route/API/protocol；不改`host_toolprovider_connection`、Host auth或host-file relay |
| Browser ordinary callers | `agine/client/src/stores/appStore/{configActions,chatActions,messageActions}.ts`、`agine/client/src/components/ToolCallCard.tsx`、`agine/client/src/lib/{http,toolproviderApi,threadHostBindings,protocol,socket}.ts`及同名tests；`socket.ts`负责Agine应用层ping禁用 | 不改`hostFileApi`/file picker，不改`agine/protocol/**`，不改Host/service |
| Host service/auth | 新`agine/service/internal/agine_http_host*.skiff`、新`agine_host_auth.skiff`、`agine_connect.skiff`、`host_toolprovider_connection.skiff`、`host_toolprovider_runtime.skiff`、Host auth/settlement focused tests | G0/C1后；connect和HTTP复用同一header parser；不改browser ordinary files或shared route/schema checkpoint |
| Host caller | `agine/host/**` | G0/C1后；不改service/client/protocol。Host file response迁移由此leaf统一拥有，避免另一个leaf同时改`HostRuntime.ts` |
| Host-file browser/relay | `agine/client/src/lib/hostFileApi.ts`、file picker/browser pane及其tests；`agine/service/internal/host_file_rpc.skiff`和`host_file_browse.test.skiff` | 决策D3后；Host端改动仍归Host caller leaf |
| Legacy cleanup | `agine/service/internal/agine_ws_*.skiff`、`agine_transport.skiff`中WS RPC helpers、facade receive分支、旧WS API/context types；`agine/client/src/lib/ws.ts`的request/send surface和test-only WS RPC helpers | 所有caller已迁移且反向搜索clean后最后执行；保留connect和真实downlink |

Legacy cleanup与contract checkpoint会串行触碰facade/schema，不能作为并行leaf；表中其它L2 leaf的
production写入范围互不重叠。

`shared-client/shared/lib/enhanced_websocket.ts`还被其它产品使用，不应由Agine leaf直接全局删heartbeat；
优先在Agine `socket.ts`提供不发data-frame heartbeat的connect-only subclass/配置。若shared library本身要
增加配置，必须单设shared-client owner，不能与Agine browser leaf隐式重叠。

## 8. 验证矩阵

### 8.1 真实test discovery

后继leaf先运行下列只读发现命令，不要靠手写测试清单：

```bash
rg --files agine/service | rg '(\.test\.skiff|\.test\.mjs)$'
rg --files agine/client | rg '(\.test\.(ts|tsx|mjs)|e2e/.*\.e2e\.ts)$'
rg --files agine/host | rg '(\.test\.ts|scripts/.*\.test\.mjs)$'
node -e "for (const p of ['agine/service/package.json','agine/client/package.json','agine/host/package.json']) { const j=require('./'+p); console.log(p, j.scripts) }"
```

当前Skiff CLI的真实形状来自`skiff/scripts/skiff.mjs`：

```text
skiff test <package-root-or-file> --artifact-root <existing-dir>
  [--base-assembly <identity>] [--deny-skips] [--require-tests]
```

它没有可用的`test --help`子命令，也不接受旧的`--profile`/
`--service-artifact-root` test参数。需要service dependency时，应由Internals canonical isolated
workflow在临时artifact store中准备base assembly，再把**该临时existing store和identity**传给
`skiff test`；不得读取stable `.skiff-instance/dev-home/artifacts`。

### 8.2 聚焦命令

Contract/service architecture最早风险探针：

```bash
cd /Users/geek/workspace/<internals-implementation-worktree>
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs \
  agine/service/internal/host_runtime_architecture.test.mjs
```

Service canonical isolated验证：

```bash
SKIFF_ROOT=/Users/geek/workspace/<skiff-implementation-worktree> \
  npm run type-check --workspace @agine/service
SKIFF_ROOT=/Users/geek/workspace/<skiff-implementation-worktree> \
  npm test --workspace @agine/service
```

若只跑`.skiff` focused tests，先由同一worktree的canonical workflow建立临时artifact root/base
assembly，再运行：

```bash
node /Users/geek/workspace/<skiff-implementation-worktree>/scripts/skiff.mjs test \
  agine/service/internal/agine_service_dispatch.test.skiff \
  --artifact-root <isolated-existing-artifact-root> \
  --base-assembly <isolated-base-assembly-identity> \
  --deny-skips --require-tests
```

Host/file/tool结果还应按同样形状发现并运行：

```text
agine/service/internal/host_file_browse.test.skiff
agine/service/internal/host_tool_settlement.test.skiff
agine/service/internal/host_toolprovider_current_directory.test.skiff
agine/service/internal/host_toolprovider_rename.test.skiff
agine/service/internal/tool_result_adapter_background.test.skiff
agine/service/internal/agent_bridge.chat_config.test.skiff
```

Browser：

```bash
npm run type-check --workspace @agine/client
npm run test:logic --workspace @agine/client -- \
  src/lib/http.test.ts \
  src/lib/ws.test.ts \
  src/lib/toolproviderApi.test.ts \
  src/lib/threadHostBindings.test.ts \
  src/lib/hostFileApi.test.ts \
  src/stores/appStore/configActions.test.ts \
  src/stores/appStore/chatActions.test.ts \
  src/stores/appStore/messageActions.test.ts \
  src/stores/appStore/registerWsEvents.test.ts \
  src/components/ToolCallCard.test.ts \
  src/architecture.client-boundaries.test.ts
```

Host：

```bash
npm run type-check --workspace @agine/host
cd agine/host
npm exec -- tsx src/GatewayClient.test.ts
npm exec -- tsx src/HostRuntime.test.ts
npm exec -- tsx src/HostToolAttemptRuntime.test.ts
npm exec -- tsx src/protocol/toolCall.test.ts
npm run test:architecture
npm run test:package-boundary
```

收尾才运行`npm test --workspace @agine/host`和受影响browser E2E。worktree browser验证必须使用
`node scripts/run-web-client.mjs agine-client`动态租用`44000`–`44999`端口；Host使用临时`--home`，
不得触碰主工作区4004–4007、stable Skiff instance或真实共享Host credential。

### 8.3 必须覆盖的正负例

| 面 | 正例 | 负例 |
| --- | --- | --- |
| User HTTP | session cookie解析为同一user；每项业务返回与原WS payload等价的HTTP envelope；owner-scoped mutation成功 | missing/invalid session 401、错误method 405、unknown path 404、malformed payload 400、跨owner资源404/拒绝、body `userId`不能越权 |
| Browser connect-only | cookie connect建立`businessIdentity=session.id`；chat async events仍到达；应用层不再发送ping | 任意browser text/binary data frame导致1003；无session upgrade拒绝；不存在`socket.request/send`业务调用 |
| Host connect | 有效Host/activation header建立稳定host business identity；max-one replacement仍4009 close-oldest | duplicate/whitespace/wrong-prefix header拒绝；冲突headers按冻结决定；旧连接从fan-out移除 |
| Host HTTP | 当前获胜Host的hello/ping/attempt/result/file result成功；attempt actions只执行一次；断线/重连按决定收敛 | 被替换旧进程、错误binding、过期/重放token、错误executor、错误attempt/tool provider/relay owner、duplicate result均拒绝或幂等 |
| Activation | token签发、hello取回Host ID、Host持久化、ack消费的happy path；丢response可按冻结规则安全重试 | expired/revoked/replayed token、ack早于持久化、错误Host ID/token组合、hello或ack重复、两种auth同时提交 |
| Host file | owner/mount/capability/path校验后请求送current Host，结果只到原tab；timeout/error保留 | 跨user/mount、file-read禁止、Host offline、stale Host result、event kind错配、过期relay、另一个tab不可窃取消费 |
| Downlink | 9项chat async和3项Host request继续由WebSocket消费 | uplink response/pong不再走WS；orphan producer/listener由reverse-search gate删除 |

## 9. Legacy反向搜索gate

删除receive前，至少要求下面搜索在production范围内只剩设计允许的connect/downlink命中：

```bash
rg -n 'event\.tag\s*==\s*"receive"|receiveEvent|WebSocketIngressEvent|ConnectionMessage' \
  agine/service agine/client agine/host
rg -n 'agine_ws_|decodeWebSocketRequest|unsupported_event_response|invalid_message_response' \
  agine/service
rg -n 'socket\.(request|send)\(|\brequest\([^)]*eventName|eventName:\s*['\''"]ping['\''"]|startHeartbeat' \
  agine/client shared-client
rg -n 'CookieWebSocketRpc|browserWsRequest|wsRequest|new WebSocket|\.send\(' \
  agine/client/e2e
rg -n 'host/(hello|activation-ack|ping|current-directory|tool-attempts)|host/files/(list|search)-result|tool_call/result' \
  agine/host
rg -n 'operation:\s*websocket|websocket:\s*$|routes:\s*$' agine/service
rg -n 'context\.connectionId|hostConnectionId|activeConnectionId|hostActivationTokenHash' \
  agine/service
rg -n 'sendErrorToConnection|sendResponse\(|sendError\(|sendTextToBusinessIdentity|sendJsonToConnection' \
  agine/service/internal
rg -n 'chat/tool-execute-(start|finish)|tool_call/request|chat/provider-(switched|retry-waiting)|machine/online_change' \
  agine/service agine/client agine/host
```

`ConnectionContext`这个名字本身可能暂时继续作为HTTP内部user context，但不得再出现在WebSocket connect
result、receive signature或作为Host HTTP事实来源。反向搜索验收应检查语义位置，不能为追求字符串零命中
误删合法HTTP context。

## 10. Worktree状态与交付判断

- Internals exact input在审计期间没有变化，`git status --short`为空。
- Skiff任务worktree在写本文档前clean；本文档是唯一预期改动。
- 没有访问或修改stable/live/instance，没有运行本地服务、PM2、MongoDB、Router、Runtime、browser或
  Host进程。
- 没有merge、rebase或push。
- 可立即解除的后继工作仅限于DAG中的non-Host、non-host-file机械迁移；整体Agine receive删除与cutover
  仍被D1–D4阻塞。
