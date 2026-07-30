# P5-F367 Codex Relay HTTP gateway migration

状态：Ready（C4 Internals consumer；与F366写入不重叠）。

## 直接父节点与输入checkpoint

- 执行DAG：`P5-H36-external-ingress-implementation-dag.md`
- 精确生态清单：`P5-F350-external-ingress-ecosystem-migration-audit-result.md`
- 已完成共享接口：
  - `P5-F352-service-call-root-selection-result.md`
  - `P5-F357-http-gateway-compiler-projection-result.md`

父节点已沿引用链连接唯一权威设计。本leaf默认只读本文件；遇到事实缺口时再沿上述顺序向上读取。

## Exact base与DAG边界

- Internals integration：`14ccfd417c9f45f00bd77015494cdd727e0f88dc`
- tree：`2327a766bcc6f32e7470b57420659fc991ef8a15`
- Skiff toolchain：从包含本task的
  `/Users/geek/workspace/skiff-phase-05-integration` checkpoint使用，并在返回证据中记录实际commit/tree。
- 完成本leaf解除Relay `17 -> 2` contract、27个raw unary admin entry和3个raw server-stream `/v1`
  entry的生态合流；不依赖F365 Host运行时接线。

## 必须完成

1. `codex-relay/service/service.yml.http`改为30个named mapping entry，删除`routes`和全部
   `operation`字段：
   - 保持全部method/path逐值不变；
   - 27个admin/OPTIONS entry为`kind: rawHttp`，handler使用`admin_http.<function>`；
   - `/v1/responses`、`/v1/responses/compact`、`/v1/models`三个entry均为`kind: rawHttp`，
     handler使用`proxy_runtime.proxy`，由exact return投影为server stream；
   - 每条entry恰好传入`request <- { kind: http.request }`；
   - 每个selector有独立稳定key，即使共享handler也不合并entry。使用可读identifier，例如
     `adminSessionGet`、`adminChatgptOauthSessionDelete`、`adminSessionOptions`、
     `v1ResponsesPost`、`v1ModelsGet`。
2. `api.yml`从Package API移除15个external-only scalar exports；在`relayProxy` public instance上增加
   `serviceCall: true`。生成ServiceContract必须精确保留：
   - `relayProxy.responsesCompleted`
   - `relayProxy.responsesCompletedResult`

   不得让admin或`v1Proxy`进入ServiceContract，也不得删除这两个interface method或改其签名。
3. `timeout`的canonical owner是现有`config.dev.yml`。删除`service.yml`中重复的legacy timeout，
   不改profile值`120000`或其它policy。
4. 更新Relay receipt/manifest tests，使其验证：
   - 30个selector唯一且method/path不变；
   - 27个raw unary与3个raw server-stream entry的handler、adapter source、mode精确；
   - 不存在`routes`、`operation`或external-only API exports；
   - ServiceContract精确2个operation，gateway entry与ingress各30，key/identity引用闭合。
5. 使用fresh isolated artifact root和本task Skiff toolchain真实发布一次Relay service package；先
   bootstrap canonical std，再依序发布`packages/llm-api`、`packages/llm-providers`。不得读取stable
   artifact store。保存JSON receipt的operation/gateway/ingress数量、mode与identity generation证据。
6. 保持外部OpenAI-compatible wire：Relay继续接收原始`HttpRequest`并逐序发出
   `HttpResponseStreamEvent`。不得把它改成typed JSON、service stream或聚合响应；现有过滤、归档与重分块
   业务语义不在本任务修改。

## 写入范围与非目标

允许：

- `codex-relay/service/{service.yml,api.yml,service-api-receipt.test.mjs}`；
- 只有聚焦测试确实需要时才可新增同目录小型fixture/helper。

禁止：

- Relay业务`.skiff`实现、admin/client/scripts、package dependency与config/state值；
- Account、AIHub、Agine、共享`scripts/**`、F269 worktree；
- Skiff、skiff-packages、stable/live、真实OAuth/外部上游、push。

若真实发布要求修改共享compiler/tooling、业务proxy实现或其它production owner，立即返回
`TASK_SCOPE_EXPANDED`。

## 验证与交付

先枚举Node tests并确认非零，再运行：

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
git diff --check
```

真实发布使用`node "$SKIFF_ROOT/scripts/skiff.mjs" package publish ... --artifact-root <fresh> --json`；
bootstrap与依赖发布可复用现有只读workflow代码或等价命令，但不得修改共享workflow。禁止root/full/live
gate及任何真实OpenAI请求。

- worktree：`/Users/geek/workspace/internals-p5-f367-relay-http-gateway`
- branch：`codex/p5-f367-relay-http-gateway`
- production/tests一个commit，worktree保持clean；不merge/rebase/push。
- 返回exact base、commit/tree、changed files、非零测试数、真实receipt摘要与自验收矩阵。结果文档由主
  Agent写入Skiff integration。
- 启动后5分钟内必须开始实际修改；否则按工作流返回`TASK_NOT_EXECUTABLE`及精确缺口。
