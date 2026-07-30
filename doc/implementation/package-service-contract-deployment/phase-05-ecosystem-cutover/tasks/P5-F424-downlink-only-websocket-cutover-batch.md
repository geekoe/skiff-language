# P5-F424 Downlink-only WebSocket cutover batch

状态：Ready for bounded audit。当前是实现检查点，不是稳定候选。

## 直接父节点与权威设计

- `P5-F421B-suspension-relay-first-ecosystem-proof-result.md`
- `P5-F423-http-authoring-current-migration-batch.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- 最终架构事实源：`doc/architecture/package-service-contract-deployment.md`

2026-07-27用户已确认第一版采用HTTP上行、WebSocket下行。上述两份权威设计已在Skiff
`ba74febaca5dbe8f2b55d6db04e0544a6758bf4b`冻结该语义：

- 每个service最多一个WebSocket entry，只拥有upgrade/connect与服务端主动下发；
- 客户端text/binary data frame不进入runtime，gateway以close code `1003`关闭；
- ping/pong/close由协议栈处理；
- 不存在用户`receive`、消息selector/envelope、typed message handler、message operation或message identity；
- connect不产生connection context；只返回accept/reject、可选business identity与policy；
- `std.websocket`下行发送保持非挂起，并从当前service `ActivationContext`解析唯一entry；
- Agine、AIHub的所有业务上行迁移到HTTP。

实现任务不得重新设计WebSocket业务路由或恢复legacy raw receive。

## 精确输入

| Repo | Integration root | Commit | Tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` | `ba74febaca5dbe8f2b55d6db04e0544a6758bf4b` | `7ac91495f85bbf997fe4f57ddfbec76b82cc753c` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

三个integration worktree在本批次建立时均clean。任何production输入变化都会使审计结果需要重新核对。

## 当前事实与遮挡关系

- F423已把AIHub和Agine HTTP manifest迁移到current named `rawHttp` entries，但按当时合同保留了legacy
  `websocket.routes[].operation: websocket`。
- current compiler在WebSocket legacy authoring处fail closed，因此F421B fresh N5不能继续到AIHub之后。
- Agine现有`internal.agine_service.websocket`同时处理connect与receive；receive按`eventName`进入
  `agine_ws_dispatch`。浏览器与Host均有WebSocket上行。
- AIHub现有`internal.aihub_service.websocket`也同时处理connect与receive；receive接受`chat.request`。
- current HTTP迁移完成不等于这些上行行为已有HTTP等价入口；必须先逐项盘点，不能直接删除dispatcher。
- Skiff旧Assembly WebSocket business ingress已在F420B退役；connect-only generation、router/runtime
  admission与outbound需要按current artifact链重新建立，不能复活旧operation模型。

上游遮挡顺序：

```text
legacy service.yml websocket authoring
  -> current parser explicit rejection
  -> connect-only typed projection / deployment / assembly尚无可用记录
  -> router/runtime无法建立current service WebSocket
  -> consumer下行迁移与fresh N5不可证明
```

## DAG

第一波只读审计互不重叠：

```text
F424A  Skiff connect-only + outbound production owner audit
F424B  Agine WebSocket uplink -> HTTP migration audit
F424C  AIHub WebSocket uplink -> HTTP migration audit
```

三份result合流后，由主Agent形成新的implementation checkpoint和有界开发leaf。预期但不预先锁死的
production扇出是：

```text
shared Skiff authoring/artifact/compiler checkpoint
  -> router/runtime connect + outbound
  -> Agine service/client/host HTTP uplink migration
  -> AIHub service/client HTTP uplink migration
  -> cheap combined fresh publish/assembly probe
  -> fresh N5 ecosystem proof
```

如果审计证明共享owner、consumer边界或依赖不同，必须先修订本DAG，不要求原审计Agent继续实现。

## 本批次完成标准

审计批次完成时必须有三份result，能够给每个后继开发节点提供：

- 精确production/test owner与真实入口；
- legacy surface完整清单及反向搜索范围；
- 已存在可复用能力与真实缺口；
- 写入范围互斥关系、依赖和上游遮挡；
- 可执行聚焦验证与最早风险探针；
- 需要generation变化的record及fixture owner；
- 任何仍会改变公共契约的未决问题。

审计不修改production/test，不运行stable/live/instance，不运行完整N5，不merge/rebase/push。

