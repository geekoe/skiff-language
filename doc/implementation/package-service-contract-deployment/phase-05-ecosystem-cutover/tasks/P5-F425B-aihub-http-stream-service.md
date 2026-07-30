# P5-F425B AIHub HTTP event stream service cutover

状态：Ready。中高风险consumer迁移。

## 直接父节点

- `P5-F425-downlink-websocket-implementation-checkpoint.md`

需要行为对照时读取父节点引用的F424C result；不得从聊天摘要补协议。

## DAG位置与输入

与F425A/C/D并行。精确Internals与Skiff输入见父节点。它不依赖connect-only Skiff节点：本leaf删除AIHub
WebSocket并只使用已存在的raw HTTP server-stream能力。完成后与F425C进入AIHub combined。

## 写入范围

仅允许`aihub/service/**`以及Skiff任务repo中的本leaf result。禁止修改`aihub/client/**`、其它Internals
service、Skiff production或skiff-packages。

## 必须实现

1. `/v1/chat/events`与`/chat/events`绑定同一个精确返回
   `Stream<std.http.HttpResponseStreamEvent>`的handler；其它五条HTTP entry继续使用unary
   `handleAihubHttp`。
2. method/body/provider/managed-request preflight在stream start前完成；失败保持现有有限4xx/5xx JSON。
3. start后按到达顺序增量发送现有SSE `aihub.llm.event` envelope：
   request-start、所有LLM item、finish，随后`[DONE]`并结束。
4. start后的decode/protocol/unavailable失败发送下一个seq的现有in-band `error` envelope并结束；不能
   丢弃已经发送的item或改成第二个HTTP status。
5. client disconnect/cancel沿既有HTTP server-stream取消链终止provider work；不得重新实现transport。
6. 删除AIHub整个WebSocket surface：manifest block、`websocket`/`AihubSocketContext` public API、
   connect/receive/context/connection send helpers和相关claims/tests。
7. 保留HTTP envelope owner：`streamEnvelope`、`streamEventSse`、`requestStartEvent`、
   `llmEventJson`、`errorEvent`。
8. receipt仍只有五个service-call operation，`ServiceProtocolIdentity`语义不变；两条events gateway
   identity、Package build、deployment与assembly允许按真实内容变化。

## 非目标

- 不修改OpenAI-compatible completions两条lossy surface。
- 不修改`managedLlm.streamChat` service-call operation。
- 不修改AIHub browser。
- 不访问真实provider或live。

## 验证

新增/更新测试至少覆盖：跨chunk增量顺序、reasoning/tool/base64/finish、post-start error、pre-start
HTTP error、cancel cleanup、无WebSocket surface、receipt五operation不变。

运行实际匹配的AIHub service聚焦入口；linked worktree使用隔离workflow：

```bash
SKIFF_ROOT=<assigned-skiff-worktree> npm run test:service-api
SKIFF_ROOT=<assigned-skiff-worktree> npm run type-check
git diff --check
```

若canonical workflow被父节点Skiff尚未完成的范围外compile blocker遮挡，必须再运行可归属的source/unit
tests并如实记录，不能修改Skiff或伪报PASS。

## 交付

在Internals提交implementation；在Skiff任务worktree新增并提交
`P5-F425B-aihub-http-stream-service-result.md`。返回两个commit/tree、测试计数和clean状态。不得
merge/rebase/push/stable/live。

