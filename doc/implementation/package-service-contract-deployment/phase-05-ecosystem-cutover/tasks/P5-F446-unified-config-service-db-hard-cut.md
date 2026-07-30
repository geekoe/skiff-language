# P5-F446 Unified Config Snapshot And Service DB Hard Cut

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
R446 independent acceptance
```

F446A先建立共享DTO/identity checkpoint。F446B不能在tooling内复制临时snapshot DTO。F446C只消费A/B
checkpoint，不能让Runtime读取source YAML或latest目录。F446D可以把Skiff test-runner与外部仓库迁移拆成
不同repo worktree并行，但必须消费同一格式，不得增加兼容parser。

## Common Acceptance

1. `configLiterals`、SecretRef全系、Package/runtime state requirement、StateBinding全系及deployment
   state/resource/policy值在production和fixture中为零。
2. 三层YAML根直接按Package ID分区；overlay、unknown Package、typed required/optional与secret权限均有
   正负例。
3. PackageArtifact只拥有当前Package自己的typed config requirements；alias、diamond或dependency不能复制。
4. 配置变化只改变snapshot ref与activation generation，不改变PackageArtifact、ServiceDeployment、
   RuntimeAssembly或其identity。
5. generation prepare、commit、drain、cold recovery始终同时携带并精确验证assembly/snapshot refs。
6. 同build在同deployment一份ConfigView，跨deployment隔离；请求、continuation、stream、callback和spawn
   都不能切换snapshot owner。
7. service DB identity只由trusted platform/environment/serviceId派生；无DB metadata不建空DB，跨service
   访问失败；physical collection由stable Package/declared collection identity系统编码，无author mapping。
8. test case snapshot、ingress overlay和DB identity按generated deployment/test run隔离；foreign target不
   打开provider DB。
9. official/internals/stable authoring不再含state、secret ref、timeout、quota、principal或resource占位值。
10. 旧artifact/profile直接拒绝；没有dual read/write、ambient env或latest-config fallback。
