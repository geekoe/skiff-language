# P5-F425B AIHub HTTP event stream service cutover result

状态：`IMPLEMENTED_WITH_CANONICAL_GENERATION_BLOCKED`。

AIHub service 的生产 source、manifest、API、receipt oracle、文档和测试已完成 HTTP
server-stream cutover；没有触发 `TASK_SCOPE_EXPANDED`。canonical package publish/type-check
仍被输入基线已有的 Skiff expression-type model 问题阻断，因此本结果不伪报 generated receipt、
gateway identity、PackageBuildId、deployment revision 或 assembly identity。

## 1. 精确输入与提交

| 输入 / 输出 | worktree / branch | commit | tree | 状态 |
| --- | --- | --- | --- | --- |
| Internals 输入 | `/Users/geek/workspace/internals-p5-f425b-aihub-service` / `codex/p5-f425b-aihub-service` | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` | clean |
| Internals implementation | 同上 | `96294f9bb2e2630af3d1980007b59b499808002a` | `3e932b2d5a9a7946a175a916b350b91e00463004` | implementation-only commit，clean |
| Skiff result 输入 | `/Users/geek/workspace/skiff-p5-f425b-aihub-service-result` / `codex/p5-f425b-aihub-service-result` | `e6ea73b67e9a1fb18e867fef3073ca33b7ecd5a5` | `54e51e6d86567473dfa269b975874d0352e63136` | clean |
| Skiff result 输出 | 同上 | 见本 leaf 的独立 result commit / 交付回执 | 见交付回执 | 只新增本文 |

Internals 写入严格限于以下八个 `aihub/service/**` 文件：

- `README.md`
- `api.yml`
- `internal/aihub_service.skiff`
- `internal/aihub_service.test.skiff`
- `internal/gemini.live.test.skiff`
- `service-api-receipt.mjs`
- `service-api-receipt.test.mjs`
- `service.yml`

没有修改 `aihub/client/**`、其它 Internals service、Skiff production 或
`skiff-packages`。

## 2. 实现结果

### 2.1 HTTP entry 与 stream 生命周期

`/v1/chat/events` 与 `/chat/events` 现在共同绑定：

```text
internal.aihub_service.handleAihubEventsHttp(
  std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent>
```

其余五条 entry（health、providers、models、两条 completions）继续绑定 unary
`handleAihubHttp`。completions 的 aggregated/lossy OpenAI compatibility 行为没有改变。

event handler 在任何 `200` stream start 前完成：

1. method 校验；
2. request body JSON 解码；
3. provider 选择；
4. managed request 构造与 `validateManagedChat` 校验。

这些阶段的失败仍是一个有限的 `4xx/5xx application/json` response stream：
恰好一个 start、可选的单个 body chunk 和一个 end，不会先写 `200`。

成功时 handler 写出：

1. `200` 和现有 SSE headers；
2. `seq:0` 的 `request-start` envelope；
3. provider 到达的每个 LLM item，各自独立 chunk、seq 单调递增；
4. 正常 `finish` 后的 `data: [DONE]`；
5. 单个 stream end。

reasoning、tool-call start/delta/end、base64、finish reason、usage 和 meta 都继续经
`llmEventJson` 投影。provider 在 `finish` 前结束或在 `finish` 后继续发 item 会成为既有
`provider_protocol_error` terminal。

### 2.2 start 后失败与取消

LLM decode、JSON decode、provider protocol 和 provider unavailable failure 在 stream
已经开始后，会以当前 state 的下一个 seq 写出既有 `aihub.llm.event/error` envelope，
随后 end；此前已写出的 item 不会丢失，也不会尝试第二个 HTTP status。error path 不写
`[DONE]`。

实现直接在 response stream ancestor 内迭代 `streamManagedChat(input.request)`，没有
`spawn`、detach、自建 transport 或 `CancelError` catch。HTTP consumer disconnect/break
因此沿 Skiff 已有 supervised server-stream cancellation chain 终止同一 provider ancestor。

### 2.3 WebSocket 删除与 envelope owner 保留

已删除：

- `service.yml` 的整个 `/ws` WebSocket block；
- `api.yml` 的 `websocket` 与 `AihubSocketContext` exports；
- connect/receive/context/connection send、WebSocket request conversion 和 socket stream
  helpers；
- README 的 `/ws`、`chat.request` 和 WebSocket preference claims；
- receipt/test 中把 WebSocket 当作生产 surface 的预期。

production source、manifest、API 和 README 对
`websocket|AihubSocketContext|handleAihubWebSocket|websocketToLlmRequest|sendStreamEvent`
的反向搜索为零。HTTP 所需的 `streamEnvelope`、`streamEventSse`、
`requestStartEvent`、`llmEventJson` 与 `errorEvent` 均保留。

### 2.4 service-call receipt 语义

receipt oracle 仍精确选择以下五个 executable service-call operation：

1. `managedLlm.streamChat`
2. `managedLlm.validateChat`
3. `managedLlm.webSearch`
4. `providerCatalog.builtinProvider`
5. `providerCatalog.model`

`handleAihubHttp`、新 `handleAihubEventsHttp`、`selectProvider`、interface declaration
以及已删除的 WebSocket surface 均明确禁止成为 service operation。
`serviceCalls: [managedLlm, providerCatalog]` 和 `managedLlm.streamChat` implementation
没有改变，所以 ServiceProtocolIdentity 的输入语义保持不变。

## 3. 验证结果

所有命令都在 assigned linked worktree 内运行，`SKIFF_ROOT` 固定为
`/Users/geek/workspace/skiff-p5-f425b-aihub-service-result`；没有访问 stable instance、
真实 provider 或 live test。

| 验证 | 结果 | 计数 / 说明 |
| --- | --- | --- |
| `SKIFF_ROOT=... npm run test:service-api` | PASS with expected skip | 8 discovered；7 pass、0 fail、1 skip。skip 仅为需要 canonical generated receipt 的 case |
| `npm run test:package-store` | PASS | 2/2 |
| `npm run test:workflow-guards` | PASS | 13/13 |
| `SKIFF_ROOT=... npm test` | BLOCKED after attributable Node tests | Node 共 23 discovered；22 pass、0 fail、1 expected skip；随后 canonical publish 命中下述同一 blocker |
| `SKIFF_ROOT=... npm run type-check` | BLOCKED, no regression | baseline 与 implementation 后均只有相同三个 expression-type model failure |
| Skiff syntax/test discovery probe | PASS as parser probe only | production parsed；49 个 `aihub_service` test、1 个 `managed_provider_transport` test parsed；另 1 个 `defaultRun false` live test parsed但未运行 |
| YAML parse | PASS | `service.yml`、`api.yml` |
| WebSocket production reverse search | PASS | 0 match |
| `git diff --check` | PASS | 0 error |

parser probe 只证明 source 可解析和测试声明可发现，不把 50 个 non-live Skiff test 伪报为
已执行。canonical runner 在进入这些 source tests 前即被 package publish 阻断。

### 3.1 不变的 canonical blocker

输入 baseline 的 `type-check` 已经失败于：

```text
internal.aihub_service:
  return object literal field `event` has no resolved expression type
internal.provider_catalog:
  return object literal field `reasoningLevels` has no resolved expression type
internal.provider_catalog:
  return object literal field `reasoning_levels` has no resolved expression type
```

implementation 后仍精确是这三个 failure；`internal.aihub_service` 的位置从 baseline
`2194:12` 变为 `2182:12`，仅因为前面的 WebSocket source 被删除，该
`streamEnvelope(... event: encodeJson(event))` expression 本身未改。另两个仍位于
`123:22` 与 `124:23`。没有新增 compile diagnostic。

按 leaf 禁令没有修改 assigned Skiff production 来绕过该父节点 blocker。因 publish 未完成，
本 leaf 没有生成、也没有声称验证：

- exact `ServiceProtocolIdentity`；
- 两条 events gateway entry identity；
- `PackageBuildId`；
- deployment revision / identity；
- assembly identity。

## 4. 自验收矩阵

| leaf 要求 | 结果 | 证据与限制 |
| --- | --- | --- |
| 1. 两条 events 共用精确 server-stream handler；其它五条 unary | PASS | manifest exact-source oracle 通过；handler signature 受 Node test 固定 |
| 2. method/body/provider/managed preflight 在 start 前，有限 JSON error | IMPLEMENTED | preflight 完整位于 `streamStart(200, ...)` 之前；新增 405/400/400/400/503 source tests 已 parse，canonical execution 被共同 blocker 遮挡 |
| 3. request-start、全部 item、finish、`[DONE]` 的增量顺序 | IMPLEMENTED | 每个 item 独立 emit；新增 10-chunk exact seq/order test 覆盖 text/reasoning/tool/base64/finish/usage，source test 已 parse |
| 4. start 后 error 用 next seq、保留已写 item、无第二 status | IMPLEMENTED | decode/protocol/unavailable 三类 fixture 均先发 text 再 error，并断言 1 start、1 end、无 DONE；source test 已 parse |
| 5. disconnect/cancel 终止 provider ancestor，不自建 transport | IMPLEMENTED | 同步 ancestor iteration；无 spawn/CancelError catch 的 source oracle 通过；slow provider consumer-break cleanup test 已 parse，runtime execution 被共同 blocker 遮挡 |
| 6. 删除全部 AIHub WebSocket surface/claims/tests | PASS | manifest/API/implementation/README reverse search为零；Node absence oracle 通过 |
| 7. 保留五个 HTTP envelope owner | PASS | 五个 owner 均存在且 events handler复用 |
| 8. receipt 仍五 operation，ServiceProtocolIdentity 语义不变 | SOURCE PASS / GENERATED ID BLOCKED | 5-operation positive/negative oracle 通过；exact generated identity 因共同 publish blocker 不可得 |
| 测试覆盖要求 | SATISFIED IN SOURCE | chunk顺序、reasoning/tool/base64/finish、post/pre-start error、cancel、无 WS、五 operation 均有对应 test/oracle；Node oracle已执行，Skiff source tests仅 parse |

## 5. 禁令与收尾

- 未运行真实 provider 或 `defaultRun false` live test。
- 未启动、重载或修改 stable instance、router、runtime、watch、MongoDB 或本机固定端口服务。
- 未执行 merge、rebase 或 push。
- 未修改范围外 repository production source。
- Internals implementation 以独立 commit 提交；Skiff 仅以独立 result commit 提交本文。
