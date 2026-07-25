# P5-F319 Service error channel delta audit

状态：Completed。结果见
`P5-F319-service-error-channel-delta-audit-result.md`。

## 直接父节点

- 全链路owner与目标矩阵：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- assembly type index：
  `P5-F298-service-error-type-index-result.md`
- request-local exception carrier：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`
- platform catch owner：
  `P5-F305-platform-catch-consumer-audit-result.md`
- linked representation identity：
  `P5-F316-representation-wrap-linked-consumer-result.md`

引用链已由F280向上连接唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。只在发现父节点与当前代码冲突时沿引用向上读取。

## DAG位置与目的

- 节点：W2-R canonical service error export/import前的delta audit。
- 当前候选是implementation checkpoint；F298/F299及linked identity已合入，F318正在独立实现
  representation eval，禁止审计者修改或重新设计该表面。
- 本审计把F280的全仓旧事实收敛为当前代码上一个可执行的runtime channel任务拆分；完成后解除
  canonical orchestrator、ordinary/stream/test-effect consumer任务。
- 这是只读审计，不是验收，不给W2-R PASS/FAIL。

证据基线：integration branch
`codex/package-service-phase-05`的提交`6b8d52ed92a7b4db16f2a38e91673f1d8dff35b8`。若production代码前进，
result必须记录实际审计HEAD；F318只会使representation eval结论失效，不应改变service error owner结论。

## 审计范围

只读检查：

- `runtime/boundary/**`
- `runtime/eval/**`中service call response、inline/test effect、ordinary/stream/ingress error路径；排除
  representation constructor实现评审
- `runtime/capability-context/**`中service response carrier
- `runtime/model/src/service_error.rs`
- `runtime/loader/**`、`runtime/linker/**`、`runtime/linked-program/**`中
  `ServiceErrorTypeIndex`的实际入口
- 与上述代码直接相连的artifact/package-schema value codec owner

不得修改production、测试或权威设计。唯一允许写入
`P5-F319-service-error-channel-delta-audit-result.md`并提交。

## 必须回答

1. 当前provider failure从eval/inline effect/stream到service response的每条真实入口、carrier和最终
   generic/legacy出口分别在哪里；caller failure从response到local exception/catch的每条真实入口在哪里。
2. `ServiceErrorTypeIndex`、`RuntimeValueCarrier`、`ServiceErrorEnvelope`和platform identity registry目前
   已接到哪些真实入口，哪些仍只是模型或fixture。
3. canonical orchestrator最小production owner应落在哪个现有crate/module；给出允许的依赖方向，证明不会形成
   eval↔boundary、boundary↔linker或model高层反向依赖。
4. public typed error编码需要的exact owner/schema/type plan/value materialization从哪些现有API取得；
   private/nonclosed/encode failure生成一次`InternalError`所缺的最小输入是什么。
5. inbound public typed error在linked/unlinked两种caller中的表示；opaque envelope如何穿过local
   `RequestException`并在未捕获时保持原bytes、`traceId/errorId`继续导出。
6. inbound `InternalError`如何materialize为普通exact nominal value；未捕获时如何识别“已有fixed envelope”并
   禁止重复包装，同时下一caller创建新本地stack。
7. ordinary、stream、cancel、ingress、service/host-boundary test effect是否能消费同一个入口；列出所有可能
   复制分类、按message/code猜测或绕过heap隔离的旧owner。
8. 将实现拆成最少的串行checkpoint与可并行consumer。每个任务必须给出精确production/test写入范围、
   blocked-by、被解除节点、最小正负探针和证据失效边界；不得把host/transport/router/telemetry的W2-W实现
   混进W2-R。

## 反向搜索与证据

至少搜索并归类：

```bash
rg -n 'ServiceErrorEnvelope|ServiceErrorTypeIndex|RequestException|RuntimeValueCarrier' runtime
rg -n 'UnhandledServiceError|RuntimeErrorPayload|response\\.error|detached_error' runtime/eval runtime/boundary runtime/capability-context
rg -n 'InternalError|ProviderUnavailableError|ProtocolError' runtime/eval runtime/boundary runtime/capability-context
```

result必须包含：

- production跳点表：入口、当前表示、owner、下游、遮挡关系；
- duplicate/legacy owner清单；
- 建议DAG及互不重叠的写入范围；
- 最早可执行的B1–B9、S1–S2、T2子集风险探针；
- 明确的设计缺口；若没有，写“无新增设计决策”。

不运行cargo、workspace、stable或live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f319-service-error-audit`
- branch：`codex/p5-f319-service-error-audit`
- 风险：只读准备；一次性新Agent，5分钟内开始写result；
- 提交并返回commit、审计HEAD、关键路径和建议DAG；
- 不push、不承接实现。
