# Phase 07：Tooling Cutover And Legacy Deletion

状态：outline-only；Phase 06 验收后再细化

## 输入

- PackageArtifact、ServiceContract、ServiceDeployment和RuntimeAssembly完整生产路径。
- 可运行的InProcessBoundary。

## 目标

- 切换 registry/release pointer、CLI、watch/dev sync、router/runtime reload、test-runner、fixtures与实际
  services。
- 物理删除 Publication aggregate/pipeline、code-owning ServiceUnit、旧 serviceAssembly tooling adapter、
  remote relay实现及fixtures、legacy adapters和旧 artifact readers/writers。
- 完成多replica、原子reload/drain、shared external storage及必要chat smoke。

## 验收边界

- production source tree不存在四对象之外的共同 publication aggregate或 dual path。
- 平台发布顺序支持 contract先发布、packages独立编译、deployment validation、assembly activation。
- 当前每个environment只有一个active full assembly；多个runtime replica扩CPU/RAM与可用性，但不承诺
  service级隔离或独立扩缩。
- 完整非live verify与受影响live/smoke通过；跨仓库改动分别提交，未经用户要求不push。

## 细化前复查

枚举所有legacy symbol/path/fixture和跨仓库consumer，按keep/rewrite/delete/add形成test disposition。
删除必须有反向搜索gate，不能只依赖测试数量。
