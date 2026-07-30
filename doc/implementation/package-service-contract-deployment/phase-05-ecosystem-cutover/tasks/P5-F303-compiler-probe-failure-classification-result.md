# P5-F303 Compiler probe failure classification结果

状态：Completed read-only。

审计代码状态：`c5ed98c754fc2d6b8f4f5cca45dc5032f68d18b4`，worktree clean。

## F302-B1

分类：mechanical fixture drift，可直接执行F304。

compiler树共有五个旧`BoundaryErrorContract`/`BoundaryOperationContract.errors`构造点：

- `compiler/tests/file_ir_execution_type_representation.rs`
- `compiler/tests/service_conformance.rs`
- `compiler/tests/shared_fixture_lane_probes.rs`
- `compiler/tests/websocket_ingress.rs`
- `compiler/driver/ecosystem_store/tests/fixtures.rs`

它们只需删除旧import/field，与production语义修复不重叠。

## F302-B2事实

四个std websocket类型不是误判；它们在`std/websocket.skiff`中都真实声明`<Context>`，并由
`std/api.yml`公开：

- `WebSocketConnection<Context>`
- `WebSocketReceiveEvent<Context>`
- `WebSocketIngressEvent<Context>`
- `WebSocketConnectResult<Context>`

source、File IR与projection handoff都精确保留non-empty `type_params`。F301按照F293 policy正确拒绝
ordinary public generic declaration，因此该失败是既定public-generic fail-close与现有std平台内建
WebSocket surface的公共语义冲突，不是删除一个错误predicate即可解决。

代码中已把这些引用作为compiler-known builtin而不是`AppliedNominal`：

- `WebSocketIngressEvent<T>`与`WebSocketConnectResult<T>`已有边界builtin encoding；
- ordinary builtin container内部的arguments允许递归投影；
- ordinary package generic declaration、public `AppliedNominal`与generic dependency schema仍必须
  fail closed。

production checked-in source里没有std之外的public generic declaration命中；同名普通package不能获得
std特例。

## 决策点

推荐保持ordinary public generic fail closed，并把上述四个exact std WebSocket声明认定为既有
compiler-known平台内建类型：

- 保留std源码声明供source type checking与package link使用；
- 不为声明本身生成`PackageSchema` index/record；
- service boundary只通过已有builtin规则投影被允许的WebSocket类型及其argument；
- `WebSocketConnection`等不在边界builtin白名单的类型仍不能跨服务；
- 特例按exact std owner + exact symbol识别，普通package同名类型仍拒绝。

其它选择是本轮正式设计public generic PackageSchema wire/identity，或把std WebSocket API改成非泛型；
两者都会改变更大的公共契约，不能作为F301小修。

## DAG

- F304：五个旧Boundary测试fixture机械迁移，Ready；
- std WebSocket production leaf：等待用户确认上述公共语义；
- 两者合流后重跑F302；此前不解除A2或F269。

