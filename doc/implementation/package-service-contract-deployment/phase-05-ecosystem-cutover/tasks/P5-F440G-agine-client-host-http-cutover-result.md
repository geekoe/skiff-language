# P5-F440G Agine client Host读取HTTP cutover result

状态：`IMPLEMENTATION_PASS`。Agine browser 的 Host list/search/current-directory 已完成普通 HTTP
cutover，取消已贯通到真实 fetch 与 E2E mock；最后一个 Agine production WebSocket `request`
consumer及其 client-local surface 已删除。没有触发 `TASK_SCOPE_EXPANDED`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| Skiff task 输入 | `75117f615d36b89bef01e851cadd6fa15b859a92` | `c0f77d3793107ef1605d90449e03164ea1620b20` |
| Internals 精确输入 | `605ebd209dacac7c95aa79dc3a508d428a352453` | `95cc84051c350f45e38a6092958d58734c5278db` |
| Internals implementation | `54dd8208eb9a8c4c7b58f97b46727932e8401ff9` | `1b065ff77eb2d927a403afda97c57857049be69b` |

Internals implementation 共修改 18 个文件，全部位于 `agine/client/**`。Skiff 侧只新增本文；result-only
commit/tree 由交付回执记录。没有修改 `agine/protocol`、`agine/host`、`agine/service`、
`shared-client`、Skiff production、权威设计或 official packages。

## 2. 实现结果

### 2.1 Host file HTTP contract

- `hostFileApi.ts` 直接消费 `@agine/protocol/http` 的
  `AGINE_HOST_HTTP_POST_PATHS.filesList/filesSearch`、对应 payload/response types，以及
  `@agine/protocol/hostPeer` 的 breadcrumb/file entry/directory/search nested types。
- list 精确调用 `POST /thread/host-files/list`，search 精确调用
  `POST /thread/host-files/search`；payload 只含 `chatId/mountId/toolProviderId/path?/query`。
- 两个 caller 都接受可选 `AbortSignal` 并传给 `ordinaryUserHttpPost`。返回值直接使用 HTTP success
  body；旧 WebSocket `request` import、envelope `unwrap` 和 client 重复 result interface 均已删除。
- HTTP error 保持 `HttpRequestError`/原 rejection，不再解析 `{ok,payload,error}` WebSocket envelope。

### 2.2 HTTP helper cancellation

- `agineHttpPost` 新增末尾可选 `HttpRequestOptions { signal? }`；`ordinaryUserHttpPost` 新增第三个可选
  options，并把同一个 signal 传入底层 helper。
- real fetch 只在 caller 提供 signal 时增加同一个 `RequestInit.signal`；未传 options 的既有 caller
  继续生成相同 URL、body、headers、credentials，mock 也继续只收到原来的两个参数。
- helper 在进入请求前检查 abort，并在共享 session setup 完成后、业务 fetch 发出前再次检查；
  session 等待期间被取消时不会发第二个 business request。
- E2E `httpPost` mock 在有 signal 时收到同一个 `{signal}` context；mock late resolve 后 helper 再次
  检查 abort，因此 mock 不能绕过 caller cancellation。

### 2.3 Browser hook ownership

- `useHostFileBrowser` 为 directory 与 search 各自持有当前 `AbortController`，sequence guard 继续作为
  第二层 late-completion fence。
- 新 directory/search request 会先取消同 lane 的旧 fetch；directory path supersession 同时取消旧
  search，query setter 在状态提交前同步取消旧 search。
- panel inactive、close 与 unmount 都取消两条 lane；search debounce cleanup 同时清理自己创建的 timer。
- abort/`AbortError` 不写 directory/search 业务错误；被 supersede 的 late resolve/reject 不能覆盖
  新路径、新 query 或 loading state。
- completion 只在 ref identity 仍指向自己的 controller 时清理 ref，旧 completion 不会清掉或误取消
  较新的 request。

### 2.4 Current-directory 与 WebSocket surface

- `getToolProviderCurrentDirectory` 现在单次调用
  `AGINE_HOST_HTTP_POST_PATHS.currentDirectory`，接受可选 signal，并返回
  `{toolProviderId,currentDirectory}` 或 `null`。
