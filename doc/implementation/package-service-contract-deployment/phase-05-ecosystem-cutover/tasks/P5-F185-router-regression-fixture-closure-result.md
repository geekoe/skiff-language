# P5-F185：Router 回归夹具收敛结果

状态：Completed

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 结果

Router 全量回归已在当前四对象协议上通过：

- compiler 夹具现在是完整 package/service authoring root，包含 `package.yml`、`api.yml`、
  `service.yml` 与 `config.dev.yml`；
- 测试使用当前 `skiff-compiler package publish` 生成真实
  `PackageArtifact v3`、`ServiceContract v3`、`ServiceDeployment`，再用
  `skiff-compiler assembly build` 生成真实 `RuntimeAssembly`；
- Router 使用生产 `FilesystemRuntimeAssemblySnapshotLoader` 读取上述 immutable records，并校验
  `skiff-service-protocol-v3`、HTTP ingress、operation identity、record path 和 assembly identity；
- 未恢复旧 compiler positional input、`--out`、`--manifest-out`、ServiceUnit、ServiceAssembly、
  artifact index 或 dev reload pointer；
- 原来三项只验证退役 ServiceUnit/ServiceAssembly 路径的测试已删除，它们的真实跨层职责由当前
  compiler authoring → production RuntimeAssembly loader 测试统一接管；
- spawn queue 夹具使用测试启动时的统一时间锚点，并补齐当前 service build 与 activation identity，
  不再依赖已经过期的固定日历时间或缺失 identity 的旧请求；
- 未修改生产 lease、deadline、queue wait 或 timeout 语义。

## 验证

- `cd router && npm test`：49 files、555/555 PASS。
- `cd router && npm run type-check`：PASS。
- `git diff --check`：PASS。
- 聚焦四文件回归：4 files、95/95 PASS。
