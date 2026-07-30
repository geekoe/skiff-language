# P5-F423B Agine current HTTP authoring and zero service-call API

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
agine/service/service.yml
agine/service/service-api-receipt.test.mjs
agine/service/service-api-receipt.mjs
agine/service/internal/agine_service_architecture.test.mjs
```

Skiff：本任务result。

不得修改Agine `.skiff` source、`api.yml`、`package.yml`、config、其它tests、AIHub、Skiff
production/test。不得派子Agent、merge/rebase/push/stable/live。

## 精确HTTP迁移

十四条HTTP entry及selector必须是：

| key | path |
| --- | --- |
| `sessionPost` | `/session` |
| `trackPost` | `/track` |
| `chatListPost` | `/chat/list` |
| `chatCreatePost` | `/chat/create` |
| `chatGetPost` | `/chat/get` |
| `chatLlmCallPost` | `/chat/llm-call` |
| `chatSendPost` | `/chat/send` |
| `hostsActivationTokenPost` | `/hosts/activation-token` |
| `providerCredentialSavePost` | `/provider/credential/save` |
| `providerCredentialDeletePost` | `/provider/credential/delete` |
| `providerChatgptPlanOauthStartPost` | `/provider/chatgpt-plan/oauth/start` |
| `providerChatgptPlanOauthSessionPost` | `/provider/chatgpt-plan/oauth/session` |
| `providerChatgptPlanOauthCancelPost` | `/provider/chatgpt-plan/oauth/cancel` |
| `providerChatgptPlanDisconnectPost` | `/provider/chatgpt-plan/disconnect` |

全部method为POST，且每条均为：

```yaml
kind: rawHttp
handler: internal.agine_service.handleAgineHttp
adapterArgs:
  - param: request
    source: { kind: http.request }
```

`websocket` block与`timeout.default: 120000` byte-equivalent保留。architecture/source test改为精确断言
named entries、每个path恰好一次、完整adapter，以及`routes`/`operation`为0。

## Service API oracle收敛

Agine `service.yml`没有`serviceCalls`；HTTP/WebSocket external ingress不属于service-call API。因此：

- package API projection继续精确包含`handleAgineHttp`与`websocket`，两者status为available；
- service protocol positive prefix与synthetic fixture为v5；
- 两个projection都不得携带`serviceOperationId`；
- validator拒绝任一helper被提升为service operation，也拒绝package API缺失/多出；
- 不因为未来WebSocket选择而把external ingress重新写入ServiceContract。

## 验证

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs
node --check agine/service/service-api-receipt.mjs
rg -n "routes:|operation: handleAgineHttp|skiff-service-protocol-v2" \
  agine/service/service.yml \
  agine/service/service-api-receipt.test.mjs \
  agine/service/service-api-receipt.mjs \
  agine/service/internal/agine_service_architecture.test.mjs
git diff --check
```

记录实际discovery/pass/fail/skip；三类旧HTTP/protocol反搜0。不得为了让publish通过而删除WebSocket。
Internals与Skiff分别单一commit，两个worktree clean；完成后不得自行承接后继。
