# P5-H35 External ingress surface separation checkpoint

状态：Design checkpoint frozen；F347–F350只读审计已完成并合入，implementation DAG ready。

## 直接父节点

- Phase 5恢复与三仓库边界：`P5-H31-r05-batch-handoff.md`
- Package-owned service-call type模型：`P5-H32-package-owned-type-cutover.md`
- generic WebSocket冲突归因：`P5-F303-compiler-probe-failure-classification-result.md`
- service error wire/observability已冻结验收：`P5-F346-service-error-wire-observability-acceptance-result.md`

唯一权威设计仍是：
`../../../../architecture/package-service-contract-deployment.md`。

## 用户纠正与已更新设计

HTTP、WebSocket等external ingress不属于`api.yml`公开的service-to-service API：

- `api.yml`继续拥有Package公开调用面及其中可投影的service-call API；
- `service.yml`拥有external route、handler/pre/guard和adapter参数来源；
- ingress handler不要求public，不进入ServiceContract或service dependency module；
- ingress使用独立`GatewayEntryIdentity`和精确Package callable target，不能借用
  `ContractOperationId`；
- ingress变化只影响gateway identity/deployment revision，不影响`ServiceProtocolIdentity`；
- runtime external codec来自linked handler signature，不要求internal handler类型进入PackageSchema；
- public generic declaration可以保留package linkage，但不能因此自动获得service-call schema，也不能让
  无关Package整体失败。

设计初始提交：`50cb4a6196856601917e4a67570b97ab68ecfdb3`。
Publication清理、`api.yml`参考与external-ingress细化提交：
`e23b15dbbe49ce4ad4f8358310584b9ce067ddb0`。

## 当前候选成熟度

当前仍是实现检查点，不是稳定候选。F346错误边界PASS继续有效；后续若修改其声明的service error
production表面，应按影响面复验。

四个只读审计已经完成并合入：

- F347：authoring/compiler/artifact/deployment，result
  `P5-F347-external-ingress-compiler-artifact-audit-result.md`；
- F348：runtime/transport/router/gateway，result
  `P5-F348-external-ingress-runtime-router-audit-result.md`；
- F349：public generic/PackageSchema与F302，result
  `P5-F349-public-generic-boundary-availability-audit-result.md`；
- F350：skiff-packages/internals真实service迁移范围，result
  `P5-F350-external-ingress-ecosystem-migration-audit-result.md`。

审计只拥有当时代码事实和DAG建议，不得改变设计。四份result中有两项旧建议已被本checkpoint之后的
权威设计明确覆盖：

- F347把handler/build、完整adapter args与内部codec plan列入`GatewayEntryIdentity`的建议无效；
  identity只覆盖external protocol surface，selector、handler/pre/guard、build、内部名义identity与内部
  codec/execution plan都由deployment revision拥有。
- F349所审计的是旧的“所有public callable自动候选service operation”路径；目标态只有
  `api.yml`显式`serviceCall: true`的root进入service projection。未标记generic callable是合法
  Package API；显式标记但boundary unavailable时必须一次报告全部结构化原因。

后续唯一执行入口是`P5-H36-external-ingress-implementation-dag.md`。必须先形成共享model/identity
checkpoint，再扇出consumer；不得让不同consumer分别发明gateway identity或ingress descriptor。
