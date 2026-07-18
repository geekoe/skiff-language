# P2-R10C：HTTP Route Fixtures

状态：cancelled；2026-07-18 terminal-only 决策后不实现。

## 目标

原任务试图在 Phase 02 保留旧 deployment/route adapter 的 HTTP route 与 ingress 覆盖。
这需要 legacy compatibility path，与终态决策冲突。

## 处置

- Phase 02 删除/移出旧 `http_routes` service publication tests，不恢复 adapter。
- `ServiceContract + PackageArtifact + ServiceDeployment ingress -> runtime route` 在 Phase 03/04 的终态
  owner 上重新建测试。
- 纯 package source/File IR 断言由 R10 保留在 canonical fixture。