- retry delay、poll loop、retry clock/options 和 production `refreshRequested` read 已删除；空 object、
  `null` 或空 directory 都是合法空结果。
- `lib/ws.ts` 的 `request` export 和 E2E mock `request` interface 已删除；frontend mock 的旧 WS
  request implementation与只为该 surface 存在的 unit mocks/assertions同步删除。
- `GlobalErrorHandler` 不再假设 request-id error 会被已删除的 `request()` 捕获。
- `on`、`off`、`socketBridge`、stream/downlink notification wiring 与 send owner保持；没有修改
  `shared-client` 的通用 `EnhancedWebSocket.request`。

## 3. 测试与取消证据

测试按 red -> green 顺序推进。初始聚焦 red 明确命中未实现的 HTTP path、signal、session 后 abort、
polling 和 WebSocket request residue；实现后结果如下：

| 验证 | 结果 |
| --- | --- |
| client full logic：`vitest run --config vitest.config.ts` | PASS，`47 files / 266 tests` |
| `npm run type-check --workspace @agine/client` | PASS，TypeScript 0 error |
| `node --experimental-strip-types --check`（14 个修改的非 TSX 文件） | PASS |
| `git diff --cached --check` | PASS |
| production reverse search + write-scope gate | PASS |

其中本 leaf 的聚焦文件在 full logic 中贡献 39 条测试：

- `hostFileApi.test.ts`：list/search exact path、payload、direct result、same signal 与 HTTP error；
- `http.test.ts`：real fetch signal、E2E same signal、mock late resolve abort、session-wait abort，以及
  no-options fetch 无 `signal` 字段；
- `useHostFileBrowser.test.ts`：directory/search supersession、inactive、close、unmount、AbortError
  suppression、late result fence与controller identity；
- `toolproviderApi.test.ts`：单次 current-directory、same signal、`refreshRequested:true` 不触发 retry、
  object/null empty result；
- `ws.test.ts` 与 architecture gate：notification connection保持、request surface消失、Host HTTP
  canonical path owner完整。

没有运行 browser/frontend E2E；frontend mock/test只做 source迁移和 syntax检查，符合本任务禁令。

## 4. 反向搜索与边界

- Agine production（排除 tests/E2E/node_modules）反搜
  `.request(`、`export async function request`、`EnhancedWebSocket.request` 和 mock `request?:`：0 hit。
- `hostFileApi.ts` 反搜 `eventName|requestId|connectionId|unwrap`：0 hit。
- `toolproviderApi.ts` 反搜 `refreshRequested|retryDelaysMs|setTimeout(`：0 hit。
- `ws.ts` 与 frontend mock 反搜旧 mock request interface/implementation：0 hit。
- `ws.ts` 正向搜索确认 `on`、`off`、`socketBridge` 三个 notification/send owner仍存在。
- 三条 production Host HTTP caller都正向引用
  `AGINE_HOST_HTTP_POST_PATHS.filesList/filesSearch/currentDirectory`。
- write-boundary gate证明所有 Internals diff 都匹配 `agine/client/**`；implementation commit 后
  Internals clean。

测试中的 `refreshRequested:true` 只是一条负向 vector，用来证明 client 不再读取或轮询该退休字段；
`types.ts:ThreadRunRequestInfo.requestId` 是 provider run 的业务字段，不是 Host transport correlation。
E2E `CookieWebSocketRpc.request` 属于独立 chat-smoke test utility，不是 Agine browser production
consumer或被删除的 `lib/ws.ts` surface。

## 5. 隔离与收尾

- 未发现范围外 Agine production WebSocket request consumer，因此无需 scope expansion。
- 未运行 build/dev/start、browser、stable/live、watch、reload、固定端口或完整 canonical workflow。
- linked worktree 验证只使用临时/只读 dependency links；生成的 cache、symlink 与 tsbuildinfo均已移出
  worktree，没有 tracked 或 ignored validation residue。
- 未派子 agent；未 merge、rebase 或 push。
- Internals implementation 已独立提交且 clean；Skiff result 提交后的最终 commit/tree 与 clean 状态由
  交付回执记录。
