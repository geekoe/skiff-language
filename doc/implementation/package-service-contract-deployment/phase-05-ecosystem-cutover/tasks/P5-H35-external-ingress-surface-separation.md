# P5-H35 External ingress surface separation checkpoint

状态：Design checkpoint / implementation audit ready。

## 直接父节点

- Phase 5恢复与三仓库边界：`P5-H31-r05-batch-handoff.md`
- Package-owned service-call type模型：`P5-H32-package-owned-type-cutover.md`
- generic WebSocket冲突归因：`P5-F303-compiler-probe-failure-classification-result.md`
- service error wire/observability已冻结验收：`P5-F346-service-error-wire-observability-acceptance-result.md`

唯一权威设计仍是：
`../../../../../architecture/package-service-contract-deployment.md`。

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

设计提交：`50cb4a6196856601917e4a67570b97ab68ecfdb3`。

## 当前候选成熟度

当前仍是实现检查点，不是稳定候选。F346错误边界PASS继续有效；后续若修改其声明的service error
production表面，应按影响面复验。

External ingress分支在四个只读审计完成前不得开始production实现：

- F347：authoring/compiler/artifact/deployment；
- F348：runtime/transport/router/gateway；
- F349：public generic/PackageSchema与F302；
- F350：skiff-packages/internals真实service迁移范围。

审计只拥有当前代码事实和DAG建议，不得改变设计。四份result合流后由主Agent形成共享model/identity
checkpoint，再扇出consumer；不得让不同consumer分别发明gateway identity或ingress descriptor。

