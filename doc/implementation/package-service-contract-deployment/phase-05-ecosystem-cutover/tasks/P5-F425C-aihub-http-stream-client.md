# P5-F425C AIHub Fetch/SSE browser cutover

状态：Ready。中风险consumer迁移。

## 直接父节点

- `P5-F425-downlink-websocket-implementation-checkpoint.md`

需要wire细节时读取父节点引用的F424C result。

## DAG位置与输入

与F425A/B/D并行。精确Internals输入见父节点。它按已冻结event envelope独立实现与单测；真实combined等待
F425B service完成。

## 写入范围

仅允许：

- `aihub/client/**`
- `aihub/README.md`
- Skiff任务repo中的本leaf result

禁止修改AIHub service、其它Internals模块、Skiff production或shared-client。

## 必须实现

1. Send改为`POST /v1/chat/events`；把生成的request id写入body `request_id`，继续使用现有service/version
   selector和业务body。
2. 使用`fetch`、`AbortController`、response body reader与可独立测试的增量SSE parser。
3. parser必须处理任意byte/chunk边界、CRLF/LF、多个records、`data: [DONE]`、UTF-8 split和有限缓冲；
   JSON envelope继续交给现有`applyStreamEnvelope`。
4. non-2xx在stream前读取有限JSON/text并走现有错误UI；finish/error是业务terminal，`[DONE]`和EOF用于
   transport完整性。terminal前EOF、malformed envelope和post-start network error必须fail closed。
5. Cancel调用`AbortController.abort()`并保持现有Cancelled UI；不得保留WebSocket close作为取消手段。
6. 删除AIHub生产WebSocket创建、chat.request send和message consumer；不要保留兼容fallback。
7. 若新增`.mjs` browser helper，static server必须返回正确JavaScript MIME并有测试。

## 非目标

- 不修改service或external wire。
- 不引入WebSocket downlink。
- 不运行真实provider、browser live或stable。

## 验证

至少增加pure parser与chat transport tests，覆盖成功、reasoning/tools/base64、server error、
AbortError、malformed/EOF和chunk split。运行AIHub client实际test入口、Node check/type-check（若有）及：

```bash
git diff --check
```

记录真实discovery/pass/fail/skip；零测试不是成功。

## 交付

在Internals提交implementation；在Skiff任务worktree新增并提交
`P5-F425C-aihub-http-stream-client-result.md`。返回两个commit/tree、测试计数和clean状态。不得
merge/rebase/push/stable/live。

