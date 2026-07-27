# P5-F427B AIHub HTTP correlation owner audit result

状态：`AUDIT_COMPLETE / READY_FOR_SPLIT_REPAIR`。

结论明确，不触发 `TASK_SCOPE_EXPANDED`：

- AIHub external HTTP body 的 `request_id`、SSE envelope 的 `requestId`，以及沿 service helper
  传递的同名参数都只为旧 WebSocket request/response envelope 保留，应删除。
- AIHub SSE envelope 的 `runId` 没有独立业务语义。它从不由 HTTP caller 单独提供，唯一 producer
  是 `runIdFromRequestId(requestId) = "run-" + requestId`；没有 persistence、cancel、abort、
  provider request、service-call 或 browser reducer consumer。它应与 `requestId` 一起删除。
- `request-start` tag、stream-local `seq`、`llmApi.LlmStreamEvent.id`、`toolCallId` 以及 provider
  `response.id` / `call_id` / `item_id` 不是 HTTP correlation alias，必须保留。
- `managedLlm.streamChat` service-call contract及 provider protocol不在 repair 写集内。

本 leaf 只新增本文，没有修改 Internals production、test、fixture、manifest、receipt或依赖代码。

## 1. 精确输入与审计边界

| 输入 | Checkout / branch | Commit | Tree | 审计前状态 |
| --- | --- | --- | --- | --- |
| Internals exact candidate | `/Users/geek/workspace/internals-phase-05-integration` / `codex/package-service-phase-05` | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | clean |
| Skiff result checkout | `/Users/geek/workspace/skiff-p5-f427b-aihub-correlation-audit` / `codex/p5-f427b-aihub-correlation-audit` | `d664bddf17adae74e15b74a2f03dc30102f1665b` | `08d95882714078c3a742280cde082871d15cb533` | clean |
| 父节点冻结的 Skiff 设计 | canonical design commit | `42337081095cce9c618508b9938cf28516054a75` | `da930e4ba3674c4913690664a537af4c5cfe0b23` | 只读设计输入 |

完整读取了 leaf、直接父节点 `P5-F427-http-correlation-field-removal-batch.md`、父节点记录的
F425B/C/D与F426结果、phase父文档、两份权威架构及适用的 Skiff / Internals `AGENTS.md`。

Internals读取范围：

- `aihub/service/**`
- `aihub/client/**`
- AIHub README、manifest、API、receipt与tests
- `packages/llm-api/**`
- 为区分 provider/service-call identity 而读取的
  `packages/llm-providers/**`、`aihub/service/internal/managed_provider_transport.skiff`
  与 codex-relay直接依赖片段

## 2. 字段与 owner 矩阵

### 2.1 应删除的 external HTTP correlation

| 字段 / carrier | 当前 producer | 当前 consumer | 实际用途 | Verdict |
| --- | --- | --- | --- | --- |
| request body `request_id` | browser `app.js`生成随机 `requestId`，`chat-stream.mjs`写入 POST JSON；两份 service fixture也手写 | `chatEventsPreflightUnsafe`在成功 preflight 后读取，缺失时填 `"http"` | 只生成下述 SSE `requestId`；不影响 method/body decode、provider/model选择或 managed request | 删除 producer、read与fixture/docs |
| client local `requestId` | `createRequestId()` | 仅传给 `streamChatEvents`并写日志/body | 不进入 reducer state；不参与 fetch/reader选择或 abort | 连同 helper 参数和 generator 删除 |
| preflight `ChatEventsPreflight.ready.requestId` | body `request_id` read | `handleAihubEventsHttp -> streamChatEventResponse` | 只把同一字符串交给 envelope helper | 删除 |
| SSE envelope `requestId` | service `streamEnvelope` | client `parseEnvelope`只验证为 string；`applyStreamEnvelope`不读取 | 旧 WebSocket req/res echo；HTTP response自身已精确关联 | 删除 |
| service helper `requestId` 参数 | preflight / test helper | `streamEventSse`、chunk/error wrapper链 | 只搬运到每个 envelope | 机械删除整条参数链 |
| SSE envelope `runId` | `runIdFromRequestId` | client `parseEnvelope`只验证为 string；app完全不读取 | 固定派生 echo，没有独立 owner | 与 `requestId`一起删除 |
| service helper `runId` 参数 | `streamChatEventResponse`局部派生 | chunk/error wrapper链 | 只搬运到每个 envelope | 机械删除整条参数链 |

