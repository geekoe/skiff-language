# P5-F445H-I7-D0 Service-scoped ingress design

状态：`READY_FOR_DOCUMENTATION_CUTOVER`。

## 1. Parent chain and DAG position

直接父节点：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`；
- `P5-F445H-I7-S1-host-runtime-router-cross-layer-receipt-result.md`；
- `P5-F445H-I7-C-codex-relay-aihub-current-contract.md`；
- `P5-F445H-I7-C1-codex-relay-provider-checkpoint-result.md`。

I7R经其task与Phase 05 DAG追溯到唯一架构事实源
`doc/architecture/package-service-contract-deployment.md`。C在真实Relay/AIHub组合assembly中暴露：
两个不同service都合法声明`GET /v1/models`，现有裸全局ingress key却把它们判为collision。

用户已经冻结新的公共路由语义：HTTP请求的`Host`不参与Skiff Router路由；外部ingress负责把Host等
外部规则映射为`x-skiff-service`与`x-skiff-version`，Router再按精确service deployment与service内部
selector分两阶段选择。D0只把该决定写入权威文档并冻结后续实现代际，不修改production。

```text
C combined assembly collision
  -> D0 authority/documentation cutover
  -> K canonical model/schema/identity/wire checkpoint
  -> compiler / assembly / Router+Runtime consumers
  -> C combined revalidation
```

## 2. Frozen baseline and read-only preflight

| 项 | 值 |
| --- | --- |
| Skiff baseline commit | `cf43c08862d40e265fe660227aeff756b1dda406` |
| Skiff baseline tree | `d15431fd529ca24bfc12e32d42f84144551ae5a1` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-d0-ingress-docs` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-d0-ingress-docs` |
| integration owner | `/root/phase05_integration_steward` |

零worktree只读预检确认：

- `http.yml`公开reference已经只定义`method + path`，但架构/Phase/Router文档仍混有全局
  `(protocol, host, method, path)`；
- Router README仍把`host`写入service HTTP entry，并把裸`globalIngress`称为唯一selector；
- 当前production generations为ServiceDeploymentInput v4、ServiceDeployment/DeploymentArtifact v3、
  RuntimeAssembly v2、runtime frame v1；本次建议代际都是对应的单步硬切，没有冲突；
- P5-T03与P5-F03B的Host全局选择器完成态、测试与结果证据不再证明新的路由契约。

## 3. Documentation write ownership

本任务独占以下文档：

```text
doc/architecture/package-service-contract-deployment.md
doc/architecture/runtime-deployment-topology.md
doc/architecture/gateway-runtime-adapter-boundary.md
doc/reference/service-yml.md
doc/reference/runtime.md
router/README.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/phase-overview.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/phase-plan.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-T03-router-active-assembly-cutover.md
  P5-F03B-router-integration-repair.md
  P5-F03B-router-integration-repair-result.md
  P5-F445H-I7-D0-service-scoped-ingress-design.md
  P5-F445H-I7-D0-service-scoped-ingress-design-result.md
```

禁止修改production、schema、identity常量、fixture、test或其它repo。不得运行build/test/live/stable，
不得访问network、Mongo、OAuth或browser。

## 4. Required authority cutover

权威文档必须一致冻结：

1. `Host`只属于Router外部的ingress映射；它可以保留为HTTP业务metadata，但不选择service、
   deployment或handler。
2. 外部ingress按Host等平台规则注入可信`x-skiff-service`与`x-skiff-version`。Skiff不在本任务或
   Router内重做local ingress；直接向Router发送这两个header是Skiff production receipt。
3. Router严格解析两个header，先在active assembly中选择唯一精确`ServiceDeploymentRef`，再只用
   service内部`IngressSelector`选择handler。HTTP selector为`(protocol, method, path)`；
   WebSocket upgrade为`(protocol, path)`，JSON-RPC method继续在已pin连接内选择。
4. RuntimeAssembly key为`(ServiceDeploymentRef, IngressSelector)`。不同service可共享相同
   method/path；同一service内重复selector失败。
5. 同一active assembly不得同时包含同一`serviceId + contractVersion`的多个deployment revision；
   缺失、非法或歧义service/version、未知deployment与跨deployment替换全部fail closed。
6. Router到Runtime的request frame携带精确deployment；WebSocket upgrade同样pin精确deployment与
   generation，连接生命周期内不从Host或ambient state重新推导。
7. 这是hard cut；旧`host` route字段、旧裸全局ingress wire和旧header无选择语义的路径不得兼容读取。
8. 代际冻结为ServiceDeploymentInput v5、ServiceDeployment v4、DeploymentArtifact v4、
   RuntimeAssembly v3、runtime frame v2；GatewayEntryIdentity/GatewayEntry v2、
   ServiceContract/ServiceProtocol、Package artifact/build/local ABI/schema与WebSocketEntryId不变。

## 5. Evidence invalidation and completion

必须在Phase 05 overview/plan及P5-T03/F03B task/result中明确：

- 旧Host全局selector语义已撤回；
- 相关测试计数和历史实现提交仍是历史事实，但不能作为新service-scoped ingress契约的验收证据；
- 后续K与consumer wave必须重新证明Relay和AIHub同为`GET /v1/models`时，两个不同header坐标精确选择
  两个deployment；同service重复route、缺失/非法header、同坐标多revision与跨deployment frame
  substitution必须失败。

完成要求：

- 上述文档没有相互竞争的route owner或selector shape；
- fenced code blocks配对、`git diff --check`通过；
- 相关反向搜索只保留明确标为撤回/历史失效的旧语义；
- 结果文档记录exact commit/tree、实际写集、检查ledger和K的冻结输入。

D0是低风险文档节点，但它改变公共契约并成为高风险K的唯一输入。D0完成只解除K，不表示production已实现，
也不恢复C combined evidence。
