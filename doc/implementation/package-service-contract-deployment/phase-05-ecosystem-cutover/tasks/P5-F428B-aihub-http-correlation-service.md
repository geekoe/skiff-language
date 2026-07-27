# P5-F428B AIHub HTTP correlation-free service stream

状态：Ready。中高风险 service wire repair。

## 直接父节点

- `P5-F427B-aihub-http-correlation-owner-audit-result.md`

父节点已冻结字段 owner、canonical SSE shape、identity 变化矩阵和精确 write set，并继续引用到
唯一权威设计。启动时只读本任务；实现需要时再沿父节点引用向上查阅。

## DAG 位置与精确输入

本节点与 F428C 并行，二者完成后解除 AIHub generated identity comparison 与 isolated combined
probe。Internals 输入为
`ed5d333b2406d5375fca8acc96f4695667c48ced`；Skiff production 证据锚定
`95efdf357a647d549bac047f5d301905df843dd3`。当前是实现检查点，不是稳定候选。

## 唯一写入范围

```text
aihub/service/internal/aihub_service.skiff
aihub/service/internal/aihub_service.test.skiff
aihub/service/internal/gemini.live.test.skiff
aihub/service/README.md
```

以及 Skiff task worktree中的本任务 result。禁止修改 `service.yml`、`api.yml`、`package.yml`、
config、receipt owner、provider code、client、其它 Internals service、Skiff production 或
skiff-packages。

## 必须实现

1. 从 HTTP preflight、stream envelope、chunk、error 与所有 helper 参数链删除
   `request_id`、`requestId`、`runId`、`runIdFromRequestId` 和 correlation aliases。
2. canonical SSE data envelope 精确为
   `{"type":"aihub.llm.event","seq":<n>,"event":<LLM event>}`。
3. 保留 `request-start` tag、stream-local `seq`、nested LLM event IDs、`toolCallId`、provider
   `response.id/call_id/item_id`、`providerCode` 与 `retryable`。
4. pre-start error 继续是有限 HTTP JSON；post-start error 使用 next seq 的新 envelope并结束；
   正常 finish 后才发送 `[DONE]`。
5. cancel/disconnect、item 顺序、terminal语义与 raw HTTP server stream不变。
6. non-live tests精确证明 success和post-start error envelope无 request/run字段；live fixture只
   更新 source，保持 `defaultRun false`，不得运行。

## 非目标与 identity

- 不修改 service-call/provider协议或 OpenAI-compatible unary surfaces。
- `ServiceProtocolIdentity`、五个 operation ID、package schema/local ABI、七个 gateway entry
  identity与 keys/selectors必须保持不变。
- Package build/deployment/runtime assembly因内容变化应改变；由后继 combined owner生成和比较。
- 不运行 stable、live或真实 provider。

## 验证

本 Agent 是以下聚焦验证的唯一 owner：

```bash
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:service-api
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:package-store
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:workflow-guards
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run type-check
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service test
git diff --check
```

按父节点第 8 节反搜 production/docs为零；显式 negative fixture如保留必须单列。最早风险探针是
source/unit test精确断言 request body read和每个 SSE envelope均无 request/run字段。任何本任务四个
production/test/doc文件变化都会使证据失效；F428C client-only改动不使本证据失效。

## Worktree、提交与交付

- Internals：`/Users/geek/workspace/internals-p5-f428b-aihub-service`
- 分支：`codex/p5-f428b-aihub-service`
- Skiff result：`/Users/geek/workspace/skiff-p5-f428b-aihub-service`
- 分支：`codex/p5-f428b-aihub-service`

启动后 5 分钟内完成第一次实际代码修改；否则按工作流返回 `TASK_NOT_EXECUTABLE`。提交
Internals implementation，再新增并提交
`P5-F428B-aihub-http-correlation-service-result.md`。返回两个 commit/tree、自验收矩阵和
clean 状态。不得 merge、rebase、push、stable 或 live；完成后不得自行承接 combined 节点。
