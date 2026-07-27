# P5-F425D Agine ordinary user HTTP service checkpoint

状态：Ready。中高风险consumer迁移；不包含Host与host-file。

## 直接父节点

- `P5-F425-downlink-websocket-implementation-checkpoint.md`

完整event矩阵与owner事实在父节点引用的F424B result。本文只解除明确不受Host决策影响的surface。

## DAG位置与输入

与F425A/B/C并行。精确Internals输入见父节点。完成后解除普通Agine browser caller迁移；Host、
host-file、legacy receive cleanup和N5仍阻塞。

## 写入范围

仅允许：

- `agine/service/service.yml`
- `agine/service/api.yml`
- `agine/service/api/agine.skiff`
- `agine/service/internal/agine_service.skiff`
- `agine/service/internal/agine_http_{routes,dispatch,chat}.skiff`
- 新的`agine_http_agent_provider.skiff`、`agine_http_tool_providers.skiff`、
  `agine_http_user_tools.skiff`、`agine_agent_commands.skiff`
- 上述owner直接对应的`.test.skiff`
- `agine/protocol/**`
- `agine/service/service-api-receipt*`
- `agine/service/internal/agine_service_architecture.test.mjs`
- Skiff任务repo中的本leaf result

禁止修改`agine/client/**`、`agine/host/**`、Host auth/connection owner、`host_file_rpc.skiff`、
legacy `agine_ws_*`、WebSocket block或其它service。

## 必须实现

在现有14个HTTP entry之外，新增并完整实现下列22个literal POST rawHttp operation：

```text
/chat/update
/chat/update_model
/chat/pin
/chat/delete
/chat/stop
/chat/regenerate
/chat/usage
/chat/move-tool-to-background
/agents/list
/agents/hidden-list
/agents/create
/agents/update
/agents/delete
/agents/reset
/agents/unhide
/provider/list
/toolproviders/list
/toolproviders/remove
/toolproviders/rename
/toolproviders/current-directory
/thread/toolproviders/list
/tool_call/result
```

要求：

1. 复用现有HTTP session guard/read与literal route/typed payload/统一HTTP envelope约定；body不接受
   `eventName`或caller提供的user identity。
2. 每项调用F424B矩阵确认的唯一业务owner；不能通过decode旧WebSocket envelope复用dispatcher。
3. `chat/regenerate`保持现有owner/not_found/not_implemented语义。
4. `agents/delete`把三步编排提取为唯一共享business function，再由HTTP调用；不在adapter复制事务顺序。
5. `/tool_call/result`只接受browser/user session与`executor=client`语义；Host credential、Host executor
   或冲突身份fail closed。Host以后使用独立`/host/tool_call/result`。
6. 不新增`tools/list`、`/thread/toolproviders/add`或`/thread/toolproviders/remove`，因为没有production
   caller。
7. manifest、route table、API/protocol与receipt的36-entry清单由本leaf同步；WebSocket legacy block保持
   byte-identical，后继cleanup再删除。
8. 不实现browser host-file list/search或任何`/host/*` operation。

## 完成标准与验证

测试至少覆盖每组正例、missing/invalid session、wrong method、unknown path、malformed payload、
cross-owner资源、body伪造user id、Host/cookie冲突及Host executor拒绝。反向证明三项dead operation没有
新增HTTP entry。

运行实际匹配的Agine source/service聚焦测试与receipt source test；current Skiff会在保留的legacy
WebSocket authoring处停止，不能为通过测试提前删除该block或修改Skiff。记录真实discovery：

```bash
SKIFF_ROOT=<assigned-skiff-worktree> npm run test:service-api
git diff --check
```

如果范围内无法形成完整可编译checkpoint，或发现某项实际依赖Host/host-file决策，停止并返回
`TASK_SCOPE_EXPANDED`，不得把它偷偷纳入本leaf。

## 交付

在Internals提交implementation；在Skiff任务worktree新增并提交
`P5-F425D-agine-user-http-service-checkpoint-result.md`。返回两个commit/tree、自验收矩阵与clean
状态。不得merge/rebase/push/stable/live。

