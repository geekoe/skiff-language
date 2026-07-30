# P5-F282 Dependency result field-read regression result

状态：Shared blocker confirmed；解除只读compiler owner审计。

## 直接父节点与权威链

- 暴露路径：
  `P5-F269-internals-test-service-migration.md`
- 相关已完成类型修复：
  `P5-F273-public-alias-expansion-result.md`
- 唯一架构事实源：
  `doc/architecture/package-service-contract-deployment.md` §3、§4、§9
- 用户可见类型事实源：
  `doc/reference/static-semantics.md` §1、§15–§17

Package/service API共用同一名义类型、字段与typed-expression机制；service dependency的code-free API view
仍引用类型owner Package。若本文与权威文档冲突，以权威文档为准。

## Fresh阻断证据

F278合入后，F269在Skiff integration `fe05440d`与skiff-packages integration `609551f0`上创建全新
artifact store：

`/tmp/skiff-f269-f278diag.DBqsFv/ecosystem-store`

从零bootstrap/publish：

`std -> http-session -> track -> llm-api -> llm-providers -> agent -> codex-relay -> aihub`

结果：

- AIHub为8/8 Available，证明F278 same-heap修正与fresh链闭合；
- 随后发布`agine/service`时，source checker在
  `internal.agent_bridge_product_commands`报告四个字段读取没有resolved expression type：
  - `stopped`：107:14
  - `runId`：108:12
  - `deleted`：128:14
  - `stoppedRunId`：129:19
- 来源分别是：
  - `agent.thread.stopThread(...) -> agent.thread.StopThreadResult`
  - `agent.thread.markDeleted(...) -> agent.thread.MarkDeletedResult`
- fresh Agent artifact中public symbol、record descriptor与callable return signature均存在且精确；
  `StopThreadResult`含`stopped: bool`、`runId: string?`，
  `MarkDeletedResult`含`deleted: bool`、`stoppedRunId: string?`。
- Agine调用结果随后进入本地object literal字段，正常规则应能从dependency-owned public nominal record解析字段。
- 先前Phase 5链能够越过这些位置；当前失败不是缺失API公开、旧artifact或same-heap eligibility。

## 边界与判断

这是shared compiler type-resolution回归，不是Agine authoring问题：

- 禁止在Agine增加类型断言、JSON往返、wrapper或字段复制workaround；
- 禁止修改Agent public API来重复暴露同一字段；
- 禁止按package id、symbol名或四个field名特判；
- 不修改open error channel/F281 shared model或same-heap语义。

当前证据尚不足以确定首次损失位于dependency ingest、PackageSchema/PackageSymbol type view、call result
typing、member access还是object-literal target propagation，因此先做有界只读审计，不直接实现。

