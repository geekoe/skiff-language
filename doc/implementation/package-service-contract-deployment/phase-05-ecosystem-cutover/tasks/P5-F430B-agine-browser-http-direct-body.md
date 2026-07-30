# P5-F430B Agine browser HTTP direct-body cutover

状态：Ready。中高风险 browser consumer migration。

## 直接父节点

- `P5-F428A-agine-http-direct-body-service-result.md`
- `P5-F427A-agine-http-correlation-owner-audit-result.md`

F428A冻结36条HTTP route的direct request/response shape；F427A冻结browser、mock、E2E owner及旧WIP
可复用边界。二者继续引用到唯一权威设计。启动时只读本任务；实现需要时再沿引用向上查阅。

## DAG位置与准备好的输入

本节点依赖F428A，完成后解除Agine service+browser combined probe。Internals基线是：

| 角色 | commit | tree |
| --- | --- | --- |
| F428A已集成direct-body service | `658540a60f83e7609818a7531c5a2944c8c8fb47` | `7cf54d209adc8ecfa85e94b04ab49bde033fcaa6` |
| 在上述基线上重放的F426C scoped WIP | `6a1396ed41a90ab17d1a97154acd211f9a295a8d` | `d1537439c5bb697c0cc887ec0e740a6b7569fe82` |

WIP已把22个ordinary caller的大部分literal path/body改为HTTP，并带有对应unit tests；它不是可接受
候选。必须删除其中的HTTP ID注入、legacy envelope flatten，并补齐原HTTP caller、mock和E2E。
当前只是实现检查点，不是稳定候选。

## 写入范围

只允许：

- `agine/client/**`
- Skiff task worktree中的本leaf result

禁止修改`agine/protocol/**`、`agine/service/**`、`agine/host/**`、其它Internals domain、
Skiff production或skiff-packages。若direct response消费必须改变service/protocol shape，返回
`TASK_SCOPE_EXPANDED`。

## 必须实现

1. `agine/client/src/lib/http.ts`：
   - HTTP helper直接发送caller业务body，不生成或注入`requestId`；
   - 删除`flattenLegacyEnvelope`和`eventName/requestId/ok/outer payload`解析；
   - 2xx直接返回业务JSON；非2xx读取`error.code/message`并保持`HttpRequestError`语义；
   - local日志可保留path/operation label、status和脱敏业务body，但不得制造wire correlation字段。
2. 原`/chat/list|create|get|send`、`/chat/llm-call`、activation/provider caller都直接读取F428A业务
   response；`/chat/llm-call`自己的业务`payload`字段不能误当wrapper删除。
3. WIP覆盖的22个ordinary business caller全部使用literal HTTP path和业务payload，不再通过
   `socket.request`/`send`。保留真实业务ID：chat/message/run/toolCall/attempt/clientInstance/provider
   IDs。
4. HTTP调用的Promise/局部AbortController拥有response；不得新增pending correlation map、header、
   query、cookie、SSE id或同义字段。
5. browser WebSocket只承担server downlink。WIP中的application JSON heartbeat no-op可保留，但不能
   因本任务删除仍被legacy WS/Host helper使用的request ID matching实现；`socket.ts`、`ws.ts`、
   `types.ts`、`GlobalErrorHandler`和cookie WebSocket RPC中的WS语义按transport分区判断。
6. unit mock与frontend `mockApp`分离HTTP/WS观测：
   - HTTP记录literal path、business body与`transport:"http"`，不合成WS response envelope；
   - WS request/send继续记录`eventName/requestId`并保持pending matching。
7. E2E helpers拆开transport：
   - `api.chat-smoke.mjs`的HTTP `/chat/list|create|get|send` body删除request ID；WS helper继续生成；
   - `system.two-hosts.e2e.ts`、`machineHarness.ts`中的HTTP activation/chat body删除correlation；
     同文件的WS frames继续保留request ID；
   - frontend assertions按HTTP记录验证22个ordinary caller和`chat/llm-call`，不再期待WS envelope。
8. HTTP tests必须精确断言业务request body不含
   `requestId/request_id/correlationId/correlation_id`，success直接body，error固定shape；legacy
   WS tests继续证明request matching有效。
9. 删除WIP或旧tests中对HTTP `*-response`、`requestId` echo、`ok`、flattened duplicate fields的
   期待。不能通过容忍两种shape让测试通过。

## 关键入口与遮挡

```text
browser action/component
  -> literal HTTP path + business body
  -> F428A direct business response/error
  -> store/component update

server downlink
  -> browser WebSocket event listener
  -> existing reducer/notification owner
```

本任务不拥有Host握手、host-file或服务编译。F428A source动态tests仍被D4 test-runner seam遮挡；
本leaf验证consumer shape，不把该遮挡伪报为service execution PASS。

## 验证

本Agent是以下聚焦验证的唯一owner：

```bash
npm run type-check --workspace @agine/client
npm run test:logic --workspace @agine/client
npm run test:frontend --workspace @agine/client
node --test agine/client/e2e/support/cookie-websocket-rpc.test.mjs
git diff --check
```

按F427A第10节执行分区反搜：HTTP producer/body/parser不得命中四个禁用alias和
`flattenLegacyEnvelope`；WS helper/Host相关正向命中必须保留并有测试。最早风险探针是
`http.test.ts` exact body/direct success/error，加一个ordinary store action和一个frontend mock
HTTP observation。

client production、mock、E2E helper或F428A协议/服务变化都会使本证据失效；不相关的Skiff
Runtime/Router改动不使client聚焦证据失效。

## Worktree、提交与交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f430b-agine-browser`
- 分支：`codex/p5-f430b-agine-browser`
- Skiff result worktree：`/Users/geek/workspace/skiff-p5-f430b-agine-browser`
- 分支：`codex/p5-f430b-agine-browser`

这是新开发Agent会话；WIP只作为代码输入，不复用旧Agent。启动后5分钟内完成第一次实际代码修改。
提交一个completion commit置于WIP commit之后，再新增并提交
`P5-F430B-agine-browser-http-direct-body-result.md`。返回最终candidate commit/tree、result
commit/tree、自验收矩阵和clean状态。不得merge、rebase、push、stable/live；完成后不得自行承接
combined节点。
