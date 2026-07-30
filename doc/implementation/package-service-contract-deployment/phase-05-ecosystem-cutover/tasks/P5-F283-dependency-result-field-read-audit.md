# P5-F283 Dependency result field-read audit

状态：Ready。

## 直接父节点与权威链

- 直接父结果：
  `P5-F282-dependency-result-field-read-regression-result.md`
- 父结果引用F269、F273以及唯一架构/静态语义事实源。

启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与证据状态

- production基线：Skiff integration `fe05440d`。
- 当前节点：只读shared compiler blocker审计；不是实现或验收。
- retained fresh store：
  `/tmp/skiff-f269-f278diag.DBqsFv/ecosystem-store`
- 并行节点：F281 open error shared model。不得修改或重新设计其任何DTO/文件。
- 完成后解除：一个有界compiler修复节点，随后F269从fresh链重跑Agine/test-service总验收。
- 若dependency type ingest、source type resolution或artifact schema发生改变，本审计证据失效。

## 审计问题

只读追踪以下真实链：

```text
Agent api.yml public result type
  -> Package schema/local ABI artifact
  -> Agine dependency ingest/code-free API view
  -> stopThread/markDeleted call result expression type
  -> local field access
  -> surrounding object literal field typing
```

必须回答：

1. 四个diagnostic的真实source位置、AST/expression形状与第一个没有resolved type的节点；
2. Agent fresh artifact中result type的exact owner、descriptor、public path、PackageSchemaTypeId与callable
   return type，以及Agine dependency view每一跳保留/丢失了什么；
3. `PackageSchema`、`PackageSymbol`、`PublicationType`、local type与transparent alias在member lookup/
   expression cache中分别走哪个production owner；
4. 为什么前序代码状态能通过、当前状态失败；用git history/diff定位最小引入范围，不能只猜F268/F273；
5. 上游失败是否遮挡同类nullable、nested field、method return、dependency-owned record与object-literal
   target typing缺口；
6. 最小production修复owner、非重叠写入范围、反向搜索范围与正负测试矩阵；
7. 是否存在公共设计缺口；若没有明确写无新增设计决策。

优先复用或增加compiler自己的最小A→B dependency fixture，不把全fresh生态发布当作开发循环。最终风险探针仍
必须由F269从真实Agine入口重跑。

## 交付

新增并提交：

`P5-F283-dependency-result-field-read-audit-result.md`

结果至少包含：

- 按执行顺序的owner表与首次损失；
- working/broken代码证据；
- 最小实现任务边界、禁止范围、测试命令和最早probe；
- 是否会与F281/W2 language consumer争抢文件；若会，给出明确依赖顺序而非并发修改；
- 没有源码workaround的说明。

## 非目标与工作边界

- 不修改production、fixture、Agine、Agent、reference/architecture或其它任务文件。
- 不运行完整compiler/workspace测试、生态publish、stable、live或chat smoke。
- 允许只读`rg`、git history/diff、artifact JSON检查，以及不会写源码的最小诊断命令。
- 只允许修改并提交本任务result文档。
- worktree：`/Users/geek/workspace/skiff-p5-f283-field-read-audit`
- branch：`codex/p5-f283-field-read-audit`
- 不push；一次性完成审计后不得自行实现。
