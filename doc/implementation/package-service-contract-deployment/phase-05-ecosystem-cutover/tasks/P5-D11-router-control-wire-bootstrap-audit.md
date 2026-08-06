# P5-D11：Router Control-Wire Bootstrap Audit

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 角色与结论

F04A修复environment并接入真实fixture后，只读审计isolated Runtime为何仍无法ready。不得编辑、提交、修复或给
F04 verdict。

结论为`DESIGN GO`：Runtime按shared wire先后发送binary `runtime.capabilities`与
`assembly.activation/register`，production `AssemblyRuntimeEndpoint`却不接受capabilities、只用text处理
activation；Router反向control也发送text，而Runtime严格拒绝text。即使逐个加case，health仍缺独立
`capabilityConnections`，并会继续遗漏既有actor/spawn/request消息。

该缺口属于F03B已经定义的“统一Runtime endpoint/session consumer”，但原DAG把整个F03B排在F04之后，形成新的
验收环。冻结F09/R10，从F03B提前拆出Router bootstrap seam；不得让F04A越界，不改Runtime或F03A shared codec，
也不提前实现F03C startup、admission、request trust boundary、generation pin、drain或lifecycle。

## 冻结修复

- production只保留一个Runtime endpoint/dispatcher/disconnect owner，server不得继续实例化缩减版第二endpoint。
- WebSocket control始终binary-only。`assembly.activation`在generic validator前按type分流，并使用F03A
  direction-aware codec；不得扩宽generic union、恢复text fallback或发明ACK/frame。
- 同一socket必须先完成capabilities；`runtimeId === replicaId`且连接期身份不变。capability connection与committed
  healthy replica是两个状态，health分别暴露，前者不得直接接流量。
- Router→Runtime只允许prepare/commit/abort；Runtime→Router只允许prepared/reject/register。保留既有
  runtime.register/health、request/response/cancel、connection.send与actor/spawn完整行为。
- R10通过后恢复F04A保留的isolated Host probe；真实ready、activation、HTTP ingress与checked-in consumer PASS
  才能给F04 verdict。
