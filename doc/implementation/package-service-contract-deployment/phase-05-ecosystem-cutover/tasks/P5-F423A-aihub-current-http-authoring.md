# P5-F423A AIHub current HTTP authoring

状态：Ready。

## 直接父节点

- `P5-F423-http-authoring-current-migration-batch.md`

## 精确起点

Internals：

```text
/Users/geek/workspace/internals-phase-05-integration
commit baf0c907ee26e48a5fb4c153825c233bde3a6234
tree   13f2f6e604fedbad80e0390e5408507430e28f8c
```

Skiff task/result base：

```text
/Users/geek/workspace/skiff-phase-05-integration
commit 5486f7ed4a7c23ea2b874871b12ff23155c1108a
tree   31895c173a781222ac7d9995a79918f20235b2d4
```

task checkout允许只增加F423 batch/leaf文档。启动时验证exact ancestry、production diff为0且两个
repo clean。

## 唯一写入

Internals：

```text
aihub/service/service.yml
aihub/service/service-api-receipt.test.mjs
aihub/service/service-api-receipt.mjs
```

Skiff：本任务result。

不得修改AIHub `.skiff` source、`api.yml`、`package.yml`、config、其它tests、Agine、Skiff
production/test。不得派子Agent、merge/rebase/push/stable/live。

## 精确迁移

七条HTTP entry及selector必须是：

| key | method | path |
| --- | --- | --- |
| `healthGet` | GET | `/health` |
| `v1ProvidersGet` | GET | `/v1/providers` |
| `v1ModelsGet` | GET | `/v1/models` |
| `v1ChatEventsPost` | POST | `/v1/chat/events` |
| `chatEventsPost` | POST | `/chat/events` |
| `v1ChatCompletionsPost` | POST | `/v1/chat/completions` |
| `chatCompletionsPost` | POST | `/chat/completions` |

每条均为：

```yaml
kind: rawHttp
handler: internal.aihub_service.handleAihubHttp
adapterArgs:
  - param: request
    source: { kind: http.request }
```

`serviceCalls`五项语义保持不变。`websocket` block与`timeout.default: 120000` byte-equivalent保留。

同步同一service的non-production receipt oracle：

- protocol positive prefix与synthetic fixture从v4到v5；
-五个service-call operation、三个package-only helper、ContractOperationId v1断言不变；
- source test精确断言七个named entry、完整adapter与`routes`/`operation`为0。

## 验证

```bash
node --test aihub/service/service-api-receipt.test.mjs
node --check aihub/service/service-api-receipt.mjs
rg -n "routes:|operation: handleAihubHttp|skiff-service-protocol-v4" \
  aihub/service/service.yml \
  aihub/service/service-api-receipt.test.mjs \
  aihub/service/service-api-receipt.mjs
git diff --check
```

预期Node 7项中6 pass、1个无generated receipt环境时expected skip；三类旧HTTP/protocol反搜0。
不得为了让publish通过而删除WebSocket。result记录HTTP迁移完成、WebSocket后继仍阻塞fresh publish；
Internals与Skiff分别单一commit，两个worktree clean。
