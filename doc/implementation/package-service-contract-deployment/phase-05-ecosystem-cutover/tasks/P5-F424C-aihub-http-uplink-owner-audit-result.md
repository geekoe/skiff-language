# P5-F424C AIHub HTTP uplink owner audit result

状态：`READY_TO_IMPLEMENT`。没有触发 `TASK_SCOPE_EXPANDED`：tracked AIHub consumer 没有独立主动
push 需求；现有 `/v1/chat/events` 业务 envelope 足以承载原浏览器可观察事件；Skiff 已有
`rawHttp + Stream<std.http.HttpResponseStreamEvent>`，不需要新增 external business protocol。

但 current AIHub route 仍返回聚合后的单个 `std.http.HttpResponse.body`，并不是真正的 HTTP
server stream。后继实现必须先把两条 events entry 切到 existing server-stream surface，不能只把浏览器
`WebSocket` 换成 current `fetch(...).text()`。

## 1. 精确输入、边界与 worktree

| 输入 | worktree / branch | commit | tree | 审计状态 |
| --- | --- | --- | --- | --- |
| Internals production | `/Users/geek/workspace/internals-phase-05-integration` / `codex/package-service-phase-05` | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` | 与任务精确匹配，审计前后 clean |
| Skiff production toolchain | `/Users/geek/workspace/skiff-phase-05-integration` 的冻结 production ancestor | `ba74febaca5dbe8f2b55d6db04e0544a6758bf4b` | `7ac91495f85bbf997fe4f57ddfbec76b82cc753c` | current checkout 只比该点新增 F424 batch/leaf 文档，production diff 为 0 |
| Skiff audit checkout 起点 | `/Users/geek/workspace/skiff-p5-f424c-aihub-audit` / `codex/p5-f424c-aihub-audit` | `4b77ed1f6bfeb3deb7f3364981cf06de6e47a522` | `ec8cd55a377e01b37a107301b3707322c64b7921` | 起点 clean |

审计只读取 Internals 的 `aihub/service/**`、`aihub/client/**`、AIHub package/test/workflow owner，
以及核对 raw HTTP stream 所需的 Skiff canonical source/doc。没有修改 Internals production、test、
fixture、manifest 或设计；Skiff 唯一写入是本文档。

## 2. 结论摘要

1. AIHub 唯一 tracked chat uplink sender 是 `aihub/client/app.js` 中
   `socket.send({type:"chat.request", ...})`；没有第二个浏览器、Host、CLI 或后台 sender。
2. AIHub 唯一下行 primitive 是
   `std.websocket.sendTextToConnection(connectionId, envelope)`。全部调用都位于一次
   `chat.request` receive 的同步处理链内；没有定时器、数据库通知、actor callback、broadcast 或其它
   与该 uplink 无关的主动 push。
3. 因此 HTTP uplink 迁移后 AIHub 不需要保留 WebSocket downlink entry。
   `AihubSocketContext`、connect 时回填的 `connectionId` business identity 和 `/ws` entry 应一起删除；
   不需要另行设计 connect identity、auth 或 policy。
4. `/v1/chat/events` 与 `/chat/events` 的成功 body 使用与 WebSocket 相同的
   `aihub.llm.event` envelope，能够保留 reasoning、tool call、base64、usage 和 provider failure
   metadata；它们是浏览器迁移的唯一正确 HTTP surface。
5. `/v1/chat/completions` 与 `/chat/completions` 是有意 lossy 的 OpenAI compatibility projection，
   只保留 text delta、一个 finish chunk 和 `[DONE]`，不能替代浏览器当前 event-level consumer。
6. current events implementation 先消费完整 provider stream，再拼接一个 `string`/`bytes` response；
   首 item timing、渐进 UI 和 transport cancellation 都没有保留。Skiff 已有精确 server-stream handler
   return type，因此这是 AIHub service implementation/deployment 缺口，不是公共 Skiff 协议缺口。
7. service 与 client write set 互斥，可以在本文冻结的 wire checkpoint 后并行开发；最终验收必须等
   service server-stream entry 先可生成，再做组合验证。

## 3. WebSocket production 行为

### 3.1 Connect、receive 与 request

`internal.aihub_service.websocket` 当前行为如下：

| phase / input | production 行为 | 下行结果 |
| --- | --- | --- |
| `connect` | 无条件 accept；context 只保存 router 给出的 `connectionId`；`businessIdentity` 也只是同一个 connection id | 无业务 frame |
| binary receive | 不解码、不关闭连接 | `requestId:"unknown"`、`runId:"run-unknown"`、`seq:0` 的 `error(unsupported_message)` |
| 非 JSON object text | JSON decode fail closed | unknown id 的 `error(invalid_json)` |
| JSON 但 `type != "chat.request"` | 读取可选 `requestId`/`runId` 后拒绝 | `error(unsupported_message)` |
| 缺失 object `body` | 不进入 LLM | `error(invalid_request)` |
| 合法 `chat.request` | 只从 `body.provider` 或 `body.model` 推导 provider；校验 model/messages/tools 等 OpenAI-compatible body | 进入 event stream |
| disconnect / 其它 ingress tag | 返回 `null` | 无 cleanup、cancel 或业务 push |

请求 outer shape 是：

```json
{
  "type": "chat.request",
  "requestId": "req-1",
  "runId": "run-req-1",
  "body": {
    "provider": "bailian",
    "model": "qwen3.7-max",
    "messages": [{"role": "user", "content": "hello"}],
    "stream": true
  }
}
```

`requestId` 缺失时使用 `"unknown"`；`runId` 缺失时使用 `run-${requestId}`。`body.stream` 不控制
AIHub 内部行为；provider request 始终按 stream 形状构造。WebSocket 只支持 body provider 或 model
catalog inference，不使用 HTTP 的 header/query provider fallback。

### 3.2 Stream item、terminal 与 error

成功 request 的下行顺序是：

1. `seq:0` 的 `request-start {provider?, model?}`；
2. `streamManagedChat` 的每个 `llmApi.LlmStreamEvent`，每项 `seq + 1`；
3. 最后一个业务 terminal 是 `finish`；WebSocket 不发送 `[DONE]`。

所有 frame 都是同一 envelope：

```json
{
  "type": "aihub.llm.event",
  "requestId": "req-1",
  "runId": "run-req-1",
  "seq": 1,
  "event": {"tag": "text-delta", "id": "text-1", "text": "hello"}
}
```

可到达的 LLM item 是 `start`、`text-delta`、`reasoning-delta`、`tool-call-start`、
`tool-call-delta`、`tool-call-end`、`base64` 和 `finish`。`finish` 保留可选
`finishReason`、`usage`、`meta`。codex-relay 的预期 provider failure 不是 transport exception；
它被解码为 `finishReason:"error"`，并在 `meta` 中保留有限的 `providerCode` 与 canonical
`retryable`，不泄漏 upstream detail。

另外三类 exception 被转成 in-band terminal error：

| exception / validation | error code | retryable |
| --- | --- | --- |
| JSON/schema/managed request decode | `invalid_request` | `false` |
| provider protocol | `provider_protocol_error` | `false` |
| provider unavailable | `provider_unavailable` | `true` |

unsupported provider 在 `request-start` 前输出 `unsupported_provider`。异常可能发生在已经发出
`request-start` 或部分 LLM item 之后；error 使用同一 `StreamSendState` 的下一个 seq。

### 3.3 真实 client 行为与取消

唯一 sender/consumer 是 `aihub/client/app.js`：

- 每次 Send 新建一个 socket和一个 request；没有 multiplex 或复用；
- open 后唯一一次 `send` 就是上述 `chat.request`；
- 每个 message 立即 JSON decode、追加 raw event，并交给 `applyStreamEnvelope` 更新 UI；
- `finish` 和 `error` 都是 client terminal；client 随后主动以 code `1000` 关闭 socket；
- transport error或 terminal 前 close 作为失败；
- Cancel 只把本地 session 标为 aborted并以 code `1000` 关闭 socket。

service 的 disconnect branch 没有把 close 关联到 provider request，也没有 `chat.cancel` message。
因此 current Cancel 的可靠语义只有“浏览器停止等待并显示 Cancelled”，不能证明 provider work 被终止。

## 4. HTTP 四条 chat route 对照

四条 named `rawHttp` entry 当前都调用同一个 unary
`handleAihubHttp(HttpRequest) -> HttpResponse`。

| HTTP entry | 当前成功 body | 与 WebSocket event wire 的关系 | 浏览器迁移结论 |
| --- | --- | --- | --- |
| `POST /v1/chat/events` | 聚合后的 SSE text；每个 `data:` 是 `aihub.llm.event`，末尾 `[DONE]` | payload item 等价；arrival timing、custom runId 和当前 error channel 不等价 | canonical target；必须先改成 raw server stream |
| `POST /chat/events` | 与 `/v1/chat/events` 同实现 | 同上 | 保留 alias，但浏览器应选 `/v1/chat/events` |
| `POST /v1/chat/completions` | 聚合后的 OpenAI chat completion SSE chunks + `[DONE]` | 只投影 `text-delta`；丢失 request-start、reasoning、tools、base64、finish meta | 不可用于当前 UI |
| `POST /chat/completions` | 与 `/v1/chat/completions` 同实现 | 同上 | 不可用于当前 UI |

### 4.1 Request 差异

events HTTP route 直接接收原 `body`，因此 provider/model/messages/tools/temperature/max_tokens 等
业务字段可逐字复用。需要把 WebSocket outer `requestId` 映射为现有 body 字段
`request_id`；HTTP service 用它生成 `requestId` 和固定的 `run-${requestId}`。tracked client 本来就只
发送这一固定 runId，所以没有 client-visible loss。不要新增 camelCase compatibility read。

HTTP provider resolution 比 WebSocket 更宽：

1. `body.provider`；
2. `x-aihub-provider`；
3. `?provider=`；
4. model catalog inference。

浏览器当前在 body 内发送 provider，因此迁移不改变选择结果。

### 4.2 Item、terminal 与 error 差异

events HTTP success 的 envelope、seq 和 event JSON 与 WebSocket 相同，并额外在最后写
`data: [DONE]`。但 current `chatEventsBodyForInput` 在返回 `HttpResponse` 前完成整个
`for event in streamManagedChat(...)`，所以 router 只能一次性写完整 body：

- client 收不到 incremental item或真实 TTFB；
- fetch abort 在 response 返回前不能通过 response body reader表达；
- provider exception 会丢弃已经拼接的局部 `output`，由 outer catch 改成完整的
  `400/502/503 application/json` response；
- WebSocket 则可能已经显示局部 item，随后收到 in-band `error`。

后继 server-stream handler 必须冻结以下 terminal 规则，才能保持 tracked client 的可观察语义：

- 在 `streamStart` 前完成 method/body/provider/managed request preflight；失败仍返回现有
  `4xx/5xx` JSON status/body；
- start 后逐项写 SSE envelope；
- 正常 `finish` 后写 `[DONE]` 并 `streamEnd`；
- start 后的 decode/protocol/unavailable failure 写下一个 seq 的既有 `error` envelope并结束；
  `error` 本身是 terminal，不把失败伪装成成功 `finish`；
- client disconnect/AbortController cancel 依赖 Skiff 已冻结的 server-stream cancel chain向 runtime
  传播；outbound `std.http.stream`/`std.http.sse` 随 ancestor cancel abort in-flight request。

这没有新增业务 envelope、path、method、content type或 provider protocol，但会把两条 gateway entry
从 unary raw HTTP identity切成 server-stream identity，必须生成新的 gateway entry/deployment revision。

### 4.3 为什么 completions 不能复用

`streamChatCompletion` 只把 `text-delta` 变成 OpenAI `choices[].delta.content`。最后从 `finish`
提取 `finishReason`/`usage`，输出一个 finish chunk，再输出 `[DONE]`。它不输出 reasoning、tool call、
base64、request/run/seq、provider failure meta，也不表达 AIHub in-band `error`。即使 request
`stream:false`，current endpoint仍返回这一 SSE compatibility body。因此它与
`managedLlm.streamChat`、AIHub event route和浏览器 reducer都是不同 surface。

## 5. WebSocket entry 删除结论

精确反向搜索显示 AIHub production 只有一个下行 native：

```text
internal/aihub_service.skiff
  std.websocket.sendTextToConnection(...)
```

所有调用点都在 `handleSocketMessage -> streamChatEventsToSocket*` 链内。client 侧也只有一个
`new WebSocket`、一个 `.send(chat.request)` 和一个 message consumer。没有证据支持 HTTP uplink 完成后
仍保留 AIHub WebSocket。

后继 service leaf 应删除：

- `service.yml` 的整个 `websocket.routes[/ws]` block；
- `api.yml` 的 `websocket` 与 `AihubSocketContext` exports；
- `AihubSocketContext`、`StreamSendState`；
- `llmRequestFromWebSocket`；
- `sendStreamEvent` / `sendNextStreamEvent`；
- `streamChatEventsToSocket*`、`handleSocketMessage`、`websocket`；
- receipt oracle和 README 中的 `/ws`/`chat.request` claims。

`streamEnvelope`、`streamEventSse`、`requestStartEvent`、`llmEventJson` 和 `errorEvent` 是 HTTP
events wire owner，不能随 WebSocket helper 一起误删。

当前 connect 的 `businessIdentity = connectionId` 只用于旧 connection-local response routing，不是用户、
tenant、session或授权身份。删除整个 entry 后没有待定义的 business identity/policy，故未触发停止条件。

## 6. Browser HTTP client 能力与最小写入点

### 6.1 已有可复用能力

- `buildRequestBody()` 已生成目标 HTTP route接受的 OpenAI-compatible body；
- `joinUrl()` 已正确添加 `service=agine.ai/aihub` 和 `version=0.1.0` selector；
- `applyStreamEnvelope()` 及 reducer 已处理 request-start、text、reasoning、tool、base64、
  finish和 error；
- `readJsonResponse()` / `formatHttpError()` 可复用为 pre-stream non-2xx error decoder；
- UI 已有 Send/Cancel busy state和 AbortError 专用状态分支。

### 6.2 真实缺口

client 当前没有：

- chat `fetch(POST /v1/chat/events)`；
- `AbortController` 和传给 fetch 的 `signal`；
- `Response.body.getReader()` / `TextDecoder`；
- 能跨任意 byte chunk boundary工作的 SSE parser；
- `[DONE]`、EOF-before-terminal、malformed envelope和 post-start network error处理；
- 对 app chat path 的任何 unit、integration或浏览器 E2E。

`aihub/client/server.test.mjs` 的两个 test 只验证静态 file server和 port parsing。shared
`scripts/run-web-client.test.mjs` 也只验证 launcher/静态首页，不会执行 `app.js`。

### 6.3 最小 client write set

client leaf 独占：

```text
aihub/client/app.js
aihub/client/index.html
aihub/client/<new-stream-helper>.mjs
aihub/client/<new-stream-helper>.test.mjs
aihub/client/server.mjs                 # .mjs browser MIME（若采用该 helper 形态）
aihub/client/server.test.mjs
aihub/README.md
```

最小迁移是：

1. 生成 requestId并写入 body `request_id`；
2. `fetch(joinUrl(base, "/v1/chat/events"), {method:"POST", headers, body, signal})`；
3. non-2xx 先读有限 JSON/text并走现有 HTTP error UI；
4. 增量解析 `data:` records，JSON envelope继续交给 `applyStreamEnvelope`；
5. `finish`/`error` 为业务 terminal；`[DONE]`/EOF负责 transport完整性；
6. Cancel 调用 `AbortController.abort()`，保持现有 `Cancelled` UI。

如果把 pure parser拆为 browser ES module，static server必须把 `.mjs` 作为 JavaScript MIME返回并补对应
server test；也可以选择不需要新 MIME 的等价可测布局。不需要修改 `styles.css`、web-client registry或
launcher。

## 7. Service、receipt、record 与 test owner

### 7.1 Service implementation write set

service leaf 独占：

```text
aihub/service/internal/aihub_service.skiff
aihub/service/internal/aihub_service.test.skiff
aihub/service/internal/gemini.live.test.skiff
aihub/service/skiff.test-doubles.json
aihub/service/service.yml
aihub/service/api.yml
aihub/service/service-api-receipt.mjs
aihub/service/service-api-receipt.test.mjs
aihub/service/README.md
```

两条 events named entry 应绑定一个精确返回
`Stream<std.http.HttpResponseStreamEvent>` 的新 handler；其它五条 HTTP entry 可继续绑定 unary
`handleAihubHttp`。`package.yml` 的 package/service dependencies、`serviceCalls` 的
`managedLlm`/`providerCatalog` roots和 `packages/llm-api/**` 均不应改变。

### 7.2 Receipt 与 manifest oracle

必须同步：

- `expectedAihubHttpEntries` 不能再假设七条 entry共享同一个 handler；两条 events entry需要独立
  server-stream handler expectation；
- `service.yml` source oracle改为要求无顶层 `websocket:`、无 `routes:`/`operation:`；
- package API path expected 移除 `websocket`，API source oracle要求 `websocket` 与
  `AihubSocketContext` export为 0；
- exact generated receipt 仍必须只暴露五个 service-call operation：
  `managedLlm.{streamChat,validateChat,webSearch}` 与
  `providerCatalog.{builtinProvider,model}`；
- `skiff.test-doubles.json` 当前按完整 test name绑定 events SSE double；测试重命名/拆分时必须同步 key，
  不能静默失去真实 reasoning/tool/finish fixture。
- `gemini.live.test.skiff` 当前读取 unary `HttpResponse.body`；它必须改为同一 server-stream event
  collector/真实 route形态，但仍保持 `test defaultRun false`，不能进入 non-live gate。

`managedLlm.streamChat` 是 Skiff service-call server stream；
`/v1/chat/events` 是 external raw HTTP server stream。二者共享内部 `streamManagedChat` producer，
但 contract operation、wire envelope、identity、error/cancel adapter都不同，不得互相替代或共用 receipt
断言。

### 7.3 Artifact generation

| record / identity | 预期变化 |
| --- | --- |
| `PackageArtifact` / `PackageBuildId` | source、public API index变化，必须新 build |
| `PackageSchemaIndexIdentity` | 删除 public `AihubSocketContext` 等无关 public path会变化 |
| `ServiceContract` / `ServiceProtocolIdentity` | 五个 selected service-call operation及其 schema不变，identity 必须保持不变 |
| 两条 events `GatewayEntryIdentity` | unary raw HTTP -> server-stream raw HTTP，必须变化 |
| AIHub WebSocket gateway entry | 删除 |
| `ServiceDeployment` revision/identity | implementation build、gateway entries、ingress变化，必须变化 |
| `RuntimeAssembly` identity | 必须选择新的 AIHub deployment，因此变化 |

因为 service protocol不变，Agine 的 `aihub/managedLlm.*` consumer不需要把 external HTTP stream当成新
service dependency，也不应因本 leaf改写其 source。最终 assembly只更新 deployment selection。

## 8. 建议 DAG 与互斥 ownership

```text
C0  本 result 冻结既有 chat-events wire
      request_id -> requestId/runId
      request-start -> LlmStreamEvent* -> finish/[DONE]
      pre-start HTTP error / post-start error envelope / abort
       |
       +--> C1 AIHub service真实raw HTTP server stream + 删除全部AIHub WebSocket
       |      write: aihub/service/**
       |
       +--> C2 AIHub browser Fetch/SSE/AbortController + parser tests
              write: aihub/client/** + aihub/README.md
                    |
                    v
C3  generated receipt/deployment/assembly + combined non-stable HTTP probe + reverse search
```

C1/C2 的文件范围不重叠，可以并行实现；C2 可按本文 wire写 unit tests，但 combined acceptance 必须等 C1
生成 server-stream deployment。无需新增共享 production protocol/types package；若实现者需要机器可读
fixture，应由 C0/C3 单一 owner放在 test-only 范围，不能让两边各自发明 envelope variant。

## 9. 验证矩阵与最早风险探针

### 9.1 本次只读证据

| 命令 / 证据 | 结果 |
| --- | --- |
| `git rev-parse HEAD HEAD^{tree}`（Internals） | 精确为任务输入 |
| AIHub `new WebSocket` / `.send(` / `fetch(` 搜索 | chat sender 只有 `app.js` 一处；另一个 fetch 只读 providers |
| AIHub `std.websocket` / `sendTextToConnection` 搜索 | 唯一 native、全部属于 chat receive response链 |
| `.test.skiff` declaration discovery | non-live 46：`aihub_service` 45、`managed_provider_transport` 1；另有明确 live `gemini.live` 1 |
| Node test declaration discovery | service receipt 7、package-store 2、client static server 2；app stream test 0 |
| `node --test aihub/service/service-api-receipt.test.mjs aihub/service/scripts/local-package-store.test.mjs aihub/client/server.test.mjs` | 11 discovered，10 pass，0 fail，1 expected generated-receipt skip |

本审计没有运行 `gemini.live.test.skiff`、stable、instance、watch、reload、完整 N5 或任何网络 provider。

### 9.2 后继聚焦正负例

| owner | 必须正例 | 必须负例 |
| --- | --- | --- |
| service stream | start/chunk/end顺序；fragment逐项可见；request-start seq0；reasoning/tool/finish/usage；success `[DONE]` | invalid JSON/model/provider pre-start status；protocol/unavailable post-start error envelope；chunk after end / duplicate start fail closed |
| client parser | 任意 UTF-8/chunk边界；同 chunk多 event；finish + `[DONE]`；reasoning/tool/base64 UI | malformed JSON；non-2xx JSON；EOF before terminal；event error；network failure；AbortError |
| manifest/receipt | 2个 server-stream events entry、5个 unary HTTP entry；五个 service-call operation不变 | `/ws`、legacy operation、package `websocket` projection、HTTP handler kind误配 |
| combined | `/v1/chat/events` 首 item在 provider completion前可观察；abort终止 reader/request；alias等价 | completions route不得冒充 event wire；client不得重新打开 WebSocket fallback |

最早风险探针不是浏览器 E2E，而是 publish AIHub package/deployment后检查两条 events entry确实生成
server-stream gateway identity，并用 source test收集 `HttpResponseStreamEvent` 验证
`start -> chunk* -> end`。该探针失败时应停在 service owner，不要让 client加 polling、buffer decode或
WebSocket fallback。

真实 source test discovery 可用：

```bash
rg -n '^test "' \
  /Users/geek/workspace/internals-phase-05-integration/aihub/service/internal \
  --glob '*.test.skiff'
```

生成 isolated ecosystem store/base assembly后，聚焦执行形态应为：

```bash
node /Users/geek/workspace/skiff-phase-05-integration/scripts/skiff.mjs test \
  /Users/geek/workspace/internals-phase-05-integration/aihub/service \
  --artifact-root <isolated-ecosystem-store> \
  --base-assembly <generated-runtime-assembly-identity> \
  --deny-skips \
  --require-tests
```

`aihub/service/package.json` 的 current `npm test` 会运行 receipt/package-store/workflow guards并建立临时
canonical package/deployment/assembly，但 `scripts/test-isolated-service.mjs` 本身没有调用
`skiff test`。后继验收不能把该 authoring workflow成功误报为 46 个 `.test.skiff` 已执行；若要把 source
execution并入 canonical script，应由 shared workflow owner单独修改。

### 9.3 Legacy 反向搜索

AIHub cutover完成后以下搜索在 `aihub/service`、`aihub/client`、`aihub/README.md` 范围应为 0：

```text
websocket:
operation: websocket
/ws
chat.request
new WebSocket
joinWebSocketUrl
streamChatOverWebSocket
createSocketSession
WebSocketIngressEvent
WebSocketConnectResult
ConnectionMessage
std.websocket
sendTextToConnection
AihubSocketContext
StreamSendState
llmRequestFromWebSocket
streamChatEventsToSocket
handleSocketMessage
```

反向搜索不得禁止或删除 `managedLlm.streamChat`、`Stream<llmApi.LlmStreamEvent>`、
`HttpResponseStreamEvent`、`/v1/chat/events`、SSE `data:` 或 body中的 `stream:true`；它们是 current
目标 surface，不是 legacy WebSocket。

## 10. Worktree 与禁止动作

- Internals integration 在审计前后均为 clean；没有写入 production/test/fixture/artifact。
- Skiff task checkout 在写 result 前为 clean；交付只包含本文档的 result-only commit。
- 没有启动 stable/live/instance/watch/router/runtime/telemetry/MongoDB/client server。
- 没有 merge、rebase、push，也没有修改或读取 stable artifact root。
