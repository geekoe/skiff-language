# P5-F428A Agine HTTP direct-body service checkpoint

状态：Ready。高风险 HTTP wire checkpoint。

## 直接父节点

- `P5-F427A-agine-http-correlation-owner-audit-result.md`

父节点已记录 production owner、36 条 HTTP route、旧 WebSocket 复用边界、F426C WIP
可复用范围及验证矩阵，并继续引用到唯一权威设计。启动时只读本任务；实现需要时再沿父节点引用向上查阅。

## DAG 位置与精确输入

本节点依赖 F427A 审计结论，必须先于 Agine browser caller checkpoint。完成后解除：

```text
F428A service/protocol direct body
  -> corrected Agine browser HTTP cutover
  -> Agine combined probe
```

Internals production 输入为
`ed5d333b2406d5375fca8acc96f4695667c48ced`；Skiff production 证据锚定
`95efdf357a647d549bac047f5d301905df843dd3`。后续 task-only dispatch commit 不改变这些
production 事实。当前只是实现检查点，不是稳定候选。

## 写入范围

只允许修改父节点第 8 节 A 所列 owner：

- `agine/protocol/http.ts`
- `agine/service/api/agine.skiff`
- `agine/service/internal/agine_transport.skiff`
- `agine/service/internal/agine_http_dispatch.skiff`
- `agine/service/internal/agine_http_chat.skiff`
- `agine/service/internal/agine_http_provider.skiff`
- `agine/service/internal/agine_http_agent_provider.skiff`
- `agine/service/internal/agine_http_tool_providers.skiff`
- `agine/service/internal/agine_http_user_tools.skiff`
- 为拆分 transport-neutral command 确实必需的
  `thread_store.skiff`、`tool_result_adapter.skiff`、`agine_ws_chat.skiff`、
  `agine_ws_host_tool_files.skiff`
- 上述 HTTP owner 的直接 `*.test.skiff`、service API receipt/architecture checker 与 Agine
  contract README
- Skiff task worktree中的本任务 result

禁止修改 `agine/client/**`、`agine/host/**`、其它 Internals service、Skiff production 或
skiff-packages。若 direct body 必须修改未列出的 production owner，返回
`TASK_SCOPE_EXPANDED`。

## 必须实现

1. 36 条普通 HTTP route 的 request 只接受父节点矩阵中的业务字段；删除 28 个 schema
   `requestId` hit、raw `requestIdFromBody` 及任何 HTTP correlation echo。
2. HTTP 对 `requestId`、`request_id`、`correlationId`、`correlation_id` 必须 fail closed，
   不能依赖当前 decoder 静默忽略 unknown field。
3. 34 条当前使用 WS envelope 的 route 改为：
   - 2xx 直接返回业务对象，无结果时为 `{}`；
   - 非 2xx 返回 `{"error":{"code":"...","message":"..."}}`；
   - 不含 transport `eventName`、`requestId`、`ok` 或 outer `payload`；
   - `/chat/llm-call` 的业务字段 `payload` 必须保留；
   - 405 继续带 `Allow: POST`。
4. HTTP adapter 不得再构造 legacy `*Input(eventName, requestId, ...)` 来复用 WS DTO。需要共享
   owner时提取 transport-neutral business command；HTTP 和 WS 各自适配到同一 owner。
5. legacy WebSocket 的 request/response matching 保持不变；WS DTO、`successEnvelope` /
   `errorEnvelope` 若仍由 WS 使用则保留在 WS 边界，不得泄漏回 HTTP。
6. 保留真正的业务身份，包括 `chatId`、`messageId`、`runId`、`toolCallId`、`attemptId`、
   `clientInstanceId` 和 provider response IDs。
7. 删除父节点确认的 dead HTTP-looking payload 声明；不得据此新增 route。
8. 更新 receipt/checker/tests，使 36 条 route 的 request、success、error 形状都被精确覆盖。

## 非目标与遮挡

- 不迁移 browser caller；F426C WIP 不得合入本分支。
- 不删除仍在 legacy WS/Host 中承担多路匹配的 request ID。
- 不设计新的幂等字段或 correlation header。
- 不运行 stable、live、真实 provider 或浏览器 E2E。
- 上游 service direct-body 未完成会遮挡 browser 响应解析，因此本节点只证明服务检查点。

## 验证与风险探针

本 Agent 是以下聚焦验证的唯一 owner：

```bash
SKIFF_ROOT=<assigned-skiff-worktree> npm run type-check --workspace @agine/service
SKIFF_ROOT=<assigned-skiff-worktree> npm test --workspace @agine/service
git diff --check
```

另按父节点第 10 节执行分区反搜，证明 HTTP 四个禁用字段和 envelope helper 不再命中，同时
WS/Host 的正向 `requestId` 命中仍存在。最早风险探针是 36 条 route 的 source/checker test：
至少含一个 success、一个业务 error、一个 405 以及四个 forbidden input aliases。

任何 production、schema、fixture 或 checker 改动都会使本节点证据失效；browser-only 后继改动
不使服务聚焦证据失效。

## Worktree、提交与交付

- Internals：`/Users/geek/workspace/internals-p5-f428a-agine-http-direct`
- 分支：`codex/p5-f428a-agine-http-direct`
- Skiff result：`/Users/geek/workspace/skiff-p5-f428a-agine-http-direct`
- 分支：`codex/p5-f428a-agine-http-direct`

启动后 5 分钟内完成第一次实际代码修改；否则按工作流返回 `TASK_NOT_EXECUTABLE`。提交
Internals implementation，再新增并提交
`P5-F428A-agine-http-direct-body-service-result.md`。返回两个 commit/tree、自验收矩阵和
clean 状态。不得 merge、rebase、push、stable 或 live；完成本节点后不得自行承接 browser
后继任务。
