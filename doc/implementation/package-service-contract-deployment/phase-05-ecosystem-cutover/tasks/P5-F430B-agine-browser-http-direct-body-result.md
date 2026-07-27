# P5-F430B Agine browser HTTP direct-body cutover result

状态：`IMPLEMENTED`。

Agine browser consumer 已完成 F428A direct-body cutover。WIP 中的 HTTP correlation 注入与
legacy envelope flatten 已删除；22 条 ordinary caller、原 chat caller、activation/provider、
frontend mock 和指定 E2E helper 已按 transport 分区。legacy WebSocket request matching 与 Host
file RPC 保持不变。没有触发 `TASK_SCOPE_EXPANDED`。

## 1. 精确输入与提交

| 输入 / 输出 | worktree / branch | commit | tree | 状态 |
| --- | --- | --- | --- | --- |
| F428A Internals 输入 | `/Users/geek/workspace/internals-p5-f430b-agine-browser` / `codex/p5-f430b-agine-browser` | `658540a60f83e7609818a7531c5a2944c8c8fb47` | `7cf54d209adc8ecfa85e94b04ab49bde033fcaa6` | direct-body service checkpoint |
| 重放的 scoped WIP | 同上 | `6a1396ed41a90ab17d1a97154acd211f9a295a8d` | `d1537439c5bb697c0cc887ec0e740a6b7569fe82` | 仅输入，不作为完成候选 |
| Internals completion candidate | 同上 | `61cd3c4cd79324c6d439d4aac8a015c1d48dfb2f` | `db88355a103e6e1939e9969756501c7f656c1344` | 独立 completion commit，clean |
| Skiff result 输入 | `/Users/geek/workspace/skiff-p5-f430b-agine-browser` / `codex/p5-f430b-agine-browser` | `e78ac59c39d957b80895b1dae61e2f9bd53d9c81` | `c9e77353d92c8affada74c4c0d2155128f13f57b` | clean |
| Skiff result 输出 | 同上 | 本文独立 result commit；精确 hash 见交付回执 | 本文独立 result tree；精确 hash 见交付回执 | 只新增本文 |

第一次实际代码修改在启动后 5 分钟内完成。completion candidate 相对 F428A 只修改
`agine/client/**`；没有修改 protocol、service、host、其它 Internals domain、Skiff production
或 skiff-packages。

## 2. 实现结果

### 2.1 HTTP helper 与 direct body

- `agine/client/src/lib/http.ts` 不再导入 `nanoid`，也不生成、接收或注入 HTTP
  `requestId`。
- `flattenLegacyEnvelope` 及对 `ok`、`eventName`、outer `payload`、`*-response` 的 HTTP
  解析已删除；统一 HTTP post owner 以 fetch Promise 归属 response。
- 2xx 直接返回 JSON business body；non-2xx 只从
  `{"error":{"code","message"}}` 构造 `HttpRequestError`，并保持 status/code/message。
- request/response log 只保留 operation label、URL/status 与递归脱敏的业务 body，不制造 wire
  correlation 字段。
- `/chat/llm-call` 的 `payload` 仍是 endpoint 自己的业务字段，unit 和 frontend 测试均证明它
  只保留一层且不会被 flatten。

### 2.2 caller 与真实业务 identity

- 22 条 `AGINE_ORDINARY_USER_HTTP_POST_PATHS` 都由唯一 browser owner 使用 literal HTTP path
  与 typed business payload；不再通过 `socket.request` 或 `socket.send`。
- 原 `/chat/list`、`/chat/create`、`/chat/get`、`/chat/send` 直接消费 F428A business
  response；activation、credential、OAuth 和 provider catalog caller 同步消费 direct body。
- `chatId`、`messageId`、`runId`、`toolCallId`、`attemptId`、`messageSeq`、
  `clientInstanceId`、OAuth `sessionId` 与 provider `responseId` 均按业务语义保留。
- 未新增 pending correlation map、header、query、cookie、SSE id、idempotency alias 或其他
  同义字段。

### 2.3 HTTP/WS mock 与 E2E 分区

- frontend `mockApp` 新增独立 `httpPost(path, body)`；HTTP observation 精确记录
  `{transport:"http",path,body}`，返回 direct business body。
- mock WS `request` 继续记录 `transport:"ws"`、`eventName`、`requestId` 并返回 matching
  envelope；WS `send` 与 downlink event owner 保持独立。
- frontend 新增 22-route ordinary matrix和 `/chat/llm-call` probe：逐条走真实 browser HTTP
  helper，断言 literal path、精确业务 body、四个禁用 alias 均不存在；同一 probe 另证 legacy
  Host-file WS request 仍有 request ID。
- `api.chat-smoke.mjs` 的 list/create/get/send HTTP body 不再带 ID，也不再检查 `body.ok` 或
  flatten envelope；agent create/delete cleanup 改走 direct HTTP。其
  `CookieWebSocketRpc` request-ID generator 仅保留在 WS owner。
