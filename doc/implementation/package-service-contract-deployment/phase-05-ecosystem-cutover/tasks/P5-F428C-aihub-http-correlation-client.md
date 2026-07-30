# P5-F428C AIHub HTTP correlation-free client stream

状态：Ready。中风险 client wire repair。

## 直接父节点

- `P5-F427B-aihub-http-correlation-owner-audit-result.md`

父节点已冻结字段 owner、canonical SSE shape、abort边界和精确 write set，并继续引用到唯一
权威设计。启动时只读本任务；实现需要时再沿父节点引用向上查阅。

## DAG 位置与精确输入

本节点与 F428B 并行，二者完成后解除 AIHub generated identity comparison 与 isolated combined
probe。Internals 输入为
`ed5d333b2406d5375fca8acc96f4695667c48ced`；Skiff production 证据锚定
`95efdf357a647d549bac047f5d301905df843dd3`。当前是实现检查点，不是稳定候选。

## 唯一写入范围

```text
aihub/client/app.js
aihub/client/chat-stream.mjs
aihub/client/chat-stream.test.mjs
```

以及 Skiff task worktree中的本任务 result。禁止修改 AIHub service、`aihub/README.md`、
static server、HTML/CSS、其它 Internals service/client、Skiff production 或 skiff-packages。

## 必须实现

1. 删除 browser request ID generator、日志注入、`streamChatEvents` transport参数与 HTTP body
   `request_id` 注入；不得换成 correlation header或同义字段。
2. parser只消费 `{type,seq,event}`；删除 envelope `requestId`、`runId` 的要求和读取。
3. reducer、terminal、有限buffer、reader cancel、AbortController与 AbortError语义保持不变。
4. request body exact assertion只含业务字段；所有 envelope fixture使用新shape。
5. 保留 nested provider/tool IDs以及它们驱动的 tool start/delta/end聚合。
6. malformed envelope、terminal前 EOF、terminal后 event、invalid UTF-8和buffer limit继续
   fail closed。

## 非目标

- 不修改 AIHub service、service-call/provider协议或生成 receipt。
- 不新增客户端 correlation map；单个 `fetch` Promise/reader就是 HTTP response owner。
- 不运行 stable、live、真实 provider或浏览器 E2E。

## 验证

本 Agent 是以下聚焦验证的唯一 owner：

```bash
node --test aihub/client/*.test.mjs
node --check aihub/client/app.js
node --check aihub/client/chat-stream.mjs
node --check aihub/client/chat-stream.test.mjs
git diff --check
```

按父节点第 8 节反搜三个文件中的 request/run/correlation production命中为零。最早风险探针是
request body exact assertion和完整 SSE sequence unit test。三个授权文件任一变化都会使本证据失效；
F428B service-only改动不使本证据失效。

## Worktree、提交与交付

- Internals：`/Users/geek/workspace/internals-p5-f428c-aihub-client`
- 分支：`codex/p5-f428c-aihub-client`
- Skiff result：`/Users/geek/workspace/skiff-p5-f428c-aihub-client`
- 分支：`codex/p5-f428c-aihub-client`

启动后 5 分钟内完成第一次实际代码修改；否则按工作流返回 `TASK_NOT_EXECUTABLE`。提交
Internals implementation，再新增并提交
`P5-F428C-aihub-http-correlation-client-result.md`。返回两个 commit/tree、自验收矩阵和
clean 状态。不得 merge、rebase、push、stable 或 live；完成后不得自行承接 combined 节点。
