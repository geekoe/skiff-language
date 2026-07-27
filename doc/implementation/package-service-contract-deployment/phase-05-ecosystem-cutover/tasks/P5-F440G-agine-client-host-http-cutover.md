# P5-F440G Agine client Host读取HTTP cutover

状态：Ready。确定性Internals实现leaf。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F440D-agine-host-peer-protocol-checkpoint-result.md`
- `P5-F439C-agine-host-jsonrpc-delta-audit-result.md`

精确Internals输入：

| Commit | Tree |
| --- | --- |
| `605ebd209dacac7c95aa79dc3a508d428a352453` | `95cc84051c350f45e38a6092958d58734c5278db` |

## 目标与写集

把浏览器Host文件list/search从Agine WebSocket request改为普通HTTP，把current-directory从轮询改为单次
HTTP，并把取消贯通到fetch `AbortSignal`。本任务可以针对HTTP mocks完成，不等待Skiff service caller。

唯一production/test写集：

- `/Users/geek/workspace/internals-p5-f440g-client-host-http/agine/client/**`

Skiff侧只允许新增本leaf result。禁止修改protocol、Host、service、shared-client、Skiff production或
权威设计。不得派子agent。

## 实现合同

1. `hostFileApi`：
   - import `@agine/protocol/http`的三条path/payload/success types与Host nested types；
   - list调用`POST /thread/host-files/list`；
   - search调用`POST /thread/host-files/search`；
   - 参数不含`eventName`、`requestId`或connection id；
   - 两个函数接受可选`AbortSignal`并传给HTTP helper；
   - 删除旧WebSocket envelope `unwrap`与client重复的Host result字段。
2. HTTP helper：
   - 为`agineHttpPost`/`ordinaryUserHttpPost`增加可选`{signal}`，传给真实`fetch`；
   - session等待后、发request前再次检查abort；
   - E2E mock获得同一个signal或等价可断言context，不能因mock绕过取消；
   - 普通既有caller不传options时行为byte-equivalent。
3. `useHostFileBrowser`：
   - directory与search各拥有当前`AbortController`；
   - 新请求、query/path supersession、panel close/inactive和unmount都abort旧fetch；
   - abort不显示业务错误、不被旧completion覆盖；既有sequence guard可作为第二层保护；
   - controller/timer在完成后只清理自己，不误abort较新的请求。
4. `getToolProviderCurrentDirectory`：
   - 单次调用现有`POST /toolproviders/current-directory`；
   - 返回`{toolProviderId,currentDirectory}`或合法空结果，不读取`refreshRequested`；
   - 删除retry delay、poll loop和相关测试clock；可接受可选AbortSignal并传给HTTP helper。
5. Agine client最后一个production WebSocket `request` consumer消失后，删除`lib/ws.ts`的`request` export、
   mock `request`接口和只服务该surface的tests。`on/off/socketBridge`与stream notification保持。
6. 不修改`shared-client`的通用`EnhancedWebSocket.request`，因为其它产品仍使用。

## 验证

至少覆盖：

- list/search exact HTTP path/payload/result和HTTP error；
- signal传入真实fetch与mock；
- directory/search supersession、close/inactive/unmount abort；
- abort不写UI error，late result不覆盖；
- current-directory单次请求、无poll/`refreshRequested`；
- client production无`ws.request`/`requestId` Host读取；
- architecture boundary仍保留WebSocket notification owner。

运行相关Vitest、client typecheck/architecture test、syntax/fmt/diff；禁止browser/live/stable和完整canonical
workflow。

若`ordinaryUserHttpPost`的signal改动必须修改shared-client/protocol，或仍有范围外Agine production
WebSocket request consumer，立即返回`TASK_SCOPE_EXPANDED`并列出精确consumer，不越界删除。

## 交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f440g-client-host-http`
- Internals branch：`codex/p5-f440g-client-host-http`
- Skiff result worktree：`/Users/geek/workspace/skiff-p5-f440g-client-host-http`
- Skiff branch：`codex/p5-f440g-client-host-http`
- result：`P5-F440G-agine-client-host-http-cutover-result.md`

实现与result分别提交；返回两个commit/tree、测试计数、abort证据、reverse search和两个clean状态。
不merge/rebase/push。