当前全部命中精确收敛在七个文件：

```text
aihub/client/app.js
aihub/client/chat-stream.mjs
aihub/client/chat-stream.test.mjs
aihub/service/README.md
aihub/service/internal/aihub_service.skiff
aihub/service/internal/aihub_service.test.skiff
aihub/service/internal/gemini.live.test.skiff
```

没有 `correlationId` / `correlation_id` 命中。HTTP body没有 `runId` read：caller即使提供同名字段，
当前 service也不会读取；HTTP `runId`始终是 `run-${requestId}`。

### 2.2 必须保留、不能当作 correlation alias 的字段

| 字段 / event | Owner与真实语义 | Consumer证据 | Verdict |
| --- | --- | --- | --- |
| `event.tag:"request-start"` | AIHub stream control event，表示 preflight已接受并公布 resolved provider/model；不携带ID | browser更新 provider/model并显示 “Request accepted” | 保留 |
| envelope `seq` | 单条 HTTP response stream内的 item顺序；service从0单调递增 | tests锁定逐项顺序，browser保存/显示最新 seq | 保留 |
| nested `LlmStreamEvent.id` | provider response、text/reasoning segment或tool-call lifecycle identity | `llm-api/decode.skiff`用 provider `response.id`、tool `id/call_id/item_id`闭合 start/delta/end；browser以 tool event `id`聚合 arguments | 保留 |
| `toolCallId` / `tool_call_id` | assistant tool call与后续 tool result的业务关联 | AIHub message parser与 `llm-providers` request encoder均要求并转发 | 保留 |
| provider `responseId` / `response.id` | provider response lifecycle identity | `llm-api`以同一 id投影 `start`和`finish` | 保留 |
| provider `call_id` / `item_id` | provider tool item/delta lifecycle identity | `llm-api`用它们选择并更新同一 tool state | 保留 |
| `finish.meta.providerCode`、`retryable` | 有限 provider failure diagnostic/重试语义 | managed LLM consumer可读；不是 request correlation | 保留 |
| codex-relay `x-client-request-id` | relay/provider HTTP header owner | AIHub的 `codexRelayRequestHeaders`并不生成它；它也不进入 AIHub event envelope | 本 batch 不修改 |

`request-start`中的单词 “request” 不是一个 correlation field；`seq`只在当前 response stream内排序，
也不把多个 HTTP requests multiplex到一条 response。

## 3. 旧 WebSocket 来源与 `runId` 判定

历史证据来自 exact candidate 的直接祖先，不依赖猜测：

- `ec91fa3^`（`eddeeb861...`）的 browser每次 Send创建一个 WebSocket，在唯一
  `chat.request` frame中发送 `requestId`和 `runId = run-${requestId}`，随后靠同一 socket读取 response。
- `a606700^`（`ec91fa3...`）的 service `handleSocketMessage`从旧 frame读取可选
  `requestId` / `runId`，并在每个 `aihub.llm.event`中原样 echo；缺失 `runId`时同样由
  `runIdFromRequestId`派生。
- HTTP迁移保留了同一 `streamEnvelope`，把 browser outer `requestId`改写成 body
  `request_id`，再由 service恢复成 `requestId`和固定 `runId`。这就是当前字段的完整来源。

当前 candidate 的完整 dataflow进一步排除独立业务语义：

1. `managedChatRequestFromProvider`显式构造 `llmApi.LlmRequest`，字段列表不含
   `request_id`、`requestId`或 `runId`。
2. provider body encoder只消费该 `LlmRequest`；correlation字段不被转发到
   Gemini、llm-providers或 codex-relay。
3. `runId`只出现在 envelope/helper参数，没有 DB、state、actor、cancel、stop、retry、
   lookup或resource read/write。
