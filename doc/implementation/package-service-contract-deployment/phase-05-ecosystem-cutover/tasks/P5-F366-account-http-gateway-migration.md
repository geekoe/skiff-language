# P5-F366 Account HTTP gateway migration

状态：Ready（C4 Internals consumer；与F367写入不重叠）。

## 直接父节点与输入checkpoint

- 执行DAG：`P5-H36-external-ingress-implementation-dag.md`
- 精确生态清单：`P5-F350-external-ingress-ecosystem-migration-audit-result.md`
- 已完成共享接口：
  - `P5-F352-service-call-root-selection-result.md`
  - `P5-F357-http-gateway-compiler-projection-result.md`

父节点已沿引用链连接唯一权威设计。本leaf默认只读本文件；遇到事实缺口时再沿上述顺序向上读取，不从聊天摘要
补设计。

## Exact base与DAG边界

- Internals integration：`14ccfd417c9f45f00bd77015494cdd727e0f88dc`
- tree：`2327a766bcc6f32e7470b57420659fc991ef8a15`
- Skiff toolchain：从包含本task的
  `/Users/geek/workspace/skiff-phase-05-integration` checkpoint使用，并在返回证据中记录实际commit/tree。
- 完成本leaf解除Account `21 -> 0` service contract与21个HTTP gateway entry的生态合流；不依赖F365
  Host运行时接线。

## 必须完成

1. 只迁移`skiff-platform/account`：
   - `service.yml.http`改为21个named mapping entry；删除`routes`和全部`operation`字段；
   - 每条entry保持原method/path，使用`kind: rawHttp`；
   - `handler`直接指向当前package source `account.<function>`；
   - 每条entry恰好传入
     `request <- { kind: http.request }`；
   - stable key使用现有函数leaf名：`ping`、`register`、`login`等21个唯一identifier，不引入第二张entry表。
2. `api.yml`继续拥有既有Package API，但所有21个scalar function都不加`serviceCall`；`accountService`
   也不加marker。生成ServiceContract必须有零个operation，不能保留假operation维持旧identity。
3. 更新Account receipt/manifest tests，使其验证：
   - 21个selector唯一且method/path不变；
   - 21个entry均为raw unary、handler与adapter source精确；
   - 不存在`routes`、`operation`或service-call marker；
   - 生成receipt的service operation数为0，gateway entry与ingress各21，key/identity引用闭合。
4. 使用fresh isolated artifact root和本task Skiff toolchain至少真实发布一次Account service package；
   先bootstrap canonical std并发布
   `/Users/geek/workspace/skiff-packages-phase-05-integration/http-session`。该依赖必须来自
   skiff-packages integration checkpoint `609551f0a65bfcc814ed4c894e4c333b4ffb10f1`，其中
   `337e3fae`已经声明所需database state；不得改用尚未合流该前置的skiff-packages `main`，也不得读取
   stable artifact store。保存JSON receipt的operation/gateway/ingress数量和identity generation证据。
5. 检查21个真实handler仍是
   `HttpRequest -> HttpResponse`，不改业务HTTP语义、客户端URL、cookie/session、DB或config。

## 写入范围与非目标

允许：

- `skiff-platform/account/{service.yml,api.yml,service-api-receipt.mjs,service-api-receipt.test.mjs}`；
- 只有聚焦测试确实需要时才可新增同目录小型fixture/helper。

禁止：

- Account业务`.skiff`实现、client、package dependency、config/state语义；
- Relay、AIHub、Agine、共享`scripts/**`、F269 worktree；
- Skiff、skiff-packages、stable/live、push。

若真实发布要求修改共享compiler/tooling、业务handler或其它production owner，立即返回
`TASK_SCOPE_EXPANDED`，不得顺手修。

## 验证与交付

先枚举Node tests并确认非零，再运行：

```bash
node --test skiff-platform/account/service-api-receipt.test.mjs
git diff --check
```

真实发布使用`node "$SKIFF_ROOT/scripts/skiff.mjs" package publish ... --artifact-root <fresh> --json`；
bootstrap与依赖发布可复用现有只读workflow代码或等价命令，但不得修改共享workflow。禁止root/full/live
gate。

- worktree：`/Users/geek/workspace/internals-p5-f366-account-http-gateway`
- branch：`codex/p5-f366-account-http-gateway`
- production/tests一个commit，worktree保持clean；不merge/rebase/push。
- 返回exact base、commit/tree、changed files、非零测试数、真实receipt摘要与自验收矩阵。结果文档由主
  Agent写入Skiff integration。
- 启动后5分钟内必须开始实际修改；否则按工作流返回`TASK_NOT_EXECUTABLE`及精确缺口。