- `system.two-hosts.e2e.ts` 的 HTTP list、activation、create、get 与
  `machineHarness.ts` 的 HTTP create 均改为 direct request/response；文件内 WS helpers 的
  request ID matching 未改。
- browser `AgentSocket` 保留 WIP 的 application JSON heartbeat no-op；真实 Host file
  WebSocket RPC、shared socket pending matching、`GlobalErrorHandler` 与 WS DTO 未删除。

## 3. 验证结果

所有命令均在 assigned linked worktree 内执行；没有启动、修改或 reload stable/live 服务。

| 验证 | 结果 | 计数 / 说明 |
| --- | --- | --- |
| `npm run type-check --workspace @agine/client` | PASS | TypeScript 0 error |
| `npm run test:logic --workspace @agine/client` | PASS | 46 files，255 tests |
| `npm run test:frontend --workspace @agine/client` | PASS | local Chrome channel，15 tests |
| `node --test agine/client/e2e/support/cookie-websocket-rpc.test.mjs` | PASS | 3/3；abort、timeout、close 都清理 pending matching |
| `node --test agine/client/e2e/support/chat-smoke-cleanup.test.mjs` | PASS | 6/6；额外 HTTP cleanup transport probe |
| `git diff --check` | PASS | 0 error |
| completion scope / clean audit | PASS | candidate 只含 `agine/client/**`，commit 后 clean |

### 3.1 分区反搜

- 精确限定 production `agine_http_*.skiff`、`agine/protocol/http.ts` 与
  `agine/client/src/lib/http.ts` 后，四个 correlation alias 为 0 hit。
- 同一 production scope 对
  `requestIdFromBody|flattenLegacyEnvelope|-[Rr]esponse|tool_call/receipt` 为 0 hit。
- F427A 给出的未排除 tests 的宽 glob 有 1 hit：
  `agine_http_chat.test.skiff:46 requestId: null`。它是 F428A 已有 source test helper，为建立
  fixture 而构造 legacy WS `ChatCreateInput`；不是 HTTP wire producer/body/parser，且 service
  在本 leaf 禁止修改，因此作为分区正向例外保留，不伪报宽 glob 零命中。
- 指定三个 E2E helper 中的残留命中全部位于 WS owner：chat smoke 的
  `CookieWebSocketRpc` 配置、two-hosts `wsRequest`/cleanup frame、machine harness
  `browserWsRequest`。HTTP fetch body 已无命中。
- `agine_ws_*.skiff`、Cookie WS RPC 与 Host source 继续有大量 `requestId` 正向命中；
  Cookie WS 3 项测试证明 matching/pending cleanup 仍有效。

## 4. 自验收矩阵

| leaf 要求 | 结果 | 代码 / 测试证据 |
| --- | --- | --- |
| 1. helper 不制造 ID、direct success、fixed non-2xx error | PASS | `http.ts`；exact body/direct success/structured error/fallback tests |
| 2. 原 chat、llm-call、activation/provider direct response | PASS | chat actions/history、ModelContextViewer、toolprovider/OAuth/config caller；unit + frontend |
| 3. 22 ordinary caller 使用 literal HTTP business payload | PASS | typed owner mapping static gate + 22-route frontend matrix |
| 4. Promise/AbortController owner response，无 correlation state | PASS | fetch Promise owner；E2E helper 局部 abort scope；反搜无 pending/header/query alias |
| 5. browser WS 只保留 downlink 与明确 legacy Host helper | PASS | ordinary production caller 的 WS 调用为零；Host-file RPC 与 WS listener 正向保留 |
| 6. HTTP/WS mock observation 分离 | PASS | `mockApp.httpPost` 与 WS request/send 独立记录；frontend transport probe |
| 7. 三个指定 E2E helper 拆 transport | PASS | chat-smoke、two-hosts、machine harness direct HTTP diff与分区反搜 |
| 8. 四 alias 精确断言，WS matching 有效 | PASS | HTTP unit/frontend 四 alias assertions；Cookie WS 3/3 |
| 9. 删除 HTTP response envelope/echo/duplicate-field期待 | PASS | scoped legacy reverse search为零；logic/frontend 全绿 |

最早风险探针均已覆盖：`http.test.ts` 的 exact body/direct success/error，一个 ordinary store
action test，以及 frontend mock HTTP observation。

## 5. 禁令与收尾

- 未修改 F428A protocol/service shape，因此没有 scope expansion。
- 未运行 combined service+browser probe，也未自行承接 combined 节点。
- 未启动或修改 stable instance、watch registry、router、runtime、telemetry、MongoDB、固定端口
  服务或真实 provider。
- 未执行 live、merge、rebase 或 push。
- Internals WIP 后存在独立 completion commit；Skiff 仅以独立 result commit 新增本文。