4. browser `streamState`只有 seq、provider、model、text、reasoning、tool calls、assets、
   raw events和terminal；没有 request/run key或pending map。
5. browser取消只调用当前 lexical `AbortController.abort()`；fetch signal与当前
   `Response.body` reader天然绑定，不查询任何ID。
6. repo-wide `aihub.llm.event` consumer搜索只得到 AIHub service producer、tracked client、
   tests和README，没有第二个 downstream consumer。

因此 `runId`不是 architecture 允许保留的“业务run identity”，只是旧 transport字段的固定别名。
本结论无需用户再决定。

## 4. 删除后的 canonical external HTTP contract

### 4.1 Request

`POST /v1/chat/events`与`POST /chat/events`继续接收现有 OpenAI-compatible business body，
例如：

```json
{
  "provider": "bailian",
  "model": "qwen3.7-max",
  "messages": [{"role": "user", "content": "hello"}],
  "stream": true
}
```

body不再声明或由 tracked client发送 `request_id`、`requestId`、`runId`或新 correlation字段。
本 repair不发明 `idempotencyKey`、`jobId`或其它ID；该 read-only chat operation没有相应业务需求。
Raw JSON parser对其它未知 OpenAI字段的既有 disposition不在本 leaf重定义，但 production不得再读取、
echo或文档化旧字段。

### 4.2 Success stream

每个 SSE JSON record的 canonical envelope为：

```json
{
  "type": "aihub.llm.event",
  "seq": 2,
  "event": {"tag": "text-delta", "id": "text-1", "text": "hello"}
}
```

顺序保持：

```text
seq 0 request-start
  -> each llmApi.LlmStreamEvent in arrival order
  -> finish envelope
  -> data: [DONE]
  -> HTTP stream end
```

不得用 header、query、cookie、SSE `id:`或另一字段补回 correlation。

### 4.3 Error与terminal

- pre-start method/body/provider/validation failure继续是有限的非2xx
  `application/json`：`{"error":{"code":"...","message":"..."}}`。当前本来就没有
  request/run字段，不新增。
- post-start decode/protocol/provider failure继续是下一个 seq 的
  `{"type":"aihub.llm.event","seq":N,"event":{"tag":"error",...}}`，随后 stream end；
  已发送 item保留，不发送 `[DONE]`。
- normal `finish`后仍必须有 `[DONE]`；`[DONE]`本身保持纯 sentinel。

### 4.4 Browser reducer与abort

- `parseEnvelope`只要求 canonical `type + seq + event`及既有 event tag约束，不再要求
  `requestId` / `runId`。
- `streamChatEvents`不再接收 `requestId`参数，也不再把 `request_id`混入 body。
- `applyStreamEnvelope`无需业务改写：它当前已经只消费 `event`和 `seq`。
- 每次 Send继续创建一个 `AbortController`；Cancel继续 abort当前 fetch，reader failure保留
  `AbortError`，service端依赖同一 supervised HTTP stream cancellation chain。

## 5. service-call / provider protocol隔离

External HTTP与内部service call是两个独立surface：

```text
external:
POST /v1/chat/events
  -> raw HttpRequest
  -> Stream<HttpResponseStreamEvent>
  -> SSE {type, seq, event}

service-call:
managedLlm.streamChat(
  llmApi.LlmRequest
) -> Stream<llmApi.LlmStreamEvent>
```

`llmApi.LlmRequest`没有 request/run correlation字段。`LlmStreamEvent.id`及其 provider来源ID属于
LLM item lifecycle；删除它们会破坏 service contract、tool delta聚合和 provider decode，绝不属于
本 repair。

以下路径必须保持 byte-for-byte不改：

```text
packages/llm-api/**
packages/llm-providers/**
aihub/service/internal/managed_provider_transport.skiff
codex-relay/**
```

同样不修改 AIHub `managedLlm.streamChat` / `validateChat` / `webSearch`及
`providerCatalog` operation selection。

## 6. Identity与receipt矩阵

