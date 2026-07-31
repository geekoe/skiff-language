# P5-F446 Unified Config Snapshot And Service DB Hard Cut

状态：**COMPLETE / R446 PASS**

## Authority

唯一语义来源：

- `doc/reference/config.md`
- `doc/reference/service-yml.md`
- `doc/reference/testing.md`
- `doc/architecture/package-service-contract-deployment.md` §11
- `doc/architecture/runtime-deployment-topology.md`
- `doc/architecture/db-capability-architecture.md`

本任务只拥有DAG和共同验收矩阵，不跨A–D生产owner实现代码。Skiff尚未发布，不保留旧profile、artifact、
SecretRef、state binding、reader、adapter或fallback。

## DAG

```text
F446A artifact/compiler hard cut
             |
             v
F446B config snapshot tooling
             |
             v
F446C activation/runtime/service DB
             |
             v
F446D test-runner + ecosystem migration
             |
             v
F448 activation owner switch atomic rebind
             |
             v
F449 service DB index admission + migration
             |
             v
R446 independent acceptance
```

F446A先建立共享DTO/identity checkpoint。F446B不能在tooling内复制临时snapshot DTO。F446C只消费A/B
checkpoint，不能让Runtime读取source YAML或latest目录。F446D可以把Skiff test-runner与外部仓库迁移拆成
不同repo worktree并行，但必须消费同一格式，不得增加兼容parser。F448负责在F446 activation事实之上把
service provider/callback的owner切换收敛为原子操作；它不能引入latest fallback或第二套snapshot reader。
F449负责让DB index metadata在prepare/cold recovery成为真实storage约束，并完成受控filtered migration；
它不能把partial index raw AST带回artifact/runtime，也不能自动drop受管index。

## Common Acceptance

1. `configLiterals`、SecretRef全系、Package/runtime state requirement、StateBinding全系及deployment
   state/resource/policy值在production和fixture中为零。
2. 三层YAML根直接按Package ID分区；overlay、unknown Package、typed required/optional与secret权限均有
   正负例。
3. PackageArtifact只拥有当前Package自己的typed config requirements；alias、diamond或dependency不能复制。
4. 配置变化只改变snapshot ref与activation generation，不改变PackageArtifact、ServiceDeployment、
   RuntimeAssembly或其identity。
5. generation prepare、commit、drain、cold recovery始终同时携带并精确验证assembly/snapshot refs。
6. snapshot顶层target environment来自受信producer输入；prepare和cold recovery在物化ConfigView前与
   activation environment精确比较。
7. 同build在同deployment一份ConfigView，跨deployment隔离；普通continuation与stream保持创建时owner。
   service provider/callback只能通过generation-pinned atomic rebinder切换到目标deployment/capability
   owner；spawn创建独立request，不在原request内偷换owner。
8. service DB identity由operator选择的受信Mongo endpoint/storage domain、environment与serviceId共同
   定界，不引入platformId；无DB metadata不建空DB，跨service访问失败。
9. physical collection由stable Package ID/declared logical collection identity系统编码；
   PackageDependency、PackageRequirement、PackageBinding和authoring均无collection-name mapping。
10. test case snapshot、ingress overlay和DB identity按generated deployment/test run隔离；foreign target不
   打开provider DB。
11. official/internals/stable authoring不再含state、secret ref、timeout、quota、principal、resource或
    collection mapping占位值。
12. 旧artifact/profile直接拒绝；没有dual read/write、ambient env或latest-config fallback。
13. `DeploymentPolicy`、`ResourcePolicy`和deployment/activation `policy` wire为零；external business
    request只使用Router operator配置的`requestTimeoutMs`，service profile、deployment和assembly不能覆盖。
14. owner rebind完整替换provider的config/DB/file/actor/spawn/WebSocket/telemetry事实，同时保留request
    deadline/内部停止/time/generation/lifecycle/trace/error/request identity/stream/test/heap limits；
    provider fresh heap、ActorRef显式owner、caller actor frame隔离与escaping old-generation stream均有
    证据。
15. 普通/unique index在prepared ACK前按同service完整plan协调；multi-version同定义合并、不同定义拒绝，
    missing additive create，managed changed/removed fail closed，unmanaged与`_id_`保留。
16. partial index在compiler明确拒绝且artifact/runtime无raw predicate AST；unique duplicate映射为脱敏、
    不可重试的`std.db.ConstraintError`，迁移只保留已确认的定义/设置/credential数据并保留旧库与备份。

当前实现状态与剩余闭合项见
[`P5-F446-closure-result.md`](P5-F446-closure-result.md)。该文档在R446独立验收前保持草稿状态。
