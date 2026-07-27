# P5-F425D Agine ordinary user HTTP service checkpoint result

状态：PASS。F425D source checkpoint完成；canonical source execution仍被Agine之前的AIHub
expression-type基线阻塞，未伪报为动态通过。

## Exact candidate

- Internals base：commit `eddeeb8615057233a8a9ba2fbcf748d863d23e3b`、tree
  `b587fc9a7d2a7916d86c01533955955c43b9ac85`
- Internals implementation：commit `36efa71fe308f0b90dbd2720d1122fa4f74045f8`、tree
  `ee86edba3cff241a0c9613878184a0f726bea89d`
- Skiff task contract input：commit `4a91a052b95c2fe254c3e062e9c170aaa5f85cfa`、tree
  `8b826e45bd227fda973764d6c5d63b5b3fc629d4`

实现只修改leaf授权的Agine service/protocol/receipt/test文件。`api.yml`不需要改变；raw HTTP
gateway不是新的service operation，package API仍只有`handleAgineHttp`与`websocket`。

## HTTP surface与business owner

在原14条raw HTTP entry之后新增精确22条literal POST entry，总数为36：

| HTTP path | typed payload（除共同的`requestId?`） | 唯一business owner |
| --- | --- | --- |
| `/chat/update` | `chatId,title?,webSearchEnabled?` | `thread_store.updateThread` |
| `/chat/update_model` | `chatId,providerId,modelId,reasoningLevel?` | `thread_store.updateThreadModel` |
| `/chat/pin` | `chatId,pinned` | `thread_store.pinThread` |
| `/chat/delete` | `chatId` | `thread_store.deleteThread` |
| `/chat/stop` | `chatId,runId?` | `agent_bridge.stopThread`；保留但忽略`runId` |
| `/chat/regenerate` | `chatId` | `thread_store.isOwnedThread`后保持既有固定语义 |
| `/chat/usage` | `chatId` | `thread_store.getUsage` |
| `/chat/move-tool-to-background` | `chatId,runId?,toolCallId` | `tool_result_adapter.moveToolToBackground` |
| `/agents/list` | 无额外字段 | `agent_runtime.listAgents` |
| `/agents/hidden-list` | 无额外字段 | `agent_runtime.listHiddenAgents` |
| `/agents/create` | `agent: JsonObject` | `agent_runtime.saveAgent` |
| `/agents/update` | `agent: JsonObject` | `agent_runtime.saveAgent` |
| `/agents/delete` | `agentId` | `agine_agent_commands.deleteOwnedAgent` |
| `/agents/reset` | `agentId` | `agent_runtime.resetAgent` |
| `/agents/unhide` | `agentId` | `agent_runtime.unhideAgent` |
| `/provider/list` | 无额外字段 | `provider_runtime.listAvailableProviders` |
| `/toolproviders/list` | 无额外字段 | `host_toolprovider_runtime.listToolProviders` |
| `/toolproviders/remove` | `toolProviderId` | `host_toolprovider_commands.removeToolProvider` |
| `/toolproviders/rename` | `toolProviderId,name` | `host_toolprovider_runtime.renameToolProvider` |
| `/toolproviders/current-directory` | `toolProviderId` | `currentDirectoryForToolProvider` |
| `/thread/toolproviders/list` | `chatId?,threadId?` | `listThreadToolProviders` |
| `/tool_call/result` | client-only tool-result fields，`executor:"client"` | `tool_result_adapter.onClientToolResult` |

manifest、literal route table、Skiff API payload、TypeScript protocol registry与receipt共同锁定
`36 total / 22 ordinary-user / 36 unique`。HTTP adapter不decode或转发legacy WebSocket envelope。

## 关键语义与边界

- 所有新增route复用既有resolve、POST method、cookie session guard/read及统一HTTP
  success/error envelope。session读取之后拒绝顶层`eventName`与caller/transport identity字段。
