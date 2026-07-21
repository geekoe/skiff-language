# P5-D05：Canonical WebSocket Authoring Bounded Audit

## 角色与输入

由未参与F04实现的只读Agent核查D04所列production WebSocket风险探针。输入为权威设计、F03B/F03C/F04/I02
合同，以及compiler、deployment、Router assembly gateway、runtime request/eval/native send的真实入口。不得
编辑、提交、创建fixture bridge或把artifact mutation当成authoring。

## 结论

当前正常source authoring不能形成可观测的canonical assembly WebSocket generation pin：现有assembly ingress
只绑定一个contract operation，gateway发送空`adapterArgs`，connect context元数据无法闭环，receive return被丢弃，
而std WebSocket参数/返回又被boundary projection与deployment eligibility拒绝。零参数operation、ambient mutable
state或test hook都不能证明旧连接在B激活后仍执行A。server-stream方案还缺compiler projection、assembly HTTP
stream dispatch、host lifecycle/backpressure/cancel，范围更大且不能服务T12 WebSocket，因此淘汰。

唯一冻结方案是F05的typed unified WebSocket ABI：同一operation用显式event union区分connect/receive；connect
返回typed accept/reject，receive调用production send capability后返回null。它只使用四对象已有type/operation/
selector字段，不新增artifact字段。审计完成只解锁F05任务合同，不表示production seam可用。