| Record / identity | 预期 | 原因 / gate |
| --- | --- | --- |
| `ServiceProtocolIdentity` | **必须不变** | 五个 selected service-call operations及 `packages/llm-api` schema closure不变 |
| 五个 `ContractOperationId` | **必须不变** | operation signature/value plan不变 |
| `PackageSchemaIndexIdentity` | **必须不变** | public schema/API path不变 |
| `PackageLocalAbiIdentity` | **必须不变** | public callable surface/signature不变；只改 private HTTP implementation body/helper |
| `PackageBuildId` / immutable `PackageArtifact` ref | **必须变化** | `aihub_service.skiff` production FileIR改变 |
| `v1ChatEventsPost` `GatewayEntryIdentity` | **必须不变** | 当前 canonical preimage只见 `rawHttp + serverStream + http.request + HttpResponseStreamEvent + fixed error projection`，不包含 raw body或SSE内层 JSON字段 |
| `chatEventsPost` `GatewayEntryIdentity` | **必须不变** | 与上一条相同 |
| 其余五条 HTTP `GatewayEntryIdentity` | **必须不变** | 未修改 |
| `GatewayEntryKey` / `IngressSelector` | **必须不变** | path、method、host、key、handler signature不变 |
| `ServiceDeployment` revision / identity | **必须变化** | implementation PackageArtifact ref变化 |
| `RuntimeAssembly` identity | **必须变化** | 选择新的 AIHub deployment/package闭包 |
| service API receipt的 protocol / operation部分 | **必须不变** | `service-api-receipt.mjs`无需修改；generated receipt需与旧值逐项比较 |
| generated package/deployment/assembly receipt | **必须更新** | 记录新 build、deployment与assembly identity |

这里特别区分“external业务JSON发生变化”和当前 `GatewayEntryIdentity`的实际 preimage。
Skiff `artifact-identity/src/gateway.rs`明确排除 callable/package事实；raw HTTP又禁止
`requestBodySchema`和typed `responseSchema`。Compiler只把精确
`HttpResponseStreamEvent` carrier schema写成 `streamItemSchema`。因此本 repair若使两条
GatewayEntryIdentity变化，说明误改了authoring、handler signature、adapter kind/source或error projection；
不能为了让 identity变化而把业务 envelope伪装成 typed schema。

## 7. Repair DAG与精确写入范围

建议 service/client分成两个并行 leaf；它们没有共享 production file，本文就是冻结的wire checkpoint：

```text
F427B audit result
  ├─► AIHub service correlation removal
  └─► AIHub client correlation removal
          │
          └──────┬──────┘
                 ▼
       generated identity comparison
       + isolated fake/test-double combined
```

### 7.1 Service repair leaf

唯一允许写：

```text
aihub/service/internal/aihub_service.skiff
aihub/service/internal/aihub_service.test.skiff
aihub/service/internal/gemini.live.test.skiff
aihub/service/README.md
```

职责：

- 去掉 preflight/envelope/chunk/error链的 request/run字段和参数；
- 保留 seq、request-start、nested LLM event IDs、terminal与cancel chain；
- non-live tests精确断言 success与post-start error envelope不含 request/run字段；
- live fixture只做source更新，仍保持 `defaultRun false`，本 repair不得运行。

不得修改 `service.yml`、`api.yml`、`package.yml`、config、receipt owner、provider code或其它service。

### 7.2 Client repair leaf

唯一允许写：

```text
aihub/client/app.js
aihub/client/chat-stream.mjs
aihub/client/chat-stream.test.mjs
```

职责：

- 删除 request ID生成、日志注入、transport参数与body注入；
- parser消费 `{type,seq,event}`；
- reducer、terminal、有限buffer、reader cancel与 AbortError语义保持不变；
- tests的 request body exact assertion不含 correlation字段，所有 envelope fixture使用新shape。

`aihub/README.md`、static server、HTML、CSS不含相关合同，不应顺手修改。

### 7.3 Combined owner

两leaf合流后单一owner：

1. 生成 canonical package/deployment/assembly receipt；
2. 比较第6节全部 identity；
3. 使用现有 test double/fake provider完成一次 isolated HTTP stream combined；
4. 不使用 stable、live或真实provider。