- `chat/regenerate`对foreign/missing chat返回`not_found`，对owned chat精确返回
  `not_implemented`和`Chat regenerate is not implemented`。
- agent delete三步顺序只存在于`agine_agent_commands.deleteOwnedAgent`：
  resolve owned agent、删除projected threads、删除resolved agent。HTTP与现有WS adapter各调用一次。
- `/tool_call/result`只接受cookie user与literal `executor:"client"`；显式非client executor返回
  `invalid_executor`，`Authorization`或`X-Agine-Host-Activation`与cookie并存时fail closed。该文件不构造
  `HostCoordinator`，只调用`onClientToolResult`。
- HTTP route table显式不含`/tools/list`、`/thread/toolproviders/add`、
  `/thread/toolproviders/remove`、host-file list/search或任何`/host/*`。
- `/toolproviders/current-directory`只复用既有owner的Host downlink refresh，不改变Host auth、
  activation、uplink或host-file决策，因此没有`TASK_SCOPE_EXPANDED`。

WebSocket manifest从`websocket:\n`到EOF仍为92 bytes，SHA-256为
`a8640c0e5be799cbf5f1c815c23ab30b371ca62b911423ab64f8842a68984a29`，与base byte-identical。
唯一legacy WS source改动是任务合同明确允许的agents/delete机械共享函数替换；envelope、response event、
request decode、actor gate及其它event均未改变。

## 验证

通过：

- `npm run test:workflow-guards`：37/37
- 聚焦receipt + Agine architecture + Host architecture：22/22
- `npm run test:architecture`：2个architecture checker均通过
- Node syntax checks：3/3
- `agine/protocol/http.ts`独立TypeScript check：1/1
- changed Skiff source parser probe：14/14 files
- `git diff --check`：PASS
- source test inventory：171 declarations / 33 files；本leaf新增11项

新增的11项source test覆盖chat、agent/provider、toolprovider每组正例、cross-owner与malformed
payload，以及missing/invalid session、body identity伪造、Host/cookie冲突、Host executor和client tool
result。wrong method与unknown path继续由既有dispatch test覆盖。

真实discovery：

1. 任务列出的精确命令
   `SKIFF_ROOT=... npm run test:service-api`在`agine/service`立即失败，因为
   `@agine/service`没有`test:service-api` script；`agine/service/package.json`不在leaf写入范围。
2. 实际canonical driver
   `SKIFF_ROOT=... npm run type-check --workspace @agine/service`在进入Agine前失败：

   ```text
   internal.aihub_service: return object literal field `event` has no resolved expression type
   internal.provider_catalog: return object literal field `reasoningLevels` has no resolved expression type
   internal.provider_catalog: return object literal field `reasoning_levels` has no resolved expression type
   ```

   因此171项Skiff source tests只记录discovery与14-file parser证据，不声明动态执行通过。
3. 补充尝试当前`skiff test`隔离runner也未进入package compile：该Skiff result worktree没有Router
   `node_modules`，Router因缺少`yaml`退出。隔离进程已停止，无stable/live使用。

## 自验收

| 要求 | 结果 |
| --- | --- |
| 精确新增22条POST并形成36-entry闭集合 | PASS |
| literal route、typed payload、session identity与统一HTTP envelope | PASS |
| 22条operation各调用唯一F424B business owner | PASS |
| regenerate既有`not_found`/`not_implemented`语义 | PASS |
| agents/delete唯一共享三步business function，WS机械等价 | PASS |
| browser/client-only tool result及Host冲突fail closed | PASS |
| dead operation、host-file与`/host/*`反向排除 | PASS |
| WebSocket manifest tail byte-identical | PASS |
| completion-test source coverage | PASS；动态execution被pre-Agine AIHub基线阻塞 |
| 写入范围、无Host/host-file扩张 | PASS |

Internals implementation提交后worktree clean。Skiff result提交前worktree clean。未merge、rebase、
push，未访问stable instance、live endpoint或真实provider。
