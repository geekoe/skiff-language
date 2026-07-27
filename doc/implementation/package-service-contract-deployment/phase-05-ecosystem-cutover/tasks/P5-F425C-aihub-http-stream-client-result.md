# P5-F425C AIHub Fetch/SSE browser cutover result

状态：`PASS`。AIHub test client 已从每请求一个 WebSocket 的 chat path 切到
`POST /v1/chat/events` 的 Fetch/SSE/AbortController path。该 consumer checkpoint 已完成；
真实 AIHub combined 仍等待 F425B service server-stream leaf，不是 stable 候选。

## 1. 精确输入与提交

| 锚点 | commit | tree |
| --- | --- | --- |
| Internals exact start | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` |
| Internals implementation | `8f05aa7c1198e48faae0402062648e9f8f02894e` | `f498f14addde33e035ebe06ccae323b9396f0396` |
| Skiff result base | `e6ea73b67e9a1fb18e867fef3073ca33b7ecd5a5` | `54e51e6d86567473dfa269b975874d0352e63136` |

Internals exact start 与 F425 checkpoint 冻结输入一致。Skiff checkout 相对该 checkpoint 只有
任务文档演进，没有 production 写入。result-only commit/tree 由交付消息记录。

## 2. 实现结果

### 2.1 Browser request 与取消

`aihub/client/app.js` 的 chat sender 现在：

- 继续通过既有 `joinUrl` 写入 `service=agine.ai/aihub` 与 `version=0.1.0` selector；
- 对 `/v1/chat/events` 发起 `POST`，保留既有 provider/model/messages/stream 等业务 body；
- 生成 request id，并由 transport 精确写为 body 的 `request_id`；
- 每次请求创建原生 `AbortController`，Cancel 只调用 `abort()`，既有 `AbortError -> Cancelled`
  UI 分支保持不变；
- 每个 JSON envelope 仍交给既有 `applyStreamEnvelope`，因此 text/reasoning/tool/base64/finish/error
  UI reducer 没有形成第二套业务语义。

`new WebSocket`、`chat.request` sender、message/close/error consumer、socket session 和
WebSocket URL helper 已从 AIHub client production 删除，没有 compatibility fallback。

### 2.2 有限增量 SSE 与 transport 完整性

新增 `aihub/client/chat-stream.mjs`，其中：

- `createSseDataParser` 使用 streaming fatal UTF-8 `TextDecoder`，可跨任意 byte/chunk 边界处理
  CRLF/LF、单 chunk 多 record、multi-line `data:` 与 `[DONE]`；
- line 与 record 都受显式字符上限约束，invalid UTF-8、超限 line/record 均 fail closed；
- `streamChatEvents` 使用 Fetch、response body reader 和 parser 增量消费，不聚合成功 body；
- non-2xx body 使用独立 byte 上限读取，完整 JSON 保留为结构化错误，非 JSON 或截断响应以有限文本进入
  既有错误 UI；
- `finish` 与 `error` 是唯一业务 terminal；`finish` 后必须观察 `[DONE]` 和 clean EOF；
  F425B 冻结的 post-start `error` path 可直接以 clean EOF 完成，也允许随后出现 `[DONE]`；
- terminal 前 EOF、malformed JSON/envelope、terminal 后 envelope、提前 `[DONE]`、缺失 finish
  `[DONE]`、invalid reader chunk 和 post-start reader/network error 全部 fail closed；
- AbortError 保持原类型，不会被改写成普通 transport error。

### 2.3 Browser module serving

`index.html` 以 module script 加载 `app.js`。静态 server 为新增 `.mjs` helper 返回
`text/javascript; charset=utf-8`，对应 server test 通过真实 ephemeral loopback HTTP response
验证 MIME。

`aihub/README.md` 已把 test client 描述更新为 Fetch/SSE endpoint 与 selector，不再声明 `/ws`
或 WebSocket origin policy。

## 3. 自验收矩阵

| F425C 条款 | production / test 证据 | 结论 |
| --- | --- | --- |
| POST、selector、`request_id` 与业务 body | `app.js::sendChat`、`joinUrl`；transport success test 精确断言 method/headers/body/query | PASS |
| Fetch + AbortController + reader | `app.js::sendChat`、`chat-stream.mjs::streamChatEvents`；AbortError test | PASS |
| chunk/UTF-8/CRLF/LF/records/DONE/有限缓冲 | `createSseDataParser`；两项 pure parser tests及 success one-byte/变长 chunk coverage | PASS |
| reasoning/tools/base64/success | success transport test逐项观察 reasoning、text、tool start/delta/end、base64、finish、usage | PASS |
| pre-stream HTTP error有限读取 | JSON 503 与 oversized text 502 tests | PASS |
| terminal/EOF/malformed/network fail closed | in-band error、malformed JSON/envelope、early EOF、missing DONE、early DONE、post-terminal envelope、reader reset tests | PASS |
| Cancel UI 与无 socket close | `activeController` 为原生 AbortController；legacy reverse search 0 | PASS |
| 无 WebSocket fallback | client + AIHub README 对 `new WebSocket`、`chat.request`、socket helpers、`/ws` 等反向搜索 0 | PASS |
| `.mjs` MIME | `server.mjs` mapping + real response assertion | PASS |
| 写入范围 | Internals commit仅含 `aihub/client/**` 与 `aihub/README.md`；Skiff只新增本 result | PASS |

## 4. 聚焦验证

| 命令 / 检查 | discovered | pass | fail | skip |
| --- | ---: | ---: | ---: | ---: |
| `node --test aihub/client/*.test.mjs` | 18 | 18 | 0 | 0 |
| `node --check`：`app.js`、`chat-stream.mjs`、`server.mjs`、两份 `.test.mjs` | 5 | 5 | 0 | 0 |

其它证据：

| 检查 | 结果 |
| --- | --- |
| client `package.json` / `tsconfig*.json` discovery | 无 type-check 入口；不伪报 skip/pass |
| legacy WebSocket / `chat.request` 反向搜索 | 0 命中 |
| `git diff --check` / staged diff check | PASS |
| Internals implementation commit 后 status | clean |

18 个 Node tests 覆盖 2 个 pure parser cases、10 个 transport 顶层 cases及其 4 个嵌套负例，
以及 2 个 static server cases。没有零测试或 expected skip。

## 5. 范围与禁止动作

Internals implementation 只修改：

```text
aihub/README.md
aihub/client/app.js
aihub/client/chat-stream.mjs
aihub/client/chat-stream.test.mjs
aihub/client/index.html
aihub/client/server.mjs
aihub/client/server.test.mjs
```

Skiff 只新增本文档；未修改 Skiff production/shared-client、AIHub service 或其它 Internals 模块。
没有运行真实 provider、browser live、stable、instance、watch、reload 或完整 N5；static server test
只使用 test-owned ephemeral loopback port。没有派子 Agent，没有 merge、rebase 或 push。