## 8. 后继验证与反向搜索 gate

建议命令均针对后继 exact clean candidate；`<SKIFF_ROOT>`必须是主Agent冻结的 Skiff integration
checkout，不得指向 stable：

```bash
node --test aihub/client/*.test.mjs
node --check aihub/client/app.js
node --check aihub/client/chat-stream.mjs
node --check aihub/client/chat-stream.test.mjs

SKIFF_ROOT=<SKIFF_ROOT> npm --prefix aihub/service run test:service-api
SKIFF_ROOT=<SKIFF_ROOT> npm --prefix aihub/service run test:package-store
SKIFF_ROOT=<SKIFF_ROOT> npm --prefix aihub/service run test:workflow-guards
SKIFF_ROOT=<SKIFF_ROOT> npm --prefix aihub/service run type-check
SKIFF_ROOT=<SKIFF_ROOT> npm --prefix aihub/service test

git diff --check
```

Correlation production/docs反搜必须为0：

```bash
rg -n -i \
  '(request_id|requestId|runId|runIdFromRequestId|correlationId|correlation_id)' \
  aihub/client/app.js \
  aihub/client/chat-stream.mjs \
  aihub/service/internal/aihub_service.skiff \
  aihub/service/README.md
```

整个 AIHub source/test/docs范围也应为0；若保留显式negative fixture，必须单独说明且 production
搜索仍为0：

```bash
rg -n -i \
  '(request_id|requestId|runId|runIdFromRequestId|correlationId|correlation_id)' \
  aihub
```

禁止误删 provider/service-call字段：

```bash
git diff --exit-code ed5d333b2406d5375fca8acc96f4695667c48ced -- \
  packages/llm-api \
  packages/llm-providers \
  aihub/service/internal/managed_provider_transport.skiff \
  codex-relay

git diff --exit-code ed5d333b2406d5375fca8acc96f4695667c48ced -- \
  aihub/service/package.yml \
  aihub/service/api.yml \
  aihub/service/service.yml \
  aihub/service/service-api-receipt.mjs
```

Tests至少覆盖：

- request body没有 request/run字段；
- request-start seq0、全部 provider item顺序、finish + `[DONE]`；
- post-start error保留前项、使用next seq、无 `[DONE]`；
- pre-start JSON error无 correlation字段；
- malformed envelope、terminal前EOF、terminal后event、invalid UTF-8与buffer limit仍fail closed；
- AbortController/reader cancel不依赖ID；
- nested response/tool IDs仍可完成 tool start/delta/end聚合；
- generated service protocol与两条 gateway identity不变，build/deployment/assembly变化。

## 9. 本次审计命令与clean状态

实际执行的只读证据：

```bash
git rev-parse HEAD^{commit} HEAD^{tree}
git status --porcelain=v1
git diff --check

rg -n -i \
  '(request_id|requestId|runId|runIdFromRequestId|correlationId)' \
  aihub

rg -n 'aihub\.llm\.event' . \
  --glob '!node_modules/**' --glob '!build/**' --glob '!dist/**'

rg -n -i \
  '(request_id|requestId|runId|runIdFromRequestId|correlationId)' \
  packages/llm-api packages/llm-providers \
  aihub/service/internal/managed_provider_transport.skiff

git show ec91fa3^:aihub/client/app.js
git show a606700^:aihub/service/internal/aihub_service.skiff
```

结果：

- correlation命中只在第2.1节七个 AIHub文件；
- `aihub.llm.event`没有 AIHub之外的 consumer；
- `packages/llm-api` / `packages/llm-providers` / managed provider中没有同名
  request/run correlation字段，只有第2.2节真实 item/provider IDs；
- Internals exact candidate审计前后 `status --porcelain`为空，`git diff --check`通过；
- Skiff result checkout写本文前 clean；
- tests、artifact generation、stable/live/provider invocation计数均为0。

没有 merge、rebase、push；没有启动或访问 stable instance、watch、reload、固定端口、MongoDB、
live endpoint或真实provider。
